//! Server implementation

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
use tokio_util::codec::{self, Decoder};

use crate::{crypto, message};

/// Things that can go wrong server-side
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
    /// The set of clients connected to the server, identified by key fingerprint.
    clients: RwLock<HashMap<String, Client>>,
}

impl Server {
    /// Starts a background task for the server, completing once the server is up-and-running.
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

    /// Implementation of the server background task
    async fn _run(self, listener: net::TcpListener, mut shutdown_rx: sync::oneshot::Receiver<()>) {
        let s = Arc::new(self); // The server is reference-counted so that it can be shared among clients
        loop {
            tokio::select! {
                // Do we have an incoming connection?
                incoming = listener.accept() => {
                    match incoming {
                        // Start a new client
                        Ok((stream, addr)) => Client::start(s.clone(), stream, addr),
                        Err(e) => {
                            eprintln!("listen error: {e}");
                            break;
                        }
                    };
                },
                // Or have we been told to shut down?
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

    /// Registers a client in the server's list.
    pub async fn register_client(&self, client: Client) -> Result<(), Error> {
        match self
            .clients
            .write()
            .await
            .entry(client.key_fingerprint.clone())
        {
            Entry::Occupied(_) => Err(Error::DuplicateClient(client.public_key.clone())),
            Entry::Vacant(entry) => {
                entry.insert(client);
                Ok(())
            }
        }
    }

    /// Removes a client from the server's list.
    pub async fn unregister_client(&self, client: &str) {
        self.clients.write().await.remove(client);
    }
}

/// A handle the application can use to shut down the server.
pub struct ShutdownHandle {
    // A channel used to request a shutdown.
    tx: sync::oneshot::Sender<()>,

    // A join handle used to wait for a shutdown.
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

/// The server-side representation of a client.
/// The client is split into two structures; this is the external interface used by the server &
/// other clients. It contains some useful information about the client, and communication channels
/// that can be used to communicate with the client's event loop.
pub struct Client {
    /// This client's DER-encoded public key.
    public_key: Vec<u8>,

    /// This client's key fingerprint: the base64-encoded SHA3-256 hash of the public key.
    key_fingerprint: String,

    /// A channel that can be used to transmit packets to the client.
    /// The event loop monitors this channel & forwards any packets over the client's socket.
    tx: mpsc::UnboundedSender<message::Clientbound>,

    /// A channel that can be used to request the creation of a chat session with this client.
    /// To request a connection, send the public key fingerprint of the client requesting the
    /// connection, and a callback channel that on which to indicate whether the request succeeded.
    request_connection_tx: mpsc::UnboundedSender<(String, oneshot::Sender<bool>)>,

    /// The client's join handle, so the server can wait for its
    /// event loop to finish before shutting down.
    join_handle: task::JoinHandle<()>,
}

impl Client {
    /// Creates and runs a Client.
    /// Returns immediately, spawning a new task for the client.
    pub fn start(server: Arc<Server>, stream: net::TcpStream, address: std::net::SocketAddr) {
        // Create communication channels...
        let (tx, rx) = sync::mpsc::unbounded_channel();
        let (join_handle_tx, join_handle_rx) = sync::oneshot::channel();
        let (request_connection_tx, request_connection_rx) = mpsc::unbounded_channel();

        // and spawn a background tas for the client
        let join_handle = tokio::task::spawn(async move {
            let join_handle = join_handle_rx.await.unwrap();

            println!("Connected to {address}");

            // Wrap the raw TCP stream in a codec that encodes & decodes JSON packets
            // See src/message.rs
            let mut stream = message::MessageCodec::default().framed(stream);

            // Wait for the client to start the handshake
            let result = match stream.next().await {
                Some(Ok(message::Serverbound::Hello { public_key })) => {
                    async {
                        // We recieved a valid "hello" message from the client.
                        // Register the client with the server...
                        let key_fingerprint = crypto::sha3_256_base64(&public_key);
                        server
                            .register_client(Client {
                                public_key: public_key.clone(),
                                key_fingerprint: key_fingerprint.clone(),
                                tx,
                                request_connection_tx,
                                join_handle,
                            })
                            .await?;

                        // and run the client's event loop.
                        let result = ClientEventLoop {
                            server: server.clone(),
                            public_key,
                            key_fingerprint: key_fingerprint.clone(),
                            stream: &mut stream,
                            rx,

                            dst_client_fingerprint: None,
                            dst_client_request: None,

                            request_connection_rx,
                            outstanding_connection_requests: HashMap::new(),
                        }
                        .run()
                        .await;

                        // The event loop has terminated. Unregister the client from the server.
                        server.unregister_client(&key_fingerprint).await;
                        result
                    }
                    .await
                }

                // The client should not send anything but a "hello" packet.
                Some(Ok(m)) => Err(Error::UnexpectedMessage(m)),
                Some(Err(e)) => Err(Error::from(e)),
                None => Ok(()),
            };

            // The client has exited; log the disconnect.
            match result {
                Ok(()) => println!("Client {address} disconnected"),
                Err(e) => {
                    // If it was due to an error, log the error and forward it to the client.
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

/// The implementation of the server's per-client event loop.
struct ClientEventLoop<'s> {
    /// A reference to the server.
    server: Arc<Server>,
    public_key: Vec<u8>,
    key_fingerprint: String,
    stream: &'s mut codec::Framed<
        net::TcpStream,
        message::MessageCodec<message::Clientbound, message::Serverbound>,
    >,
    rx: mpsc::UnboundedReceiver<message::Clientbound>,

    dst_client_fingerprint: Option<String>,
    dst_client_request: Option<oneshot::Receiver<bool>>,

    request_connection_rx: mpsc::UnboundedReceiver<(String, oneshot::Sender<bool>)>,
    outstanding_connection_requests: HashMap<String, oneshot::Sender<bool>>,
}

impl<'s> ClientEventLoop<'s> {
    /// Runs the client event loop.
    async fn run(&mut self) -> Result<(), Error> {
        loop {
            // Wait for an event...
            tokio::select! {
                message = self.stream.next() => {
                    match message.transpose()? {
                        // We recieved a message from the client.
                        Some(m) => self.handle_message(m).await?,

                        // The client closed the connection; exit the event loop.
                        None => break,
                    }
                },
                message = self.rx.recv() => {
                    match message {
                        // We have a message to be sent to the client.
                        Some(m) => self.stream.feed(m).await?,

                        // The server is shutting down.
                        None => break,
                    }
                }
                request = self.request_connection_rx.recv() => {
                    match request {
                        // Someone requested a chat session with this client.
                        Some((peer, resp)) => if self.dst_client_fingerprint.is_none() {
                            // Keep track of the request.
                            self.outstanding_connection_requests.insert(peer, resp);
                        } else {
                            // We already have a session, reject the request.
                            _ = resp.send(false);
                        }
                        None => {
                            _ = self.stream.feed(message::Clientbound::Shutdown {
                                message: "Destination client disconnected".to_string(),
                            }).await;
                        }
                    }
                }
                response = async { self.dst_client_request.as_mut().unwrap().await }, if self.dst_client_request.is_some() => {
                    // Someone responded to our request for a chat session.

                    // We no longer have an outstanding request.
                    self.dst_client_request = None;

                    if response.unwrap_or(false) {
                        // The other client said yes. (Arbitrarily) select our client to generate a session key
                        let dst_fingerprint = self.dst_client_fingerprint.as_ref().unwrap();
                        if let Some(dst_key) = self.server.clients.read().await.get(dst_fingerprint).map(|c| &c.public_key) {
                            self.stream.feed(message::Clientbound::ChooseKey { dst_public_key: dst_key.clone() }).await?;
                        } else {
                            self.stream.feed(message::Clientbound::Shutdown {
                                message: "Destination client disconnected".to_string(),
                            }).await?;
                        }
                    } else {
                        // The other client said no (or shut down).
                        self.dst_client_fingerprint = None;
                        self.stream.feed(message::Clientbound::Shutdown {
                            message: "Destination client rejected connection".to_string(),
                        }).await?;
                    }
                }
            };
            self.stream.flush().await?;
        }

        // We've exited the event loop. If we have an active session,
        // let the other end know we've disconnected
        if let Some(dst_fingerprint) = self.dst_client_fingerprint.as_ref() {
            if let Some(dst_client) = self.server.clients.read().await.get(dst_fingerprint) {
                _ = dst_client.tx.send(message::Clientbound::Shutdown {
                    message: "Remote peer ended the chat".to_string(),
                });
            }
        }

        self.stream.flush().await?;
        Ok(())
    }

    /// Handles packets recieved from the client over the network.
    async fn handle_message(&mut self, message: message::Serverbound) -> Result<(), Error> {
        match message {
            message::Serverbound::ConnectRequest {
                ref peer_fingerprint,
            } => {
                // The client has asked to begin a chat session.

                // Make sure we don't already have an active session or outstanding request.
                if self.dst_client_request.is_some() || self.dst_client_fingerprint.is_some() {
                    return Err(Error::UnexpectedMessage(message));
                }

                let clients = self.server.clients.read().await;

                // We hold a lock on clients, so only one client can be in this function at a time.
                // To avoid race conditions, make sure we process any incoming connection requests
                // before submitting one of our own.
                while let Ok((peer, channel)) = self.request_connection_rx.try_recv() {
                    self.outstanding_connection_requests.insert(peer, channel);
                }

                // Make sure the destination client exists
                let dst_client = clients.get(peer_fingerprint);
                if dst_client.is_none() {
                    self.stream
                        .feed(message::Clientbound::ConnectFail {
                            reason: "No client with the specified key fingerprint is connected."
                                .to_string(),
                        })
                        .await?;
                    return Ok(());
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
                    // Submit a connection request to this peer.
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

                // If there are any outsanding connection requests to other clients, reject them.
                self.outstanding_connection_requests
                    .drain()
                    .for_each(|req| _ = req.1.send(false));

                self.dst_client_fingerprint = Some(peer_fingerprint.clone());
                self.stream
                    .feed(message::Clientbound::ConnectWaiting)
                    .await?;
            }
            message::Serverbound::UseKey {
                session_key,
                signature,
            } => {
                // The client has given us an encrypted session key to forward to its peer.

                // Make sure it actually has a peer
                if let Some(dst_fingerprint) = self.dst_client_fingerprint.as_ref() {
                    if let Some(dst_client) = self.server.clients.read().await.get(dst_fingerprint)
                    {
                        _ = dst_client.tx.send(message::Clientbound::UseKey {
                            dst_public_key: self.public_key.clone(),
                            session_key,
                            signature,
                        });
                    } else {
                        self.stream
                            .feed(message::Clientbound::Shutdown {
                                message: "Destination client disconnected".to_string(),
                            })
                            .await?;
                    }
                } else {
                    return Err(Error::UnexpectedMessage(message::Serverbound::UseKey {
                        session_key,
                        signature,
                    }));
                }
            }
            message::Serverbound::Message(m) => {
                // The client has sent us an encrypted message to forward to its peer.
                println!("Intercepted ciphertext: {m:?}");

                // Make sure it actually has a peer
                if let Some(dst_fingerprint) = self.dst_client_fingerprint.as_ref() {
                    if let Some(dst_client) = self.server.clients.read().await.get(dst_fingerprint)
                    {
                        _ = dst_client.tx.send(message::Clientbound::Message(m));
                    } else {
                        self.stream
                            .feed(message::Clientbound::Shutdown {
                                message: "Destination client disconnected".to_string(),
                            })
                            .await?;
                    }
                } else {
                    return Err(Error::UnexpectedMessage(message::Serverbound::Message(m)));
                }
            }

            // A Hello message wouldn't make any sense, we've already completed the handshake
            message::Serverbound::Hello { .. } => return Err(Error::UnexpectedMessage(message)),
        };

        Ok(())
    }
}
