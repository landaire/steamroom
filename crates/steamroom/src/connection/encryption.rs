use crate::crypto;
use crate::error::CryptoError;
use aes::cipher::BlockModeEncrypt;
use aes::cipher::KeyInit;
use hmac::Hmac;
use hmac::Mac;
use sha1::Sha1;

type HmacSha1 = Hmac<Sha1>;
type Aes256EcbEnc = ecb::Encryptor<aes::Aes256>;

/// Session cipher for Steam CM TCP connections.
///
/// Wire format: `ECB(IV) || CBC(plaintext)`
///
/// Encrypt:
/// 1. Derive IV = HMAC-SHA1(key[0..16], header_3 || plaintext)[0..13] || header_3
/// 2. AES-256-CBC encrypt plaintext with derived IV
/// 3. AES-256-ECB encrypt the IV itself
/// 4. Output: encrypted_IV(16) || ciphertext
///
/// Decrypt:
/// 1. AES-256-ECB decrypt first 16 bytes → IV
/// 2. AES-256-CBC decrypt remaining bytes with recovered IV
pub struct SessionCipher {
    session_key: [u8; 32],
}

impl SessionCipher {
    pub fn new(session_key: [u8; 32]) -> Self {
        Self { session_key }
    }

    pub fn encrypt(&self, plaintext: &[u8]) -> Vec<u8> {
        let header = [0u8; 3];

        // Derive IV: HMAC-SHA1(key[0..16], header || plaintext)[0..13] || header
        let mut mac =
            HmacSha1::new_from_slice(&self.session_key[..16]).expect("valid HMAC key length");
        mac.update(&header);
        mac.update(plaintext);
        let hmac_result = mac.finalize().into_bytes();
        let mut iv = [0u8; 16];
        iv[..13].copy_from_slice(&hmac_result[..13]);
        iv[13..16].copy_from_slice(&header);

        // CBC encrypt plaintext
        let ciphertext = crypto::symmetric_encrypt_cbc(plaintext, &self.session_key, &iv)
            .expect("AES encrypt with valid key");

        // ECB encrypt the IV
        let enc = Aes256EcbEnc::new_from_slice(&self.session_key).unwrap();
        let mut encrypted_iv = [0u8; 16];
        enc.encrypt_padded_b2b::<aes::cipher::block_padding::NoPadding>(&iv, &mut encrypted_iv)
            .unwrap();

        let mut output = Vec::with_capacity(16 + ciphertext.len());
        output.extend_from_slice(&encrypted_iv);
        output.extend_from_slice(&ciphertext);
        output
    }

    pub fn decrypt(&self, data: &[u8]) -> Result<Vec<u8>, CryptoError> {
        if data.len() < 32 {
            return Err(CryptoError::DecryptionFailed);
        }

        // ECB decrypt first 16 bytes to recover IV
        let iv = crypto::symmetric_decrypt_ecb_nopad(&data[..16], &self.session_key)?;

        // CBC decrypt remaining bytes
        crypto::symmetric_decrypt_cbc(&data[16..], &self.session_key, &iv)
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
    fn tampered_data_fails() {
        let key = [0x42u8; 32];
        let cipher = SessionCipher::new(key);
        let mut encrypted = cipher.encrypt(b"test");
        encrypted[0] ^= 0xff;
        assert!(cipher.decrypt(&encrypted).is_err());
    }
}
