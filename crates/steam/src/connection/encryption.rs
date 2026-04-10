use crate::error::CryptoError;

pub struct SessionCipher {
    session_key: [u8; 32],
}

impl SessionCipher {
    pub fn new(session_key: [u8; 32]) -> Self {
        Self { session_key }
    }

    pub fn encrypt(&self, plaintext: &[u8]) -> Vec<u8> {
        todo!()
    }

    pub fn decrypt(&self, data: &[u8]) -> Result<Vec<u8>, CryptoError> {
        todo!()
    }
}
