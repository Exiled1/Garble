use thiserror::Error;

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
    port: usize,
    hostname: String,
}

impl ChatClient {
    fn new(port: usize, hostname: String) -> Self{
        ChatClient{
            port,
            hostname
        }
    }
}

