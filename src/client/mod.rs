use thiserror::Error;
use openssl::rsa::{Rsa, Padding};
use futures::prelude::*;
use tokio::net::TcpListener;
mod client;

#[derive(Error, Debug)]
pub enum ClientError {
    #[error(transparent)]
    IOError(#[from] std::io::Error),
}

pub struct ChatClient {
    // Stuff to fill later
    username: String,
    port: usize,
    hostname: String,
    private_key: Option<Vec<u8>>,
    public_key: Option<Vec<u8>>,
}

impl ChatClient {
<<<<<<< Updated upstream
    // Makes a connection to the port:hostname, and sets our username, does not set the pub/priv key pair yet.
    pub async fn new(port: usize, hostname: String, username: String) -> Self{
=======
    pub fn new(port: usize, hostname: String) -> Self{
>>>>>>> Stashed changes
        ChatClient{
            username,
            port,
            hostname,
            private_key: None,
            public_key: None
        }
    }

    fn generate_keypair(&mut self) {

    }

    fn hash_keys(&mut self){

    }

    fn server_pkey_exchange(&self){

    }

    // ---------------- Public API --------------------

    pub async fn terminate_connection(&mut self) {

    }

    pub async fn send_message(&mut self, msg: String) {

    }

    //
    pub async fn recieve_message(&mut self) {

    }

    pub async fn server_connect(&mut self) {
        
    }
}

