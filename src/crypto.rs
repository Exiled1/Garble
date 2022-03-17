//! Implementations of cryptographic algorithms

use openssl::error::ErrorStack;
use openssl::hash::{Hasher, MessageDigest};
use openssl::pkey::{Private, Public};
use openssl::rand::rand_bytes;
use openssl::rsa::{Padding, Rsa, RsaRef};
use openssl::sign::{Signer, Verifier};
use openssl::{base64, pkey};

// Choose a key, & given a key encrypt & decrypt messages.
use crate::client::ClientError;
use crate::message::Encrypted;

// This generates a pub/priv keypair for making a server connection, generates a new keypair if there already exists one. Note: Should not be called outside of the server_connect function.
pub fn generate_keypair(keysize: Option<u32>) -> Result<Rsa<Private>, ClientError> {
    let keypair = Rsa::generate(keysize.unwrap_or(2048))?;
    Ok(keypair)
}

/// Calculates the base64'd SHA3-256 hash of a buffer.
pub fn sha3_256_base64(buf: &[u8]) -> String {
    let mut hasher = Hasher::new(MessageDigest::sha3_256()).expect("OpenSSL error");
    hasher.update(buf).expect("OpenSSL error");
    let hash = hasher.finish().expect("OpenSSL error");
    base64::encode_block(&hash)
}

// Simply hashes the public key and returns the base64 string result of it. This is for identifying a user while giving no information to the server.
pub fn hash_pub_key<T: pkey::HasPublic>(keypair: &Rsa<T>) -> String {
    sha3_256_base64(&keypair.public_key_to_der().expect("OpenSSL error"))
}

use openssl::symm::{decrypt_aead, encrypt_aead, Cipher};

pub struct SessionKey {
    // Rsa to negotatiate aes key, aes key is session key.
    // When session key is made,
    // Given a decrypted message, use the session key to encrypt.
    // Given an encrypted message, use a session key to decrypt.
    session_key: Vec<u8>,
}

impl SessionKey {
    /// Randomly generates a session key.
    pub fn generate() -> Result<Self, ErrorStack> {
        let mut session_key = [0; 32]; // 32 * 8 = 256
        rand_bytes(&mut session_key)?;

        Ok(SessionKey {
            session_key: session_key.to_vec(),
        })
    }

    /// Use RSA to encrypt & sign the session key.
    pub fn encrypt_session_key(
        &self,
        pub_key: &RsaRef<Public>,
        sign_key: &RsaRef<Private>,
    ) -> Result<(Vec<u8>, Vec<u8>), ErrorStack> {
        // let mut iv = [0; 256];
        // let cipher = Cipher::aes_256_gcm();
        // temporary key is the session key.
        let mut enc_key = vec![0u8; sign_key.size() as usize];
        // Pub key defaults to 2048, so this should be 256, but can be set. If padding fails, remove that option. Also, I think signkey.size is okay to use here to decide the size.
        pub_key.public_encrypt(&self.session_key, &mut enc_key, Padding::PKCS1)?;
        // Please don't fail me ;-;
        let sign_key = openssl::pkey::PKey::from_rsa(sign_key.to_owned())?;
        let mut signer = Signer::new(MessageDigest::sha3_256(), sign_key.as_ref())?;
        signer.update(&enc_key)?;
        let signature = signer.sign_to_vec()?;

        Ok((enc_key.to_vec(), signature))
    }

    /// The opposite of encrypt.
    pub fn decrypt_session_key(
        enc_session_key: Vec<u8>,
        signature: Vec<u8>,
        dec_key: &RsaRef<Private>,
        verifier_key: &RsaRef<Public>,
    ) -> Result<Self, ClientError> {
        // Verify the encrypted session key using their public key.
        let verifier_key = openssl::pkey::PKey::from_rsa(verifier_key.to_owned())?;
        let mut verifier = Verifier::new(MessageDigest::sha3_256(), verifier_key.as_ref())?;
        verifier.update(&enc_session_key)?;

        if verifier.verify(&signature)? {
            // Decrypt with our private key.
            let mut dec_session_key: Vec<u8> = vec![0; dec_key.size() as usize];
            let length =
                dec_key.private_decrypt(&enc_session_key, &mut dec_session_key, Padding::PKCS1)?;
            dec_session_key.truncate(length);
            Ok(SessionKey {
                session_key: dec_session_key,
            })
        } else {
            Err(ClientError::InvalidSignature)
        }
    }

    /// Uses AES-256-GCM to encrypt a message.
    pub fn session_encrypt_message(
        &self,
        message: String,
    ) -> Result<crate::message::Encrypted, ClientError> {
        let cipher = Cipher::aes_256_gcm();
        let mut iv = vec![0; 12]; // 12 * 8 = 96; 128 bit block, with 32 bit counter = 128
        rand_bytes(&mut iv)?; // Randomize the bits in the iv for a unique iv of 96 bits.
        let aad = [];
        let mut tag = vec![0; 16]; // Recommended max for tag size in AES GCM is 16 bytes.
        let ciphertext = encrypt_aead(
            cipher,
            &self.session_key,
            Some(&iv),
            &aad,
            message.as_bytes(),
            &mut tag,
        )?;
        Ok(Encrypted {
            iv,
            ciphertext,
            tag,
        })
    }

    /// Uses AES-256-GCM to encrypt a message.
    pub fn session_decrypt_message(&self, message: Encrypted) -> Result<String, ClientError> {
        let cipher = Cipher::aes_256_gcm();
        //let mut iv = vec![0; 12]; // 12 * 8 = 96; 128 bit block, with 32 bit counter = 128
        //rand_bytes(&mut iv)?; // Randomize the bits in the iv for a unique iv of 96 bits.
        let aad = [];
        //let mut tag = vec![0; 16]; // Recommended max for tag size in AES GCM is 16 bytes.
        let unencrypted_msg = decrypt_aead(
            cipher,
            &self.session_key,
            Some(&message.iv),
            &aad,
            &message.ciphertext,
            &message.tag,
        )?;

        Ok(String::from_utf8(unencrypted_msg)?)
    }
}
