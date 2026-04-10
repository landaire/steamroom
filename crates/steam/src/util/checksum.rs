#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct Adler32(pub u32);

impl Adler32 {
    pub fn compute(data: &[u8]) -> Self {
        Self(adler::adler32_slice(data))
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct SteamAdler32(pub u32);

impl SteamAdler32 {
    pub fn compute(data: &[u8]) -> Self {
        // Steam uses a non-standard Adler-32 with zero seed instead of 1
        let mut a: u32 = 0;
        let mut b: u32 = 0;
        for &byte in data {
            a = (a + byte as u32) % 65521;
            b = (b + a) % 65521;
        }
        Self((b << 16) | a)
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct Crc32(pub u32);

impl Crc32 {
    pub fn compute(data: &[u8]) -> Self {
        Self(crc32fast::hash(data))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Sha1Hash(pub [u8; 20]);

impl Sha1Hash {
    pub fn compute(data: &[u8]) -> Self {
        use sha1::Digest;
        let hash = sha1::Sha1::digest(data);
        Self(hash.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_known_value() {
        let data = b"hello";
        let crc = Crc32::compute(data);
        assert_eq!(crc.0, 0x3610a686);
    }

    #[test]
    fn sha1_known_value() {
        let data = b"hello";
        let hash = Sha1Hash::compute(data);
        let expected: [u8; 20] = [
            0xaa, 0xf4, 0xc6, 0x1d, 0xdc, 0xc5, 0xe8, 0xa2, 0xda, 0xbe,
            0xde, 0x0f, 0x3b, 0x48, 0x2c, 0xd9, 0xae, 0xa9, 0x43, 0x4d,
        ];
        assert_eq!(hash.0, expected);
    }
}
