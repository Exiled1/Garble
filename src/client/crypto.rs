use openssl::aes::AesKey;
use openssl::error::ErrorStack;
use openssl::hash::MessageDigest;
use openssl::pkey::{PKey, PKeyRef, Public};
use openssl::rand::rand_bytes;
use openssl::rsa::Padding;
use openssl::sign::{Signer, Verifier};

// Choose a key, & given a key encrypt & decrypt messages.
use super::ClientError;
use super::{Private, Rsa};
use openssl::symm::{decrypt_aead, encrypt_aead, Cipher};

pub struct SessionKey {
    // Rsa to negotatiate aes key, aes key is session key.
    // When session key is made,
    // Given a decrypted message, use the session key to encrypt.
    // Given an encrypted message, use a session key to decrypt.
    session_key: Vec<u8>,
}

impl SessionKey {
    fn new(given_key: Option<Vec<u8>>) -> Result<Self, ErrorStack> {
        match given_key {
            None => {
                let mut session_key = [0; 32]; // 32 * 8 = 256
                rand_bytes(&mut session_key)?;

                Ok(SessionKey {
                    session_key: session_key.to_vec(),
                })
            }
            Some(session_key) => Ok(SessionKey { session_key }),
        }
    }

    /// Use aes 256 to encrypt
    pub fn encrypt_session_key(
        self,
        pub_key: Rsa<Public>,
        sign_key: PKey<Private>,
    ) -> Result<(Vec<u8>, Vec<u8>), ErrorStack> {
        // let mut iv = [0; 256];
        // let cipher = Cipher::aes_256_gcm();
        // temporary key is the session key.
        let mut enc_key = vec![0 as u8; sign_key.size()];
        // Pub key defaults to 2048, so this should be 256, but can be set. If padding fails, remove that option. Also, I think signkey.size is okay to use here to decide the size.
        let bytes = pub_key.public_encrypt(&self.session_key, &mut enc_key, Padding::PKCS1)?;
        // Please don't fail me ;-;
        let mut signer = Signer::new(MessageDigest::sha3_256(), &sign_key)?;
        signer.update(&enc_key);
        let signature = signer.sign_to_vec()?;

        Ok((enc_key.to_vec(), signature))
    }

    /// The opposite of encyrpt.
    pub fn decrypt_session_key(
        enc_session_key: Vec<u8>,
        signature: Vec<u8>,
        dec_key: Rsa<Private>,
        verifier_key: PKey<Public>,
    ) -> Result<(Self, Vec<u8>), ClientError> {
        // Decrypt with our private key, validate with their public key.
        let mut dec_session_key: Vec<u8> = vec![0; dec_key.size() as usize];
        dec_key.private_decrypt(&enc_session_key, &mut dec_session_key, Padding::PKCS1)?;

        // Verifies using their key.
        let mut verifier = Verifier::new(MessageDigest::sha3_256(), &verifier_key)?;
        verifier.update(&signature);
        match verifier.verify(&signature)? {
            true => {
                return Ok((SessionKey::new(Some(dec_session_key))?, signature));
            }
            false => {
                return Err(ClientError::InvalidSignature);
            }
        }
    }

    pub fn session_encrypt_message(
        self,
        message: String,
    ) -> Result<(Vec<u8>, Vec<u8>, Vec<u8>), ClientError> {
        let cipher = Cipher::aes_256_gcm();
        let mut iv = vec![0; 12]; // 12 * 8 = 96; 128 bit block, with 32 bit counter = 128
        rand_bytes(&mut iv)?; // Randomize the bits in the iv for a unique iv of 96 bits.
        let aad = [];
        let mut tag = vec![0; 16]; // Recommended max for tag size in AES GCM is 16 bytes.
        let encrypted_message = encrypt_aead(
            cipher,
            &self.session_key,
            Some(&iv),
            &aad,
            message.as_bytes(),
            &mut tag,
        )?;
        Ok((encrypted_message, tag, iv))
    }

    pub fn session_decrypt_message(
        self,
        enc_message: Vec<u8>,
        tag: Vec<u8>,
        iv: Vec<u8>,
    ) -> Result<String, ClientError> {
        let cipher = Cipher::aes_256_gcm();
        //let mut iv = vec![0; 12]; // 12 * 8 = 96; 128 bit block, with 32 bit counter = 128
        //rand_bytes(&mut iv)?; // Randomize the bits in the iv for a unique iv of 96 bits.
        let aad = [];
        //let mut tag = vec![0; 16]; // Recommended max for tag size in AES GCM is 16 bytes.
        let unencrypted_msg = decrypt_aead(
            cipher,
            &self.session_key,
            Some(&iv),
            &aad,
            &enc_message,
            &tag,
        )?;
        
        Ok(String::from_utf8(unencrypted_msg)?)
    }
}
