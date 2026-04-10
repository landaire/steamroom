pub mod rsa;

use crate::error::CryptoError;

pub fn symmetric_decrypt_cbc(data: &[u8], key: &[u8], iv: &[u8]) -> Result<Vec<u8>, CryptoError> {
    todo!()
}

pub fn symmetric_decrypt_ecb(data: &[u8], key: &[u8]) -> Result<Vec<u8>, CryptoError> {
    todo!()
}

pub fn symmetric_encrypt_cbc(data: &[u8], key: &[u8], iv: &[u8]) -> Result<Vec<u8>, CryptoError> {
    todo!()
}
