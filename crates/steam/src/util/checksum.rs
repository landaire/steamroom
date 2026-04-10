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
        todo!()
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
