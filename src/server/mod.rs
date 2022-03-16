use futures::prelude::*;
use std::{
    collections::{hash_map::Entry, HashMap},
    ops::DerefMut,
    sync::Arc,
};
use thiserror::Error;
use tokio::{
    net,
    sync::{self, mpsc, RwLock},
    task,
};
use tokio_util::codec::Decoder;

use crate::message;

#[derive(Error, Debug)]
pub enum Error {
    #[error("unexpected message: {0:?}")]
    UnexpectedMessage(message::Serverbound),
    #[error("duplicate client (public key {0:?} is already connected)")]
    DuplicateClient(Vec<u8>),
    #[error("I/O error")]
    IoError(
        #[from]
        #[source]
        std::io::Error,
    ),
}

/// The server, which listens for and keeps track of clients.
pub struct Server {
    clients: RwLock<HashMap<Vec<u8>, Client>>,
}

impl Server {
    /// Starts the server, completing once the server is up-and-running.
    pub async fn start(bind_addr: &str) -> std::io::Result<ShutdownHandle> {
        let server = Server {
            clients: RwLock::new(HashMap::new()),
        };

        let listener = net::TcpListener::bind(bind_addr).await?;
        let (shutdown_tx, shutdown_rx) = sync::oneshot::channel();
        let join_handle = tokio::task::spawn(server._run(listener, shutdown_rx));

        Ok(ShutdownHandle {
            tx: shutdown_tx,
            join: join_handle,
        })
    }

    async fn _run(self, listener: net::TcpListener, mut shutdown_rx: sync::oneshot::Receiver<()>) {
        let s = Arc::new(self);
        loop {
            tokio::select! {
                incoming = listener.accept() => {
                    match incoming {
                        Ok((stream, addr)) => Client::start(s.clone(), stream, addr),
                        Err(e) => {
                            eprintln!("listen error: {e}");
                            break;
                        }
                    };
                },
                _ = &mut shutdown_rx => {
                    break
                }
            }
        }

        // Let clients know that we're shutting down.
        let clients = std::mem::take(s.clients.write().await.deref_mut()); // don't hold the lock
        for (_id, client) in clients.into_iter() {
            _ = client
                .shutdown(Some("Server shutting down".to_string()))
                .await;
        }
    }

    pub async fn register_client(&self, client: Client) -> Result<(), Error> {
        match self.clients.write().await.entry(client.public_key.clone()) {
            Entry::Occupied(_) => Err(Error::DuplicateClient(client.public_key.clone())),
            Entry::Vacant(entry) => {
                entry.insert(client);
                Ok(())
            }
        }
    }

    pub async fn unregister_client(&self, client: &[u8]) {
        self.clients.write().await.remove(client);
    }
}

/// A handle the application can use to shut down the server.
pub struct ShutdownHandle {
    tx: sync::oneshot::Sender<()>,
    join: tokio::task::JoinHandle<()>,
}

impl ShutdownHandle {
    /// Terminates the server, waiting for it to stop and returning its exit status.
    pub async fn shutdown(self) -> Result<(), tokio::task::JoinError> {
        self.tx
            .send(())
            .expect("send cannot fail; the server is not closed");
        self.join.await
    }

    /// Waits for the server to stop.
    pub async fn stopped(&mut self) {
        self.tx.closed().await
    }

    /// Returns true if the server has stopped.
    pub fn is_stopped(&self) -> bool {
        self.tx.is_closed()
    }
}

pub struct Client {
    public_key: Vec<u8>,
    /* TODO: we'll probably want this eventually
    address: std::net::SocketAddr,
    */
    tx: mpsc::UnboundedSender<message::Clientbound>,
    join_handle: task::JoinHandle<()>,
}

impl Client {
    /// Creates and runs a Client.
    /// Returns immediately, spawning a new task for the client.
    pub fn start(server: Arc<Server>, stream: net::TcpStream, address: std::net::SocketAddr) {
        let (tx, rx) = sync::mpsc::unbounded_channel();
        let (join_handle_tx, join_handle_rx) = sync::oneshot::channel();
        let join_handle = tokio::task::spawn(async move {
            let join_handle = join_handle_rx.await.unwrap();

            println!("Connected to {address}");
            let mut stream = message::MessageCodec::default().framed(stream);
            let result = match stream.next().await {
                Some(Ok(message::Serverbound::Hello { public_key })) => {
                    async {
                        // We recieved a valid "hello" message from the client.
                        // Register the client with the server...
                        server
                            .register_client(Client {
                                public_key: public_key.clone(),
                                //address,
                                tx,
                                join_handle,
                            })
                            .await?;

                        // and run the client's event loop.
                        let result = ClientEventLoop {
                            /*
                            server,
                            public_key,
                            address,
                            */
                            stream: &mut stream,
                            rx,
                        }
                        .run()
                        .await;

                        server.unregister_client(&public_key).await;
                        result
                    }
                    .await
                }
                Some(Ok(m)) => Err(Error::UnexpectedMessage(m)),
                Some(Err(e)) => Err(Error::from(e)),
                None => Ok(()),
            };

            match result {
                Ok(()) => println!("Client {address} disconnected"),
                Err(e) => {
                    println!("Client {address} disconnected: {e}");
                    _ = stream
                        .send(message::Clientbound::Shutdown {
                            message: e.to_string(),
                        })
                        .await;
                }
            }
        });
        // send the join handle to the client
        join_handle_tx.send(join_handle).unwrap();
    }

    /// Sends a message to the client.
    pub async fn send(&self, message: message::Clientbound) {
        self.tx
            .send(message)
            .expect("send cannot fail; the client is not closed");
    }

    /// Terminates the client, flushing all pending messages.
    pub async fn shutdown(self, message: Option<String>) -> Result<(), task::JoinError> {
        if let Some(m) = message {
            self.send(message::Clientbound::Shutdown { message: m })
                .await;
        }
        drop(self.tx);
        self.join_handle.await
    }
}

struct ClientEventLoop<
    's,
    S: Stream<Item = std::io::Result<message::Serverbound>>
        + Sink<message::Clientbound, Error = std::io::Error>
        + Unpin,
> {
    /* TODO: we'll want these eventually
    server: Arc<Server>,
    public_key: String,
    address: std::net::SocketAddr,
    */
    stream: &'s mut S,
    rx: mpsc::UnboundedReceiver<message::Clientbound>,
}

impl<
        's,
        S: Stream<Item = std::io::Result<message::Serverbound>>
            + Sink<message::Clientbound, Error = std::io::Error>
            + Unpin,
    > ClientEventLoop<'s, S>
{
    async fn run(&mut self) -> Result<(), Error> {
        loop {
            tokio::select! {
                message = self.stream.next() => {
                    match message.transpose()? {
                        Some(m) => self.handle_message(m).await?,
                        None => break,  // The client closed the connection.
                    }
                },
                message = self.rx.recv() => {
                    match message {
                        Some(m) => self.stream.feed(m).await?,
                        None => break, // The server closed the connection.
                    }
                }
            };
            self.stream.flush().await?;
        }
        self.stream.flush().await?;
        Ok(())
    }

    async fn handle_message(&mut self, message: message::Serverbound) -> Result<(), Error> {
        match message {
            
            m => return Err(Error::UnexpectedMessage(m)),
        };

        Ok(())
    }
}
