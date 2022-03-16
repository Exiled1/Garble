use bytes::Buf;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use tokio_util::codec;

#[derive(Serialize, Deserialize, Debug)]
pub enum Clientbound {
    Message{message: String},
    Shutdown { message: String },
}

#[derive(Serialize, Deserialize, Debug)]
pub enum Serverbound {
    Hello { public_key: Vec<u8> },
    Message{ message: String },
    
}

pub struct MessageCodec<Tx: Serialize, Rx: DeserializeOwned>(
    std::marker::PhantomData<Tx>,
    std::marker::PhantomData<Rx>,
);

impl <Tx: Serialize, Rx: DeserializeOwned> Default for MessageCodec<Tx, Rx> {
    fn default() -> Self {
        Self(Default::default(), Default::default())
    }
}

impl <Tx: Serialize, Rx: DeserializeOwned> codec::Encoder<Tx> for MessageCodec<Tx, Rx> {
    type Error = std::io::Error;

    fn encode(&mut self, item: Tx, dst: &mut bytes::BytesMut) -> Result<(), Self::Error> {
        let encoded = serde_json::to_vec(&item)?;
        dst.reserve(encoded.len() + 1);
        dst.extend_from_slice(&encoded);
        dst.extend_from_slice(&[0]); // null terminator
        Ok(())
    }
}
impl <Tx: Serialize, Rx: DeserializeOwned> codec::Decoder for MessageCodec<Tx, Rx> {
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
