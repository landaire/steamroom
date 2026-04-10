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
        let header = [0u8; 3];

        // HMAC-SHA1(session_key[0..16], header + plaintext)
        let mut mac =
            HmacSha1::new_from_slice(&self.session_key[..16]).expect("valid HMAC key length");
        mac.update(&header);
        mac.update(plaintext);
        let hmac_result = mac.finalize().into_bytes();

        // Build IV: hmac[0..13] + header[0..3] = 16 bytes
        let mut iv = [0u8; 16];
        iv[..13].copy_from_slice(&hmac_result[..13]);
        iv[13..16].copy_from_slice(&header);

        // AES-256-CBC encrypt the plaintext with this IV
        let ciphertext =
            crypto::symmetric_encrypt_cbc(plaintext, &self.session_key, &iv)
                .expect("AES encrypt failed with valid key");

        // Wire: IV (16) + ciphertext
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

        // AES-256-CBC decrypt
        let decrypted = crypto::symmetric_decrypt_cbc(ciphertext, &self.session_key, iv)?;

        // The decrypted data is the plaintext (header bytes were in the IV, not in ciphertext).
        // Verify HMAC: HMAC-SHA1(session_key[0..16], iv[13..16] + plaintext) matches iv[0..13]
        let mut mac =
            HmacSha1::new_from_slice(&self.session_key[..16]).map_err(|_| CryptoError::DecryptionFailed)?;
        mac.update(&iv[13..16]); // 3-byte header
        mac.update(&decrypted);
        let expected = mac.finalize().into_bytes();

        // Constant-time compare first 13 bytes
        if expected[..13] != iv[..13] {
            return Err(CryptoError::DecryptionFailed);
        }

        Ok(decrypted)
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

    #[test]
    fn encrypted_has_iv_prefix() {
        let key = [0x42u8; 32];
        let cipher = SessionCipher::new(key);
        let encrypted = cipher.encrypt(b"test");
        // 16 bytes IV + 16 bytes ciphertext (4 bytes padded to 16 with PKCS7)
        assert_eq!(encrypted.len(), 32);
    }

    #[test]
    fn tampered_data_fails() {
        let key = [0x42u8; 32];
        let cipher = SessionCipher::new(key);
        let mut encrypted = cipher.encrypt(b"test");
        encrypted[0] ^= 0xff; // tamper with HMAC in IV
        assert!(cipher.decrypt(&encrypted).is_err());
    }
}
