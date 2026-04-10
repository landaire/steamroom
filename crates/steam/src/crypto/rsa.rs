use crate::error::CryptoError;
use rsa::{Pkcs1v15Encrypt, Oaep, RsaPublicKey, pkcs8::DecodePublicKey};

// Steam's universe-public RSA key for session encryption (DER hex from steamclient64.dll).
const STEAM_PUBLIC_KEY_HEX: &str = "30819d300d06092a864886f70d010101050003818b0030818702818100dfec1ad62c10662c17353a14b07c59117f9dd3d82b7ae3e015cd191e46e87b8774a2184631a9031479828ee945a24912a923687389cf69a1b16146bdc1bebfd6011bd881d4dc90fbfe4f527366cb9570d7c58eba1c7a3375a1623446bb60b78068fa13a77a8a374b9ec6f45d5f3a99f99ec43ae963a2bb881928e0e714c042890201 11";

fn decode_hex(hex: &str) -> Vec<u8> {
    let hex: String = hex.chars().filter(|c| !c.is_whitespace()).collect();
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect()
}

pub fn encrypt_with_steam_public_key(data: &[u8]) -> Result<Vec<u8>, CryptoError> {
    let der = decode_hex(STEAM_PUBLIC_KEY_HEX);
    let public_key = RsaPublicKey::from_public_key_der(&der)
        .map_err(|e| CryptoError::Rsa(e.to_string()))?;

    let mut rng = rand::thread_rng();
    let padding = Oaep::new::<sha1::Sha1>();
    public_key
        .encrypt(&mut rng, padding, data)
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
