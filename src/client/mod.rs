#![allow(unused)]
use crate::message::{self, Clientbound, MessageCodec, Serverbound};
use crate::server::Client;
use bytes::Bytes;
use futures::SinkExt;
use openssl::{
    error::{Error as SslError, ErrorStack},
    pkey::Private,
    rsa::{Padding, Rsa},
    sha::Sha256,
};
use std::io::Read;
use std::marker::PhantomData;
use std::string::FromUtf8Error;
use thiserror::Error;
use tokio::{
    net::{TcpListener, TcpStream},
    sync::mpsc::{error, unbounded_channel, UnboundedReceiver, UnboundedSender},
};
use tokio_stream::StreamExt;
use tokio_util::codec::{Decoder, Framed};
mod crypto;
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

    #[error("server shutdown: {}", reason.as_deref().unwrap_or("connection closed").to_string())]
    ServerShutdown { reason: Option<String> },

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
}

impl ChatConnector {
    async fn new(hostname: &str, port: usize) -> Result<Self, ClientError> {
        let keypair = ChatClient::generate_keypair(None)?;

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
    async fn request_connection(
        self,
        peer_fingerprint: Vec<u8>,
    ) -> Result<ChatClient, ConnectError> {
        // Choose a session key & encrypt it with the user's public key
        let proposed_session_key = todo!();

        self.tcp_stream
            .send(message::Serverbound::ConnectRequest { peer_fingerprint })
            .await
            .map_err(ClientError::from)?;

        loop {
            match self.tcp_stream.next().await {
                Some(Ok(message::Clientbound::ConnectWaiting)) => continue,
                Some(Ok(message::Clientbound::ChooseKey { dst_public_key })) => {
                    break Ok(ChatClient::new(
                        self.tcp_stream,
                        todo!("generate peer session key, encrypt with RSA"),
                    ))
                }
                Some(Ok(message::Clientbound::UseKey {
                    dst_public_key,
                    session_key,
                    signature,
                })) => {
                    break Ok(ChatClient::new(
                        self.tcp_stream,
                        todo!("decrypt peer session key with RSA"),
                    ))
                }
                Some(Ok(message::Clientbound::ConnectFail { reason })) => {
                    break Err(ConnectError::ConnectRefused {
                        connector: self,
                        reason,
                    })
                }

                Some(Ok(message::Clientbound::Shutdown { message })) => {
                    break Err(ConnectError::from(ClientError::ServerShutdown {
                        reason: Some(message),
                    }))
                }
                None => {
                    break Err(ConnectError::from(ClientError::ServerShutdown {
                        reason: None,
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
    pub inbound: UnboundedSender<String>,
    pub outbound: UnboundedReceiver<String>,
    pub tcp_stream: MessageStream,
    session_key: Vec<u8>,
}
impl ClientTask {
    fn new(
        // From terminal
        inbound: UnboundedSender<String>,
        // To server
        outbound: UnboundedReceiver<String>,
        tcp_stream: MessageStream,
        session_key: Vec<u8>,
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
        loop {
            // recieves OUTBOUND data FROM the terminal and sends SERVERBOUND data TO the server. (Gods this was confusing LOL)

            // Any data we recieve from the terminal, we send to the server
            // Any data we get from the server we send to the terminal.
            // self.inbound.send().await
            // self.outbound.recv().await // user messages

            tokio::select! {
                msg_from_server  = self.tcp_stream.next() => {
                    match msg_from_server {
                        None => {break;},
                        Some(Err(e)) => {todo!("{e}")},
                        Some(Ok(message::Clientbound::Shutdown{message})) => {todo!("{message}")}
                        Some(Ok(message::Clientbound::Message{encrypted_content, tag})) => {
                            todo!()
                        }
                        Some(Ok(message)) => todo!("{message:?}")
                    }
                }, // If we get data from the server, send it to the terminal.
                msg_from_terminal = self.outbound.recv() => {
                    match msg_from_terminal {
                        None => {break;},
                        Some(message) => todo!(),
                    }
                }, // If we get data from the terminal, send it to the server
            }
        }

        // Sends inbound msg to terminal and receives outbound messages.
    }
}

pub struct TerminalTask {
    // Chat client takes stuff from the terminal and sends it.
    pub inbound: UnboundedReceiver<String>,
    pub outbound: UnboundedSender<String>,
}
impl TerminalTask {
    fn new(inbound: UnboundedReceiver<String>, outbound: UnboundedSender<String>) -> Self {
        TerminalTask { inbound, outbound }
    }
}

impl ChatClient {
    // Makes a connection to the port:hostname, and sets our username, does not set the pub/priv key pair yet.
    fn new(message_stream: MessageStream, session_key: Vec<u8>) -> Self {
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

    // This generates a pub/priv keypair for making a server connection, generates a new keypair if there already exists one. Note: Should not be called outside of the server_connect function.

    fn generate_keypair(keysize: Option<u32>) -> Result<Rsa<Private>, ClientError> {
        let keypair = Rsa::generate(keysize.unwrap_or(2048))?;
        Ok(keypair)
    }

    // Simply hashes the public key and returns the string result of it. This is for identifying a user while giving no information to the server.
    fn hash_pub_key(keypair: Rsa<Private>) -> Result<Vec<u8>, ClientError> {
        let mut hasher = Sha256::new();
        hasher.update(keypair.public_key_to_der().as_ref().unwrap());
        let hash = hasher.finish();
        let hashed_pub_key = hash.bytes().collect::<Result<Vec<u8>, std::io::Error>>()?;
        Ok(hashed_pub_key)
    }

    // Client hashes the pkey and sends the result to the server, returns Result<(), ClientError>.
    fn server_pkey_exchange(&self) -> Result<(), ClientError> {
        Ok(())
    }

    // The private method that takes a message from send_message and performs correct IO.
    fn stream_send(&mut self, server_bound_msg: message::Serverbound) {}
    // ---------------- Public API --------------------
}
