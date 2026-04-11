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
        // Steam uses a non-standard Adler-32 with zero seed (a=0, b=0) instead
        // of the standard (a=1, b=0). The relationship after n bytes is:
        //   a_steam = a_std - 1
        //   b_steam = b_std - n
        // (all mod 65521). This lets us use the `adler` crate's SIMD-optimized
        // implementation and adjust the result.
        const MOD: u32 = 65521;
        let std_checksum = adler::adler32_slice(data);
        let a_std = std_checksum & 0xFFFF;
        let b_std = std_checksum >> 16;
        let n_mod = (data.len() as u32) % MOD;
        let a_steam = (a_std + MOD - 1) % MOD;
        let b_steam = (b_std + MOD - n_mod) % MOD;
        Self((b_steam << 16) | a_steam)
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
    fn steam_adler32_known_value() {
        // Hand-computed: zero-seed Adler32 of "hello"
        assert_eq!(SteamAdler32::compute(b"hello").0, 0x0627_0214);
        assert_eq!(SteamAdler32::compute(b"").0, 0x0000_0000);
    }

    #[test]
    fn steam_adler32_matches_naive() {
        fn naive_steam_adler32(data: &[u8]) -> u32 {
            let mut a: u32 = 0;
            let mut b: u32 = 0;
            for &byte in data {
                a = (a + byte as u32) % 65521;
                b = (b + a) % 65521;
            }
            (b << 16) | a
        }

        // Small inputs
        for input in [b"" as &[u8], b"a", b"hello", b"test chunk data!"] {
            assert_eq!(
                SteamAdler32::compute(input).0,
                naive_steam_adler32(input),
                "mismatch for {:?}",
                input,
            );
        }

        // Large input (1 MB)
        let big: Vec<u8> = (0..1_048_576u32).map(|i| (i % 251) as u8).collect();
        assert_eq!(SteamAdler32::compute(&big).0, naive_steam_adler32(&big));
    }

    #[test]
    fn sha1_known_value() {
        let data = b"hello";
        let hash = Sha1Hash::compute(data);
        let expected: [u8; 20] = [
            0xaa, 0xf4, 0xc6, 0x1d, 0xdc, 0xc5, 0xe8, 0xa2, 0xda, 0xbe, 0xde, 0x0f, 0x3b, 0x48,
            0x2c, 0xd9, 0xae, 0xa9, 0x43, 0x4d,
        ];
        assert_eq!(hash.0, expected);
    }
}
