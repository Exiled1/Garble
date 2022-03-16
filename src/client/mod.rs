#![allow(unused)]
use std::io::Read;
use bytes::Bytes;
use futures::{prelude::*, channel::{oneshot::channel}};
use openssl::{
    envelope::Open,
    error::{Error as SslError, ErrorStack},
    rsa::{Padding, Rsa}, pkey::Private,
};
use sha3::{Digest, Sha3_512};
use thiserror::Error;
use tokio::{net::{TcpListener, TcpStream}, sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel}};
use tokio_stream::StreamExt;
use tokio_util::codec::{Framed, Decoder};
use crate::message::{self, MessageCodec, Serverbound, Clientbound};
use crate::server::Client;

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
    pub inbound: UnboundedReceiver<String>,
    pub outbound: UnboundedSender<String>
}

impl ChatClient {
    // Makes a connection to the port:hostname, and sets our username, does not set the pub/priv key pair yet.
    pub async fn new(port: usize, hostname: String, username: String) -> Result<Self, ClientError> {
        // Make a pair of channels for msg from Terminal to Client thread (outbound messages; user sent).
        // Then from client to terminal (inbound messages; terminal sent).
        // Goal is to separate inbound from outbound stuff to a background task.
        let (inbound_tx, inbound_rx) = unbounded_channel();
        let (outbound_tx, outbound_rx) = unbounded_channel();
        // tokio::spawn({

        // })
        Ok(ChatClient {
            // port,
            // hostname: hostname.clone(),
            // keypair: ChatClient::generate_keypair(None)?,
            // tcp_stream: ChatClient::connect(hostname, port).await?
            inbound: inbound_rx,
            outbound: outbound_tx
        })
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
    fn stream_send(&mut self, server_bound_msg: message::Serverbound){

    }
    // ---------------- Public API --------------------

    async fn encrypt_message(&self, msg: String) -> Result<Vec<u8>, ClientError> {
        Ok(vec![1,2,3,4])
    }

    pub async fn terminate_connection(&mut self) -> Result<(), ClientError> { 
        Ok(())
    }

    // Background Tokio task to send a message. However, 
    pub async fn send_message(&mut self, msg: String) -> Result<(), ClientError>{

        Ok(())
    }


    // Takes no parameters, instead it'll attempt to connect to the stuff listed by
    pub async fn connect(hostname: String, port: usize) -> Result<MessageStream, ClientError>{
        // TcpStream is not a stream, it's a stream of bytes (NOT A STREAM), I guess it's an AsyncRead/AsyncWrite.
        let not_a_stream = TcpStream::connect(format!("{hostname}:{port}")).await?;
        let mut actual_stream = message::MessageCodec::default().framed(not_a_stream);
        Ok(actual_stream)
    }   
}
