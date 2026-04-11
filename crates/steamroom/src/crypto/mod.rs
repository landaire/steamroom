pub mod rsa;

use crate::error::CryptoError;
use aes::cipher::BlockModeDecrypt;
use aes::cipher::BlockModeEncrypt;
use aes::cipher::KeyInit;
use aes::cipher::KeyIvInit;

type Aes256CbcEnc = cbc::Encryptor<aes::Aes256>;
type Aes256CbcDec = cbc::Decryptor<aes::Aes256>;
type Aes256EcbDec = ecb::Decryptor<aes::Aes256>;

pub fn symmetric_encrypt_cbc(data: &[u8], key: &[u8], iv: &[u8]) -> Result<Vec<u8>, CryptoError> {
    if key.len() != 32 {
        return Err(CryptoError::InvalidKeyLength(key.len()));
    }
    let cipher = Aes256CbcEnc::new_from_slices(key, iv)
        .map_err(|_| CryptoError::InvalidKeyLength(key.len()))?;
    Ok(cipher.encrypt_padded_vec::<aes::cipher::block_padding::Pkcs7>(data))
}

pub fn symmetric_decrypt_cbc(data: &[u8], key: &[u8], iv: &[u8]) -> Result<Vec<u8>, CryptoError> {
    if key.len() != 32 {
        return Err(CryptoError::InvalidKeyLength(key.len()));
    }
    let cipher = Aes256CbcDec::new_from_slices(key, iv)
        .map_err(|_| CryptoError::InvalidKeyLength(key.len()))?;
    cipher
        .decrypt_padded_vec::<aes::cipher::block_padding::Pkcs7>(data)
        .map_err(|_| CryptoError::InvalidPadding)
}

pub fn symmetric_decrypt_ecb_nopad(data: &[u8], key: &[u8]) -> Result<Vec<u8>, CryptoError> {
    if key.len() != 32 {
        return Err(CryptoError::InvalidKeyLength(key.len()));
    }
    let cipher =
        Aes256EcbDec::new_from_slice(key).map_err(|_| CryptoError::InvalidKeyLength(key.len()))?;
    cipher
        .decrypt_padded_vec::<aes::cipher::block_padding::NoPadding>(data)
        .map_err(|_| CryptoError::InvalidPadding)
}

pub fn symmetric_decrypt_ecb(data: &[u8], key: &[u8]) -> Result<Vec<u8>, CryptoError> {
    if key.len() != 32 {
        return Err(CryptoError::InvalidKeyLength(key.len()));
    }
    let cipher =
        Aes256EcbDec::new_from_slice(key).map_err(|_| CryptoError::InvalidKeyLength(key.len()))?;
    cipher
        .decrypt_padded_vec::<aes::cipher::block_padding::Pkcs7>(data)
        .map_err(|_| CryptoError::InvalidPadding)
}

type Aes256EcbEnc = ecb::Encryptor<aes::Aes256>;

pub fn symmetric_encrypt_ecb_nopad(data: &[u8], key: &[u8]) -> Result<Vec<u8>, CryptoError> {
    if key.len() != 32 {
        return Err(CryptoError::InvalidKeyLength(key.len()));
    }
    let cipher =
        Aes256EcbEnc::new_from_slice(key).map_err(|_| CryptoError::InvalidKeyLength(key.len()))?;
    Ok(cipher.encrypt_padded_vec::<aes::cipher::block_padding::NoPadding>(data))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aes_cbc_known_vector() {
        // Test vector generated with Python cryptography library
        let key: Vec<u8> = (0..32).collect();
        let iv: Vec<u8> = (0..16).collect();
        let plaintext = b"Hello Steam CM!";
        let expected_ct: [u8; 16] = [
            0x08, 0xab, 0x41, 0x23, 0x8a, 0xf7, 0x79, 0x4f, 0x21, 0xe7, 0x88, 0xd6, 0xe3, 0x03,
            0xbe, 0x06,
        ];

        let ct = symmetric_encrypt_cbc(plaintext, &key, &iv).unwrap();
        assert_eq!(&ct, &expected_ct, "AES-256-CBC encrypt mismatch");

        let pt = symmetric_decrypt_cbc(&ct, &key, &iv).unwrap();
        assert_eq!(&pt, plaintext, "AES-256-CBC decrypt mismatch");
    }
}
