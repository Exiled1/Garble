use crate::crypto::{self, SessionKey};
use crate::message::{self, Clientbound, MessageCodec, Serverbound};
use futures::SinkExt;
use openssl::{error::ErrorStack, pkey::Private, rsa::Rsa};
use std::string::FromUtf8Error;
use thiserror::Error;
use tokio::{
    net::TcpStream,
    sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender},
};
use tokio_stream::StreamExt;
use tokio_util::codec::{Decoder, Framed};
type MessageStream = Framed<TcpStream, MessageCodec<Serverbound, Clientbound>>;

#[derive(Error, Debug)]
pub enum ClientError {
    #[error(transparent)]
    IoError(#[from] std::io::Error),

    #[error(transparent)]
    SslError(#[from] ErrorStack),

    #[error("Invalid signature")]
    InvalidSignature,

    #[error("Invalid utf8 conversion")]
    InvalidConversion(#[from] FromUtf8Error),

    #[error("Server terminated connection: {reason}")]
    ServerShutdown { reason: String },

    #[error("invalid message recieved from server: {message:?}")]
    InvalidMessage { message: message::Clientbound },
}

#[derive(Debug)]
pub struct ChatConnector {
    tcp_stream: MessageStream,
    pub keypair: Rsa<Private>,
}

#[derive(Error, Debug)]
pub enum ConnectError {
    #[error(transparent)]
    ClientError(#[from] ClientError),

    #[error("connection failed: {reason}")]
    ConnectRefused {
        connector: ChatConnector,
        reason: String,
    },

    // this error message gets an exclamation point because it means someone tried to attack you
    #[error("key {key:?} does not match requested fingerprint {fingerprint}!")]
    FingerprintMismatch { fingerprint: String, key: Vec<u8> },
}

impl ChatConnector {
    pub async fn new(hostname: &str, port: usize) -> Result<Self, ClientError> {
        let keypair = crypto::generate_keypair(None)?;

        // TcpStream is not a stream, it's a stream of bytes (NOT A STREAM), I guess it's an AsyncRead/AsyncWrite.
        let not_a_stream = TcpStream::connect(format!("{hostname}:{port}")).await?;
        let mut message_stream = message::MessageCodec::default().framed(not_a_stream);

        message_stream
            .send(message::Serverbound::Hello {
                public_key: keypair.public_key_to_der()?,
            })
            .await?;

        Ok(Self {
            tcp_stream: message_stream,
            keypair,
        })
    }
    pub async fn request_connection(
        mut self,
        peer_fingerprint: String,
    ) -> Result<ChatClient, ConnectError> {
        // Choose a session key & encrypt it with the user's public key
        self.tcp_stream
            .send(message::Serverbound::ConnectRequest {
                peer_fingerprint: peer_fingerprint.clone(),
            })
            .await
            .map_err(ClientError::from)?;

        loop {
            match self.tcp_stream.next().await {
                Some(Ok(message::Clientbound::ConnectWaiting)) => continue,
                Some(Ok(message::Clientbound::ChooseKey { dst_public_key })) => {
                    if crypto::sha3_256_base64(&dst_public_key) != peer_fingerprint {
                        return Err(ConnectError::FingerprintMismatch {
                            fingerprint: peer_fingerprint,
                            key: dst_public_key,
                        });
                    }

                    let dst_public_key =
                        Rsa::public_key_from_der(&dst_public_key).map_err(ClientError::from)?;

                    // Generate a temporary AES key for the session.
                    let key = SessionKey::new(None).map_err(ClientError::from)?;

                    // Encrypt it with the peer's public key, and sign it with our private key.
                    let (encrypted, signature) = key
                        .encrypt_session_key(dst_public_key.as_ref(), self.keypair.as_ref())
                        .map_err(ClientError::from)?;

                    // Send the encyrpted key to the other client.
                    self.tcp_stream
                        .send(message::Serverbound::UseKey {
                            session_key: encrypted,
                            signature,
                        })
                        .await
                        .map_err(ClientError::from)?;
                    break Ok(ChatClient::new(self.tcp_stream, key));
                }
                Some(Ok(message::Clientbound::UseKey {
                    dst_public_key,
                    session_key,
                    signature,
                })) => {
                    if crypto::sha3_256_base64(&dst_public_key) != peer_fingerprint {
                        return Err(ConnectError::FingerprintMismatch {
                            fingerprint: peer_fingerprint,
                            key: dst_public_key,
                        });
                    }

                    let dst_public_key =
                        Rsa::public_key_from_der(&dst_public_key).map_err(ClientError::from)?;
                    let session_key = SessionKey::decrypt_session_key(
                        session_key,
                        signature,
                        self.keypair.as_ref(),
                        dst_public_key.as_ref(),
                    )?;

                    break Ok(ChatClient::new(self.tcp_stream, session_key));
                }
                Some(Ok(message::Clientbound::ConnectFail { reason })) => {
                    break Err(ConnectError::ConnectRefused {
                        connector: self,
                        reason,
                    })
                }

                Some(Ok(message::Clientbound::Shutdown { message })) => {
                    break Err(ConnectError::from(ClientError::ServerShutdown {
                        reason: message,
                    }))
                }
                None => {
                    break Err(ConnectError::from(ClientError::ServerShutdown {
                        reason: "socket closed by remote server".to_string(),
                    }))
                }

                Some(Ok(message)) => {
                    break Err(ConnectError::from(ClientError::InvalidMessage { message }))
                }

                Some(Err(e)) => break Err(ConnectError::from(ClientError::from(e))),
            }
        }
    }
}

// We're gonna need to define the interface from Client to Server & from Server to Client, but can't be same struct b/c of ownership.
pub struct ChatClient {
    // port: usize,
    // hostname: String,
    // keypair: Rsa<Private>,
    // tcp_stream: MessageStream,
    pub terminal_task: TerminalTask,
}

// This channel sends stuff from the client to the terminal.
struct ClientTask {
    pub inbound: UnboundedSender<Result<String, ClientError>>,
    pub outbound: UnboundedReceiver<String>,
    pub tcp_stream: MessageStream,
    session_key: SessionKey,
}
impl ClientTask {
    fn new(
        // From terminal
        inbound: UnboundedSender<Result<String, ClientError>>,
        // To server
        outbound: UnboundedReceiver<String>,
        tcp_stream: MessageStream,
        session_key: SessionKey,
    ) -> Self {
        ClientTask {
            inbound,
            outbound,
            tcp_stream,
            session_key,
        }
    }

    // Tokio doesn't have a 2-way channel, pain ;-;
    async fn run_background_task(mut self) {
        let mut closed = false;
        while !closed {
            // recieves OUTBOUND data FROM the terminal and sends SERVERBOUND data TO the server. (Gods this was confusing LOL)

            // Any data we recieve from the terminal, we send to the server
            // Any data we get from the server we send to the terminal.
            // self.inbound.send().await
            // self.outbound.recv().await // user messages

            let result = async {
                tokio::select! {
                    msg_from_server  = self.tcp_stream.next() => {
                        match msg_from_server {
                            None => closed = true,
                            Some(Err(e)) => return Err(ClientError::from(e)),
                            Some(Ok(message::Clientbound::Shutdown{message})) => {
                                closed = true;
                                return Err(ClientError::ServerShutdown { reason: message });
                            }
                            Some(Ok(message::Clientbound::Message(m))) => {
                                let decrypted = self.session_key.session_decrypt_message(m)?;
                                _ = self.inbound.send(Ok(decrypted));
                            }
                            Some(Ok(message)) => return Err(ClientError::InvalidMessage { message })
                        }
                    }, // If we get data from the server, send it to the terminal.
                    msg_from_terminal = self.outbound.recv() => {
                        match msg_from_terminal {
                            None => closed = true,
                            Some(message) => {
                                let encrypted = self.session_key.session_encrypt_message(message)?;
                                self.tcp_stream.send(message::Serverbound::Message(encrypted)).await?;
                            },
                        }
                    }, // If we get data from the terminal, send it to the server
                };
                Ok(())
            }.await;

            if let Err(e) = result {
                _ = self.inbound.send(Err(e));
            }
        }
    }
}

pub struct TerminalTask {
    // Chat client takes stuff from the terminal and sends it.
    pub inbound: UnboundedReceiver<Result<String, ClientError>>,
    pub outbound: UnboundedSender<String>,
}
impl TerminalTask {
    fn new(
        inbound: UnboundedReceiver<Result<String, ClientError>>,
        outbound: UnboundedSender<String>,
    ) -> Self {
        TerminalTask { inbound, outbound }
    }
}

impl ChatClient {
    // Makes a connection to the port:hostname, and sets our username, does not set the pub/priv key pair yet.
    fn new(message_stream: MessageStream, session_key: SessionKey) -> Self {
        // Make a pair of channels for msg from Terminal to Client thread (outbound messages; user sent).
        // Then from client to terminal (inbound messages; terminal sent).
        // Goal is to separate inbound from outbound stuff to a background task.
        let (inbound_tx, inbound_rx) = unbounded_channel();
        let (outbound_tx, outbound_rx) = unbounded_channel();

        let terminal_task = TerminalTask::new(inbound_rx, outbound_tx);
        let client_task = ClientTask::new(inbound_tx, outbound_rx, message_stream, session_key);

        // Spawn our client background task.
        tokio::spawn(client_task.run_background_task());

        ChatClient { terminal_task }
    }
}
