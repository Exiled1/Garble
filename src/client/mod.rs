#![allow(unused)]
use crate::message::{self, Clientbound, MessageCodec, Serverbound};
use crate::server::Client;
use bytes::Bytes;
use futures::{channel::oneshot::channel, prelude::*};
use openssl::{
    envelope::Open,
    error::{Error as SslError, ErrorStack},
    pkey::Private,
    rsa::{Padding, Rsa},
};
use sha3::{Digest, Sha3_512};
use std::io::Read;
use thiserror::Error;
use tokio::{
    net::{TcpListener, TcpStream},
    sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender},
};
use tokio_stream::StreamExt;
use tokio_util::codec::{Decoder, Framed};

type MessageStream = Framed<TcpStream, MessageCodec<Serverbound, Clientbound>>;

#[derive(Error, Debug)]
pub enum ClientError {
    #[error(transparent)]
    IOError(#[from] std::io::Error),

    #[error(transparent)]
    RsaError(#[from] ErrorStack),
    // #[error(transparent)]
    // HashingError(#[from] )
}

// We're gonna need to define the interface from Client to Server & from Server to Client, but can't be same struct b/c of ownership.
pub struct ChatClient {
    // port: usize,
    // hostname: String,
    // keypair: Rsa<Private>,
    // tcp_stream: MessageStream,
    pub terminal_channel: TerminalChannel,
}

// This channel sends stuff from the client to the terminal.
struct ClientChannel {
    pub inbound: UnboundedReceiver<String>,
    pub outbound: UnboundedSender<String>,
}
impl ClientChannel {
    fn new(inbound: UnboundedReceiver<String>, outbound: UnboundedSender<String>) -> Self {
        ClientChannel { inbound, outbound }
    }
}

// Chat client takes stuff from the terminal and sends it.
pub struct TerminalChannel {
    pub inbound: UnboundedSender<String>,
    pub outbound: UnboundedReceiver<String>,
    pub tcp_stream: MessageStream,
    pub keypair: Rsa<Private>,
}
impl TerminalChannel {
    fn new(
        inbound: UnboundedSender<String>,
        outbound: UnboundedReceiver<String>,
        tcp_stream: MessageStream,
        keypair: Rsa<Private>,
    ) -> Self {
        TerminalChannel { inbound, outbound, tcp_stream, keypair }
    }
}

impl ChatClient {
    // Makes a connection to the port:hostname, and sets our username, does not set the pub/priv key pair yet.
    pub async fn new(port: usize, hostname: String) -> Result<Self, ClientError> {
        // Make a pair of channels for msg from Terminal to Client thread (outbound messages; user sent).
        // Then from client to terminal (inbound messages; terminal sent).
        // Goal is to separate inbound from outbound stuff to a background task.
        let (inbound_tx, inbound_rx) = unbounded_channel();
        let (outbound_tx, outbound_rx) = unbounded_channel();

        let message_stream = ChatClient::connect(hostname, port).await?;

        let keypair = ChatClient::generate_keypair(None)?;

        let terminal_channel = TerminalChannel::new(inbound_tx, outbound_rx, message_stream, keypair);
        let client_channel = ClientChannel::new(inbound_rx, outbound_tx);

        // Spawn our client background task.
        tokio::spawn(ChatClient::run_background_task(client_channel));

        Ok(ChatClient {
            terminal_channel,
        })
    }

    // Yes, it looks backwards, tokio doesn't have a 2-way channel ;-;
    async fn run_background_task(client_channel: ClientChannel) {
        // Sends inbound msg to terminal and receives outbound messages.
    }
    // This generates a pub/priv keypair for making a server connection, generates a new keypair if there already exists one. Note: Should not be called outside of the server_connect function.

    fn generate_keypair(keysize: Option<u32>) -> Result<Rsa<Private>, ClientError> {
        let keypair = Rsa::generate(keysize.unwrap_or(2048))?;
        Ok(keypair)
    }

    // Simply hashes the public key and returns the string result of it. This is for identifying a user while giving no information to the server.
    // fn hash_pub_key(&mut self) -> Result<Vec<u8>, ClientError> {
    //     let mut hasher = Sha3_512::new();
    //     hasher.update(self.keypair.public_key_to_pem().as_ref().unwrap());
    //     let hash = hasher.finalize();
    //     let hashed_pub_key = hash.bytes().collect::<Result<Vec<u8>, std::io::Error>>()?;
    //     Ok(hashed_pub_key)
    // }

    // Client hashes the pkey and sends the result to the server, returns Result<(), ClientError>.
    fn server_pkey_exchange(&self) -> Result<(), ClientError> {
        Ok(())
    }

    // The private method that takes a message from send_message and performs correct IO.
    fn stream_send(&mut self, server_bound_msg: message::Serverbound) {}
    // ---------------- Public API --------------------

    async fn encrypt_message(&self, msg: String) -> Result<Vec<u8>, ClientError> {
        Ok(vec![1, 2, 3, 4])
    }

    pub async fn terminate_connection(&mut self) -> Result<(), ClientError> {
        Ok(())
    }

    // Background Tokio task to send a message. However,
    pub async fn send_message(&mut self, msg: String) -> Result<(), ClientError> {
        Ok(())
    }

    // Takes no parameters, instead it'll attempt to connect to the stuff listed by
    async fn connect(hostname: String, port: usize) -> Result<MessageStream, ClientError> {
        // TcpStream is not a stream, it's a stream of bytes (NOT A STREAM), I guess it's an AsyncRead/AsyncWrite.
        let not_a_stream = TcpStream::connect(format!("{hostname}:{port}")).await?;
        let mut actual_stream = message::MessageCodec::default().framed(not_a_stream);
        Ok(actual_stream)
    }
}
