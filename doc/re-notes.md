# Reverse Engineering Notes

Notes from analyzing `steamclient64.dll` via Binary Ninja MCP.

## Session Cipher (CNetFilterEncryption)

Source: `netfilter.cpp` in steamclient64.dll

### Wire Format

```
[AES-256-ECB encrypted IV: 16 bytes] [AES-256-CBC ciphertext]
```

This is the same `ECB(IV) + CBC(data)` format used by `CCrypto::SymmetricEncrypt` /
`CCrypto::SymmetricDecrypt` throughout the Steam client (including depot chunk encryption).

### Encrypt (CWorkItemNetFilterEncrypt::ThreadProcess)

1. Compute `hmac = HMAC-SHA1(session_key[0..16], header_3bytes + plaintext)`
2. Build `IV = hmac[0..13] || header_3bytes` (16 bytes total)
3. AES-256-CBC encrypt `plaintext` using `IV` and full 32-byte `session_key`, PKCS7
4. AES-256-ECB encrypt `IV` using the full 32-byte key
5. Output: `ECB(IV) || CBC(plaintext)`

The 3-byte header is a sequence counter stored at IV[13..16]. In practice these are
zero bytes for the initial messages.

The HMAC derivation is deterministic, but the IV is still ECB-encrypted on the wire.
This was the key insight that was missing — earlier attempts sent the raw HMAC-derived
IV without ECB encryption, which the server silently dropped.

### Decrypt (CWorkItemNetFilterDecrypt)

1. AES-256-ECB decrypt first 16 bytes → recover `IV`
2. AES-256-CBC decrypt remaining bytes using recovered `IV` and full 32-byte key
3. Optionally verify: `HMAC-SHA1(key[0..16], IV[13..16] + plaintext)[0..13] == IV[0..13]`
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

## AES Symmetric Crypto (CCrypto::SymmetricEncrypt / SymmetricDecrypt)

Source: `crypto_aescbc_openssl.cpp`

All symmetric encryption in the Steam client uses the same format:

```
[AES-ECB(IV): 16 bytes] [AES-CBC(plaintext, IV, key): N bytes, PKCS7 padded]
```

**Encrypt** (`CCrypto::SymmetricEncrypt`):
1. Generate or derive 16-byte IV
2. AES-CBC encrypt plaintext with PKCS7 padding using IV and key
3. AES-ECB encrypt the IV itself using the same key
4. Output: `ECB(IV) || CBC(plaintext)`

**Decrypt** (`CCrypto::SymmetricDecrypt`):
1. AES-ECB decrypt first 16 bytes → recover IV
2. AES-CBC decrypt remaining bytes using recovered IV and key, remove PKCS7

## Download Pipeline (CCSInterface / CDepotReconstruct)

Source: `csinterface.cpp`, `depotreconstruct.cpp`

### Architecture

The Steam client uses a multi-stage pipeline:

1. **CCSInterface** — manages HTTP chunk requests to CDN servers
2. **CDepotReconstruct** — orchestrates the full download: manifest parsing,
   chunk scheduling, decrypt, decompress, file assembly

### Concurrency Model

From `CCSInterface::YieldingDownloadChunks` (sub_138661a20):

- **Batching**: chunks are submitted in a tight loop, not one at a time
- **Hard cap**: 1024 (`0x400`) total outstanding requests
- **Bandwidth-adaptive throttle**: outstanding bytes limited to `bandwidth * 6.0`
  (6 seconds of buffered data based on measured download rate)
- **Minimum concurrency**: 2 requests (normal) or 8 (for certain server types)
- Tracked via `m_unChunkRequestsOutstanding` (offset 0x298) and
  `m_cubChunksOutstanding` (offset 0x288)

### Thread Pools

- IO thread pool: configurable via `DepotReconstructionNumIOThreads` convar (default "32")
- Separate decrypt thread pool for CPU-bound AES/decompress work
- `CWorkItemReadFromChunkStore` for cached chunk reads
- `CProcessChunkWorkItem` for decrypt + decompress

This format is used for:
- CM session cipher (netfilter encryption)
- Depot chunk encryption/decryption
- Manifest filename encryption/decryption

Key sizes: 128-bit or 256-bit (depot/session keys are 256-bit).
Uses AES-NI when available, falls back to software T-table implementation.

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
