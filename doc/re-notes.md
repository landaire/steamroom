# Reverse Engineering Notes

Notes from analyzing `steamclient64.dll` via Binary Ninja MCP.

## AES Symmetric Crypto (CCrypto::SymmetricEncrypt / SymmetricDecrypt)

Source: `crypto_aescbc_openssl.cpp`

All symmetric encryption in the Steam client uses the same wire format:

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

This format is used for:
- CM session cipher (netfilter encryption)
- Depot chunk encryption/decryption
- Manifest filename encryption/decryption

Key sizes: 128-bit or 256-bit (depot/session keys are 256-bit).
Uses AES-NI when available, falls back to software T-table implementation.

## Session Cipher (CNetFilterEncryption)

Source: `netfilter.cpp`

### Encrypt (CWorkItemNetFilterEncrypt::ThreadProcess)

1. Compute `hmac = HMAC-SHA1(session_key[0..16], header_3bytes + plaintext)`
2. Build `IV = hmac[0..13] || header_3bytes` (16 bytes total)
3. AES-256-CBC encrypt `plaintext` using `IV` and full 32-byte `session_key`, PKCS7
4. AES-256-ECB encrypt `IV` using the full 32-byte key
5. Output: `ECB(IV) || CBC(plaintext)`

The 3-byte header is a sequence counter stored at IV[13..16]. In practice these are
zero bytes for the initial messages.

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

Source: `crypto_rsa_openssl.cpp`

- Uses OpenSSL `RSA_PKCS1_OAEP_PADDING` (value 4) = OAEP with SHA-1
- Public key for universe "Public" stored as hex-encoded DER at `0x1394161d0`
- Key is 1024-bit RSA, exponent 0x11 (17)
- Encrypts `session_key(32) + nonce(16)` = 48 bytes
- 23 universe keys embedded total; universe 0 (index 0) is the "Public" universe key

## CM Server Protocol

### Connection Flow (TCP / Netfilter)

1. TCP connect to CM server (port 27017)
2. Server sends `ChannelEncryptRequest` (EMsg 1303) with protocol_version, universe, 16-byte nonce
3. Client RSA-OAEP encrypts `session_key + nonce`, sends `ChannelEncryptResponse` (EMsg 1304)
4. Server sends `ChannelEncryptResult` (EMsg 1305) with eresult
5. Client sends `ClientHello` (EMsg 9805) with protocol_version
6. Client sends `ClientLogon` (EMsg 5514) with auth credentials
7. All messages after step 4 are encrypted with the session cipher

### Connection Flow (WebSocket)

1. WebSocket connect to `wss://{host}:{port}/cmsocket/`
2. TLS handles encryption — no ChannelEncrypt handshake needed
3. Client sends `ClientHello` (EMsg 9805), then `ClientLogon` (EMsg 5514)
4. Messages are plaintext protobuf over the WebSocket binary frames

### VT01 Framing (TCP only)

All TCP messages use VT01 framing:
```
[payload_length: u32 LE] [magic: "VT01" = 0x56543031] [payload]
```

### Message Format (Protobuf)

```
[raw_emsg: u32 LE (emsg | 0x80000000)] [header_len: u32 LE] [CMsgProtoBufHeader] [body]
```

## Depot Manifest Format

### V4 Format

Section layout:
```
[magic: 0x71F617D0] [size: u32 LE] [ContentManifestPayload protobuf]
[magic: 0x1F4812BE] [size: u32 LE] [ContentManifestMetadata protobuf]
[magic: 0x1B81B817] [size: u32 LE] [signature data]
[4 bytes trailing CRC]
```

No EndOfManifest marker — the parser reads until EOF or unknown magic.

### V5 Format

```
[magic: 0x1B81B817] [size: u32 LE] [ContentManifestPayload protobuf]
[magic: 0x1F4DB10B] [size: u32 LE] [ContentManifestMetadata protobuf]
[magic: 0x1B81B813] [size: u32 LE] [ContentManifestSignature protobuf]
[magic: 0xD64BF064] (EndOfManifest)
```

Manifest data from CDN is ZIP-compressed (PK header).

## Chunk Compression (VZ / Valve LZMA)

Chunk data after decryption may be compressed. Detected by first bytes:

| Prefix | Format |
|--------|--------|
| `PK` (0x50 0x4B) | ZIP archive — extract first file |
| `VZ` (0x56 0x5A) | Valve LZMA (see below) |
| `5D` (0x5D) | Raw LZMA stream |
| Other | Uncompressed |

### VZ Format

```
"VZ" (2 bytes) + VZ header (5 bytes) + LZMA properties (5 bytes) + LZMA data
```

The LZMA properties byte starts at offset 7. The uncompressed size is NOT in
the stream — it comes from the manifest's `cb_original` field per chunk. To
decode, construct a standard LZMA stream: `props(5) + uncompressed_size(8 LE) + data`.

## Download Pipeline (CCSInterface / CDepotReconstruct)

Source: `csinterface.cpp`, `depotreconstruct.cpp`

### Architecture

1. **CCSInterface** — manages HTTP chunk requests to CDN servers
2. **CDepotReconstruct** — orchestrates download: manifest parsing,
   chunk scheduling, decrypt, decompress, file assembly

### Concurrency Model

From `CCSInterface::YieldingDownloadChunks`:

- **Batching**: chunks submitted in a tight loop, not one at a time
- **Hard cap**: 1024 (`0x400`) total outstanding requests
- **Bandwidth-adaptive throttle**: outstanding bytes limited to `bandwidth * 6.0`
  (6 seconds of buffered data based on measured download rate)
- **Minimum concurrency**: 2 requests (normal) or 8 (for certain server types)
- Tracked via `m_unChunkRequestsOutstanding` (offset 0x298) and
  `m_cubChunksOutstanding` (offset 0x288)

### Thread Pools

- IO thread pool: configurable via `DepotReconstructionNumIOThreads` convar (default 32)
- Separate decrypt thread pool for CPU-bound AES/decompress work

## Proto Extraction

`steamclient64.dll` embeds serialized `FileDescriptorProto` blobs in `.rdata`.
Registration entries in `.data` contain `(size: u32, ptr: u64)` pairs pointing to blobs.
Scanner finds blobs by matching `0x0a` + varint + `.proto` suffix pattern.
106 proto files extracted successfully using `proto-extract` tool.
