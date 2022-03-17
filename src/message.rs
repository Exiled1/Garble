use bytes::Buf;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use tokio_util::codec;

/// Packets that can be sent from the client to the server.
#[derive(Serialize, Deserialize, Debug)]
pub enum Serverbound {
    /// Sent on startup to register the client with the server.
    /// Includes the client's DER-encoded public key.
    Hello { public_key: Vec<u8> },

    /// Sent by a client to request an E2E-encrypted channel with another client.
    /// Includes the key fingerprint of the peer to chat with (the SHA3-256 hash of their key),
    ConnectRequest { peer_fingerprint: String },

    /// The client's response to a ChooseKey packet.
    UseKey {
        session_key: Vec<u8>,
        signature: Vec<u8>,
    },

    /// Send an encrypted message to another client.
    Message(Encrypted),
}

/// Packets that can be sent from the client to the server.
#[derive(Serialize, Deserialize, Debug)]
pub enum Clientbound {
    /// Indicates that a ConnectRequest has been recieved, and the server is waiting for the other
    /// client to accept the request.
    /// Once the connection is established, the server will send either a ChooseKey or UseKey
    /// packet.
    ConnectWaiting,

    /// The ConnectRequest failed. reason is a human-readable message describing the error.
    ConnectFail { reason: String },

    /// The server has established the connection and selected this client to generate the session key.
    /// The client should respond with a Serverbound::UseKey packet.
    ChooseKey { dst_public_key: Vec<u8> },

    /// The server has established the connection, and the other client has chosen a key.
    UseKey {
        dst_public_key: Vec<u8>,
        session_key: Vec<u8>,
        signature: Vec<u8>,
    },

    /// Another client has sent an encrypted message.
    Message(Encrypted),

    /// The server is terminating the connection. message is a human-readable reason for the
    /// shutdown.
    Shutdown { message: String },
}

/// A message that has been encrypted and MACed.
#[derive(Debug, Serialize, Deserialize)]
pub struct Encrypted {
    pub iv: Vec<u8>,
    pub ciphertext: Vec<u8>,
    pub tag: Vec<u8>,
}

#[derive(Debug)]
pub struct MessageCodec<Tx: Serialize, Rx: DeserializeOwned>(
    std::marker::PhantomData<Tx>,
    std::marker::PhantomData<Rx>,
);

impl<Tx: Serialize, Rx: DeserializeOwned> Default for MessageCodec<Tx, Rx> {
    fn default() -> Self {
        Self(Default::default(), Default::default())
    }
}

impl<Tx: Serialize + std::fmt::Debug, Rx: DeserializeOwned> codec::Encoder<Tx>
    for MessageCodec<Tx, Rx>
{
    type Error = std::io::Error;

    fn encode(&mut self, item: Tx, dst: &mut bytes::BytesMut) -> Result<(), Self::Error> {
        let encoded = serde_json::to_vec(&item)?;
        dst.reserve(encoded.len() + 1);
        dst.extend_from_slice(&encoded);
        dst.extend_from_slice(&[0]); // null terminator
        Ok(())
    }
}
impl<Tx: Serialize, Rx: DeserializeOwned + std::fmt::Debug> codec::Decoder
    for MessageCodec<Tx, Rx>
{
    type Error = std::io::Error;
    type Item = Rx;

    fn decode(&mut self, src: &mut bytes::BytesMut) -> Result<Option<Rx>, Self::Error> {
        if let Some(end_index) = src.iter().position(|&x| x == 0) {
            let buf = src.split_to(end_index);
            src.advance(1); // skip null terminator
            Ok(Some(serde_json::from_slice(&buf)?))
        } else {
            Ok(None)
        }
    }
}
