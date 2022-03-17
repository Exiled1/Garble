use futures::prelude::*;
use std::{
    collections::{hash_map::Entry, HashMap},
    ops::DerefMut,
    sync::Arc,
};
use thiserror::Error;
use tokio::{
    net,
    sync::{self, mpsc, oneshot, RwLock},
    task,
};
use tokio_util::codec::Decoder;

use crate::message;

#[derive(Error, Debug)]
pub enum Error {
    #[error("unexpected message: {0:?}")]
    UnexpectedMessage(message::Serverbound),

    #[error("duplicate client (public key fingerprint {0:?} is already connected)")]
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
    tx: mpsc::UnboundedSender<message::Clientbound>,
    request_connection_tx: mpsc::UnboundedSender<(Vec<u8>, oneshot::Sender<bool>)>,
    join_handle: task::JoinHandle<()>,
}

impl Client {
    /// Creates and runs a Client.
    /// Returns immediately, spawning a new task for the client.
    pub fn start(server: Arc<Server>, stream: net::TcpStream, address: std::net::SocketAddr) {
        let (tx, rx) = sync::mpsc::unbounded_channel();
        let (join_handle_tx, join_handle_rx) = sync::oneshot::channel();
        let (request_connection_tx, request_connection_rx) = mpsc::unbounded_channel();
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
                                request_connection_tx,
                                join_handle,
                            })
                            .await?;

                        // and run the client's event loop.
                        #[allow(unreachable_code, clippy::diverging_sub_expression)]
                        let result = ClientEventLoop {
                            server,
                            key_fingerprint: todo!(),
                            stream: &mut stream,
                            rx,

                            dst_client_fingerprint: None,
                            dst_client_request: None,

                            request_connection_rx,
                            outstanding_connection_requests: HashMap::new(),
                        }
                        .run()
                        .await;

                        if let Err(e) = result {
                            println!("Client {address} disconnected due to error: {e}");
                        }

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
    server: Arc<Server>,
    key_fingerprint: Vec<u8>,
    stream: &'s mut S,
    rx: mpsc::UnboundedReceiver<message::Clientbound>,

    dst_client_fingerprint: Option<Vec<u8>>,
    dst_client_request: Option<oneshot::Receiver<bool>>,

    request_connection_rx: mpsc::UnboundedReceiver<(Vec<u8>, oneshot::Sender<bool>)>,
    outstanding_connection_requests: HashMap<Vec<u8>, oneshot::Sender<bool>>,
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
                request = self.request_connection_rx.recv() => {
                    match request {
                        Some((peer, resp)) => if self.dst_client_fingerprint.is_none() {
                            self.outstanding_connection_requests.insert(peer, resp);
                        } else {
                            // We already have a connection, reject the request.
                            _ = resp.send(false);
                        }
                        None => {
                            _ = self.stream.feed(message::Clientbound::Shutdown {
                                message: "Destination client disconnected".to_string(),
                            }).await;
                        }
                    }
                }
                response = self.dst_client_request.as_mut().unwrap(), if self.dst_client_request.is_some() => {
                    if response.unwrap_or(false) {
                        // The other client said yes. Select our client to generate a session key
                        let dst_fingerprint = self.dst_client_fingerprint.as_ref().unwrap();
                        if let Some(dst_key) = self.server.clients.read().await.get(dst_fingerprint).map(|c| &c.public_key) {
                            self.stream.feed(message::Clientbound::ChooseKey { dst_public_key: dst_key.clone() }).await?;
                        } else {
                            self.stream.feed(message::Clientbound::Shutdown {
                                message: "Destination client disconnected".to_string(),
                            }).await?;
                        }
                    } else {
                        self.stream.feed(message::Clientbound::Shutdown {
                            message: "Destination client rejected connection".to_string(),
                        }).await?;
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
            message::Serverbound::ConnectRequest {
                ref peer_fingerprint,
            } => {
                if self.dst_client_request.is_some() || self.dst_client_fingerprint.is_some() {
                    return Err(Error::UnexpectedMessage(message));
                }

                let clients = self.server.clients.read().await;

                // We have a lock on clients, so only one client can be in this function at a time.
                // To avoid race conditions, make sure we process any incoming connection requests
                while let Ok((peer, channel)) = self.request_connection_rx.try_recv() {
                    self.outstanding_connection_requests.insert(peer, channel);
                }

                // Make sure the client exists
                let dst_client = clients.get(peer_fingerprint);
                if dst_client.is_none() {
                    self.stream
                        .feed(message::Clientbound::ConnectFail {
                            reason: "No client with the specified key fingerprint is connected."
                                .to_string(),
                        })
                        .await?;
                }
                let dst_client = dst_client.unwrap();

                // Do we have an outstanding connection request from this peer?
                if let Some(req) = self
                    .outstanding_connection_requests
                    .remove(peer_fingerprint)
                {
                    // Yes, approve the request. The peer will then begin the key exchange.
                    if req.send(true).is_err() {
                        self.stream
                            .feed(message::Clientbound::Shutdown {
                                message: "Destination client disconnected".to_string(),
                            })
                            .await?;
                    }
                } else {
                    // Submit a connection request to this client.
                    let (request_tx, request_rx) = oneshot::channel();
                    self.dst_client_request = Some(request_rx);
                    let result = dst_client
                        .request_connection_tx
                        .send((self.key_fingerprint.clone(), request_tx));
                    if result.is_err() {
                        self.stream
                            .feed(message::Clientbound::Shutdown {
                                message: "Destination client disconnected".to_string(),
                            })
                            .await?;
                    }
                }

                // If there are any other outsanding connection requests, reject them.
                self.outstanding_connection_requests
                    .drain()
                    .for_each(|req| _ = req.1.send(false));

                self.dst_client_fingerprint = Some(peer_fingerprint.clone());
                self.stream
                    .feed(message::Clientbound::ConnectWaiting)
                    .await?;
            }
            message::Serverbound::Message {
                ref encrypted_content,
                ref tag,
            } => {
                if let Some(dst_fingerprint) = self.dst_client_fingerprint.as_ref() {
                    if let Some(dst_client) = self.server.clients.read().await.get(dst_fingerprint)
                    {
                        _ = dst_client.tx.send(message::Clientbound::Message {
                            encrypted_content: encrypted_content.clone(),
                            tag: tag.clone(),
                        });
                    } else {
                        self.stream
                            .feed(message::Clientbound::Shutdown {
                                message: "Destination client disconnected".to_string(),
                            })
                            .await?;
                    }
                } else {
                    return Err(Error::UnexpectedMessage(message));
                }
            }
            m => return Err(Error::UnexpectedMessage(m)),
        };

        Ok(())
    }
}
