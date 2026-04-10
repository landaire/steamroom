use crate::error::CryptoError;
use rsa::{Pkcs1v15Encrypt, RsaPublicKey, pkcs8::DecodePublicKey};

// Steam's universe-public RSA key for session encryption.
// This is the well-known public key used by every Steam client.
const STEAM_PUBLIC_KEY_PEM: &str = "-----BEGIN PUBLIC KEY-----
MIGdMA0GCSqGSIb3DQEBAQUAA4GLADCBhwKBgQDf7BrWLBBkvYtNsNPHplFLSuI4
PBnFdVgiyE0GJEEOVNg9mOQ4gBMfFmbIhqKS1aE5TACEmKTTQ7Gzf2JRAV8KYXHP
SmVbCnFttS7M0JWxN6Lhret2CTNBCyRVDAlX1UPOjfBoBOR4jsMUCRNOTW1nP63g
U7Jds6TjBfQPhUkHOQIBAw==
-----END PUBLIC KEY-----";

pub fn encrypt_with_steam_public_key(data: &[u8]) -> Result<Vec<u8>, CryptoError> {
    let public_key = RsaPublicKey::from_public_key_pem(STEAM_PUBLIC_KEY_PEM)
        .map_err(|e| CryptoError::Rsa(e.to_string()))?;

    let mut rng = rand::thread_rng();
    public_key
        .encrypt(&mut rng, Pkcs1v15Encrypt, data)
        .map_err(|e| CryptoError::Rsa(e.to_string()))
}

pub fn encrypt_with_rsa_public_key(
    data: &[u8],
    modulus_hex: &str,
    exponent_hex: &str,
) -> Result<Vec<u8>, CryptoError> {
    use rsa::BigUint;

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
