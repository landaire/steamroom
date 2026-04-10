pub mod rsa;

use aes::cipher::{BlockDecryptMut, BlockEncryptMut, KeyIvInit, KeyInit};
use crate::error::CryptoError;

type Aes256CbcEnc = cbc::Encryptor<aes::Aes256>;
type Aes256CbcDec = cbc::Decryptor<aes::Aes256>;
type Aes256EcbDec = ecb::Decryptor<aes::Aes256>;

pub fn symmetric_encrypt_cbc(data: &[u8], key: &[u8], iv: &[u8]) -> Result<Vec<u8>, CryptoError> {
    if key.len() != 32 {
        return Err(CryptoError::InvalidKeyLength(key.len()));
    }
    let cipher = Aes256CbcEnc::new_from_slices(key, iv)
        .map_err(|_| CryptoError::InvalidKeyLength(key.len()))?;
    Ok(cipher.encrypt_padded_vec_mut::<aes::cipher::block_padding::Pkcs7>(data))
}

pub fn symmetric_decrypt_cbc(data: &[u8], key: &[u8], iv: &[u8]) -> Result<Vec<u8>, CryptoError> {
    if key.len() != 32 {
        return Err(CryptoError::InvalidKeyLength(key.len()));
    }
    let cipher = Aes256CbcDec::new_from_slices(key, iv)
        .map_err(|_| CryptoError::InvalidKeyLength(key.len()))?;
    cipher
        .decrypt_padded_vec_mut::<aes::cipher::block_padding::Pkcs7>(data)
        .map_err(|_| CryptoError::InvalidPadding)
}

pub fn symmetric_decrypt_ecb(data: &[u8], key: &[u8]) -> Result<Vec<u8>, CryptoError> {
    if key.len() != 32 {
        return Err(CryptoError::InvalidKeyLength(key.len()));
    }
    let cipher = Aes256EcbDec::new_from_slice(key)
        .map_err(|_| CryptoError::InvalidKeyLength(key.len()))?;
    cipher
        .decrypt_padded_vec_mut::<aes::cipher::block_padding::Pkcs7>(data)
        .map_err(|_| CryptoError::InvalidPadding)
}
