//! The client module, which handles all communication with the server.
//! The client is split into two Tokio tasks: the terminal task (in src/bin/client.rs), and
//! the client task which runs in the background (in this file).

use crate::crypto::SessionKey;
use crate::message::{self, Clientbound, MessageCodec, Serverbound};
use futures::SinkExt;
use std::string::FromUtf8Error;
use thiserror::Error;
use tokio::{
    net::TcpStream,
    sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender},
};
use tokio_stream::StreamExt;
use tokio_util::codec::Framed;

// Re-export public interfaces from submodules
mod readline;
pub use readline::Readline;

mod connect;
pub use connect::{ChatConnector, ConnectError};

/// A TCP stream that carries null-terminated JSON messages as described in src/message.rs.
type MessageStream = Framed<TcpStream, MessageCodec<Serverbound, Clientbound>>;

/// An error that can occur while the client is running.
#[derive(Error, Debug)]
pub enum ClientError {
    /// Something went wrong with the network.
    #[error(transparent)]
    IoError(#[from] std::io::Error),

    /// Something went wrong with a cryptographic operation.
    #[error(transparent)]
    SslError(#[from] openssl::error::ErrorStack),

    /// A key we recieved could not be verified.
    #[error("Invalid signature")]
    InvalidSignature,

    /// A message was not valid UTF-8.
    #[error("Invalid utf8 conversion")]
    InvalidConversion(#[from] FromUtf8Error),

    /// The connection was closed by the server.
    #[error("Server terminated connection: {reason}")]
    ServerShutdown { reason: String },

    /// The server sent a message that was not expected in this context.
    #[error("invalid message recieved from server: {message:?}")]
    InvalidMessage { message: message::Clientbound },
}

/// This structure is held by the terminal task and is the public interface to the client task. It
/// contains channels that the terminal task can use to send & recieve messages.
pub struct ChatClient {
    /// A channel delivering the stream of messages & errors recieved by the client task.
    pub inbound: UnboundedReceiver<Result<String, ClientError>>,

    /// A channel for sending messages to the client task.
    pub outbound: UnboundedSender<String>,
}
impl ChatClient {
    /// Creates a ChatClient, given a connection to the server & a sesion key
    ///
    /// This method should only be called by ChatConnector, since it requires completing the
    /// initial handshake to determine the session key.
    fn new(message_stream: MessageStream, session_key: SessionKey) -> Self {
        // Make a pair of channels for msg from Terminal to Client thread (outbound messages; user sent).
        // Then from client to terminal (inbound messages; terminal sent).
        // Goal is to separate inbound from outbound stuff to a background task.
        let (inbound_tx, inbound_rx) = unbounded_channel();
        let (outbound_tx, outbound_rx) = unbounded_channel();

        let client_task = ClientTask::new(inbound_tx, outbound_rx, message_stream, session_key);

        // Spawn our client background task.
        tokio::spawn(client_task.run_background_task());

        ChatClient {
            inbound: inbound_rx,
            outbound: outbound_tx,
        }
    }
}

/// This structure contains the internal state of the client task.
struct ClientTask {
    /// Sends messages & errors to the terminal task.
    inbound: UnboundedSender<Result<String, ClientError>>,

    /// Recieves messages from the terminal task.
    outbound: UnboundedReceiver<String>,

    /// The stream of packets to & from the server.
    tcp_stream: MessageStream,

    /// The AES-256-GCM key used to encrypt messages.
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
                        // We got a message! What is it?
                        match msg_from_server {
                            // The connection has been closed; stop.
                            None => closed = true,

                            // Something went wrong, bail out.
                            Some(Err(e)) => return Err(ClientError::from(e)),

                            // The server has asked us to shut down, stop & raise an "error".
                            Some(Ok(message::Clientbound::Shutdown{message})) => {
                                closed = true;
                                return Err(ClientError::ServerShutdown { reason: message });
                            }

                            // We've recieved an encrypted message.
                            Some(Ok(message::Clientbound::Message(m))) => {
                                // Decrypt it...
                                let decrypted = self.session_key.session_decrypt_message(m)?;
                                // and send it to the terminal atsk
                                _ = self.inbound.send(Ok(decrypted));
                            }

                            // We've recieved some other message, which doesn't make any sense.
                            Some(Ok(message)) => return Err(ClientError::InvalidMessage { message })
                        }
                    },
                    msg_from_terminal = self.outbound.recv() => {
                        // The user entered something on the terminal!
                        match msg_from_terminal {
                            // The terminal has been closed, stop.
                            None => closed = true,

                            Some(message) => {
                                // The user entered a message. Encrypt it...
                                let encrypted = self.session_key.session_encrypt_message(message)?;
                                // ...and send it over the network.
                                self.tcp_stream.send(message::Serverbound::Message(encrypted)).await?;
                            },
                        }
                    },
                };
                Ok(())
            }.await;

            if let Err(e) = result {
                // If we returned an error, forward it to the terminal task.
                _ = self.inbound.send(Err(e));
            }
        }
    }
}
