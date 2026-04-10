# Reverse Engineering Notes

Notes from analyzing `steamclient64.dll` via Binary Ninja MCP.

## Session Cipher (CNetFilterEncryption)

Source: `netfilter.cpp` in steamclient64.dll

### Wire Format

```
[IV: 16 bytes] [AES-256-CBC ciphertext]
```

### Encrypt (CWorkItemNetFilterEncrypt::ThreadProcess)

1. Compute `hmac = HMAC-SHA1(session_key[0..16], header_3bytes + plaintext)`
2. Build `IV = hmac[0..13] || header_3bytes` (16 bytes total)
3. AES-256-CBC encrypt `plaintext` using `IV` and full 32-byte `session_key`, PKCS7 padding
4. Output: `IV || ciphertext`

The 3-byte header is stored at IV[13..16]. In practice these are zero bytes.

### Decrypt (CWorkItemNetFilterDecrypt, sub_138f2bd70)

1. Split input: `IV = data[0..16]`, `ciphertext = data[16..]`
2. AES-256-CBC decrypt `ciphertext` using `IV` and full 32-byte key
3. Verify: `HMAC-SHA1(key[0..16], IV[13..16] + plaintext)[0..13] == IV[0..13]`
4. Return `plaintext`

### Key Details

- HMAC key is first 16 bytes of the 32-byte session key
- AES key is the full 32-byte session key
- HMAC verification uses constant-time comparison (bitwise OR reduction)
- 13 bytes of HMAC + 3 bytes header = 16 byte IV

## RSA Encryption (CCrypto::RSAEncrypt)

Source: `crypto_rsa_openssl.cpp` in steamclient64.dll

- Uses OpenSSL `RSA_PKCS1_OAEP_PADDING` (value 4) = OAEP with SHA-1
- Public key for universe "Public" stored as hex-encoded DER at `0x1394161d0`
- Key is 1024-bit RSA, exponent 0x11 (17)
- Encrypts `session_key(32) + nonce(16)` = 48 bytes

### DER Key (Universe Public)

```
30819d300d06092a864886f70d010101050003818b00308187
02818100
dfec1ad62c10662c17353a14b07c59117f9dd3d82b7ae3e015
cd191e46e87b8774a2184631a9031479828ee945a24912a923
687389cf69a1b16146bdc1bebfd6011bd881d4dc90fbfe4f52
7366cb9570d7c58eba1c7a3375a1623446bb60b78068fa13a7
7a8a374b9ec6f45d5f3a99f99ec43ae963a2bb881928e0e714
c0428902
0111
```

## AES-CBC (CCrypto::SymmetricEncrypt)

Source: `crypto_aescbc_openssl.cpp`

- Standard AES-CBC with PKCS7 padding
- Supports 128-bit and 256-bit keys
- Uses AES-NI when available
- Two wrappers: `sub_138cec7a0` (random IV) and `sub_138cec830` (caller-provided IV)

## CM Server Protocol

### Connection Flow

1. TCP connect to CM server (port 27017)
2. Server sends `ChannelEncryptRequest` (EMsg 1303) with protocol_version, universe, 16-byte nonce
3. Client RSA-OAEP encrypts `session_key + nonce`, sends `ChannelEncryptResponse` (EMsg 1304)
4. Server sends `ChannelEncryptResult` (EMsg 1305) with eresult
5. All subsequent messages are encrypted with the session cipher

### Framing

All TCP messages use VT01 framing:
```
[payload_length: u32 LE] [magic: "VT01" = 0x56543031] [payload]
```

### Message Format (Protobuf)

```
[raw_emsg: u32 LE (emsg | 0x80000000)] [header_len: u32 LE] [CMsgProtoBufHeader] [body]
```

## Proto Extraction

`steamclient64.dll` embeds serialized `FileDescriptorProto` blobs in `.rdata`.
Registration entries in `.data` contain `(size: u32, ptr: u64)` pairs pointing to blobs.
Scanner finds blobs by matching `0x0a` + varint + `.proto` suffix pattern.
106 proto files extracted successfully.
