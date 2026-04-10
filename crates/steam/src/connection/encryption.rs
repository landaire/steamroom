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
        // Generate random IV
        let mut iv = [0u8; 16];
        getrandom::getrandom(&mut iv).expect("failed to generate random IV");

        let encrypted = crypto::symmetric_encrypt_cbc(plaintext, &self.session_key, &iv)
            .expect("encryption should not fail with valid key");

        // HMAC-SHA1 of the IV + ciphertext, using the session key
        let mut mac = HmacSha1::new_from_slice(&self.session_key)
            .expect("HMAC key length should be valid");
        mac.update(&iv);
        mac.update(&encrypted);
        let hmac_result = mac.finalize().into_bytes();

        // Output: IV (16) + ciphertext + HMAC (20)
        let mut output = Vec::with_capacity(16 + encrypted.len() + 20);
        output.extend_from_slice(&iv);
        output.extend_from_slice(&encrypted);
        output.extend_from_slice(&hmac_result);
        output
    }

    pub fn decrypt(&self, data: &[u8]) -> Result<Vec<u8>, CryptoError> {
        // Input: IV (16) + ciphertext + HMAC (20)
        if data.len() < 36 {
            return Err(CryptoError::DecryptionFailed);
        }

        let iv = &data[..16];
        let hmac_offset = data.len() - 20;
        let encrypted = &data[16..hmac_offset];
        let received_hmac = &data[hmac_offset..];

        // Verify HMAC
        let mut mac = HmacSha1::new_from_slice(&self.session_key)
            .map_err(|_| CryptoError::InvalidKeyLength(self.session_key.len()))?;
        mac.update(iv);
        mac.update(encrypted);
        mac.verify_slice(received_hmac)
            .map_err(|_| CryptoError::DecryptionFailed)?;

        crypto::symmetric_decrypt_cbc(encrypted, &self.session_key, iv)
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
