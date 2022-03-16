#![allow(unused)]
use crate::message::{self, Clientbound, MessageCodec, Serverbound};
use crate::server::Client;
use bytes::Bytes;
use futures::SinkExt;
use openssl::{
    envelope::Open,
    error::{Error as SslError, ErrorStack},
    pkey::Private,
    rsa::{Padding, Rsa},
};
use sha3::{Digest, Sha3_512};
use std::io::Read;
use std::marker::PhantomData;
use thiserror::Error;
use tokio::{
    net::{TcpListener, TcpStream},
    sync::mpsc::{error, unbounded_channel, UnboundedReceiver, UnboundedSender},
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

    #[error(transparent)]
    SendStringError(#[from] error::SendError<String>),
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
    pub keypair: Rsa<Private>,
}
impl ClientTask {
    fn new(
        // From terminal
        inbound: UnboundedSender<String>,
        // To server
        outbound: UnboundedReceiver<String>,
        tcp_stream: MessageStream,
        keypair: Rsa<Private>,
    ) -> Self {
        ClientTask {
            inbound,
            outbound,
            tcp_stream,
            keypair,
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
                        Some(Ok(message::Clientbound::Message{message})) => {
                            self.inbound.send(message);
                        }
                    }
                }, // If we get data from the server, send it to the terminal.
                msg_from_terminal = self.outbound.recv() => {
                    match msg_from_terminal {
                        None => {break;},
                        Some(message) => {self.tcp_stream.send(message::Serverbound::Message{message}).await.expect("todo: handle error");},
                    }
                }, // If we get data from the terminal, send it to the server
            }
        }

        // Sends inbound msg to terminal and receives outbound messages.
    }
}

// Chat client takes stuff from the terminal and sends it.
pub struct TerminalTask {
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
    pub async fn new(port: usize, hostname: String) -> Result<Self, ClientError> {
        // Make a pair of channels for msg from Terminal to Client thread (outbound messages; user sent).
        // Then from client to terminal (inbound messages; terminal sent).
        // Goal is to separate inbound from outbound stuff to a background task.
        let (inbound_tx, inbound_rx) = unbounded_channel();
        let (outbound_tx, outbound_rx) = unbounded_channel();

        let mut message_stream = ChatClient::connect(hostname, port).await?;

        let keypair = ChatClient::generate_keypair(None)?;

        message_stream
            .send(message::Serverbound::Hello {
                public_key: keypair.public_key_to_pem()?,
            })
            .await?;

        let terminal_task = TerminalTask::new(inbound_rx, outbound_tx);
        let client_task = ClientTask::new(inbound_tx, outbound_rx, message_stream, keypair);

        // Spawn our client background task.
        tokio::spawn(client_task.run_background_task());

        Ok(ChatClient { terminal_task })
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

    // Takes no parameters, instead it'll attempt to connect to the stuff listed by
    async fn connect(hostname: String, port: usize) -> Result<MessageStream, ClientError> {
        // TcpStream is not a stream, it's a stream of bytes (NOT A STREAM), I guess it's an AsyncRead/AsyncWrite.
        let not_a_stream = TcpStream::connect(format!("{hostname}:{port}")).await?;
        let mut actual_stream = message::MessageCodec::default().framed(not_a_stream);
        Ok(actual_stream)
    }
}
