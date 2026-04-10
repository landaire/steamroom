use crate::error::CryptoError;
use rsa::{Pkcs1v15Encrypt, RsaPublicKey};
use rsa::BigUint;

pub fn encrypt_with_rsa_public_key(
    data: &[u8],
    modulus_hex: &str,
    exponent_hex: &str,
) -> Result<Vec<u8>, CryptoError> {
    let modulus = BigUint::parse_bytes(modulus_hex.as_bytes(), 16)
        .ok_or_else(|| CryptoError::Rsa("invalid modulus hex".into()))?;
    let exponent = BigUint::parse_bytes(exponent_hex.as_bytes(), 16)
        .ok_or_else(|| CryptoError::Rsa("invalid exponent hex".into()))?;

    let public_key = RsaPublicKey::new(modulus, exponent)
        .map_err(|e| CryptoError::Rsa(e.to_string()))?;

    let mut rng = rand::thread_rng();
    public_key
        .encrypt(&mut rng, Pkcs1v15Encrypt, data)
        .map_err(|e| CryptoError::Rsa(e.to_string()))
}
