use crate::error::CryptoError;

pub fn encrypt_with_steam_public_key(
    data: &[u8],
    modulus: &[u8],
    exponent: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    todo!()
}
