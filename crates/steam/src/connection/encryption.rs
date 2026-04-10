use hmac::{Hmac, Mac};
use sha1::Sha1;
use crate::crypto;
use crate::error::CryptoError;

type HmacSha1 = Hmac<Sha1>;

pub struct SessionCipher {
    session_key: [u8; 32],
}

impl SessionCipher {
    pub fn new(session_key: [u8; 32]) -> Self {
        Self { session_key }
    }

    pub fn encrypt(&self, plaintext: &[u8]) -> Vec<u8> {
        // Generate random 16-byte IV
        let mut iv = [0u8; 16];
        getrandom::getrandom(&mut iv).expect("RNG failed");

        // AES-256-CBC encrypt with PKCS7 padding
        let ciphertext = crypto::symmetric_encrypt_cbc(plaintext, &self.session_key, &iv)
            .expect("AES encrypt with valid key");

        // Wire: [IV 16 bytes] [ciphertext]
        let mut output = Vec::with_capacity(16 + ciphertext.len());
        output.extend_from_slice(&iv);
        output.extend_from_slice(&ciphertext);
        output
    }

    pub fn decrypt(&self, data: &[u8]) -> Result<Vec<u8>, CryptoError> {
        if data.len() < 32 {
            return Err(CryptoError::DecryptionFailed);
        }

        let iv = &data[..16];
        let ciphertext = &data[16..];
        crypto::symmetric_decrypt_cbc(ciphertext, &self.session_key, iv)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let key = [0x42u8; 32];
        let cipher = SessionCipher::new(key);
        let plaintext = b"hello, steam!";
        let encrypted = cipher.encrypt(plaintext);
        let decrypted = cipher.decrypt(&encrypted).unwrap();
        assert_eq!(&decrypted, plaintext);
    }
}
