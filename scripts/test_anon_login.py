#!/usr/bin/env python3
"""Minimal Steam anonymous login test to verify protocol correctness."""
import socket, struct, os, hashlib, hmac, sys, zlib

from cryptography.hazmat.primitives.asymmetric import padding
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.ciphers import Cipher, algorithms, modes
from cryptography.hazmat.primitives import padding as sym_padding

STEAM_PUBLIC_KEY_DER = bytes.fromhex(
    "30819d300d06092a864886f70d010101050003818b00308187"
    "02818100dfec1ad62c10662c17353a14b07c59117f9dd3d82b"
    "7ae3e015cd191e46e87b8774a2184631a9031479828ee945a2"
    "4912a923687389cf69a1b16146bdc1bebfd6011bd881d4dc90"
    "fbfe4f527366cb9570d7c58eba1c7a3375a1623446bb60b780"
    "68fa13a77a8a374b9ec6f45d5f3a99f99ec43ae963a2bb8819"
    "28e0e714c042890201 11".replace(" ", "")
)

def recv_frame(s):
    data = b''
    while len(data) < 8:
        chunk = s.recv(8 - len(data))
        if not chunk:
            raise ConnectionError("EOF reading frame header")
        data += chunk
    length = struct.unpack('<I', data[:4])[0]
    assert data[4:8] == b'VT01', f"bad magic: {data[4:8]}"
    payload = b''
    while len(payload) < length:
        chunk = s.recv(length - len(payload))
        if not chunk:
            raise ConnectionError("EOF reading frame payload")
        payload += chunk
    return payload

def send_frame(s, payload):
    frame = struct.pack('<I', len(payload)) + b'VT01' + payload
    s.sendall(frame)

def session_encrypt(plaintext, key):
    header_3 = b'\x00\x00\x00'
    h = hmac.new(key[:16], header_3 + plaintext, hashlib.sha1)
    iv = h.digest()[:13] + header_3
    padder = sym_padding.PKCS7(128).padder()
    padded = padder.update(plaintext) + padder.finalize()
    c = Cipher(algorithms.AES256(key), modes.CBC(iv))
    enc = c.encryptor()
    return iv + enc.update(padded) + enc.finalize()

def session_decrypt(data, key):
    iv, ct = data[:16], data[16:]
    c = Cipher(algorithms.AES256(key), modes.CBC(iv))
    dec = c.decryptor()
    padded = dec.update(ct) + dec.finalize()
    unpadder = sym_padding.PKCS7(128).unpadder()
    return unpadder.update(padded) + unpadder.finalize()

def varint(v):
    r = b''
    while v > 0x7f:
        r += bytes([v & 0x7f | 0x80]); v >>= 7
    return r + bytes([v & 0x7f])

def field_varint(fn, v):
    return varint((fn << 3) | 0) + varint(v)

def field_fixed64(fn, v):
    return varint((fn << 3) | 1) + struct.pack('<Q', v)

def field_string(fn, v):
    encoded = v.encode() if isinstance(v, str) else v
    return varint((fn << 3) | 2) + varint(len(encoded)) + encoded

def main():
    s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    s.settimeout(15)

    # Use a known working CM IP
    cm = ('162.254.195.66', 27017)
    print(f"Connecting to {cm}...")
    s.connect(cm)

    # 1. ChannelEncryptRequest
    payload = recv_frame(s)
    body = payload[20:]
    nonce = body[8:] if len(body) > 8 else b''
    print(f"EncryptRequest: nonce={nonce.hex()}")

    # 2. Generate session key, RSA encrypt
    session_key = os.urandom(32)
    pub_key = serialization.load_der_public_key(STEAM_PUBLIC_KEY_DER)
    encrypted_key = pub_key.encrypt(
        session_key + nonce,
        padding.OAEP(mgf=padding.MGF1(hashes.SHA1()), algorithm=hashes.SHA1(), label=None)
    )

    # 3. ChannelEncryptResponse
    crc = zlib.crc32(encrypted_key) & 0xFFFFFFFF
    resp_body = struct.pack('<II', 1, len(encrypted_key)) + encrypted_key + struct.pack('<II', crc, 0)
    resp = struct.pack('<I', 1304) + struct.pack('<QQ', 0xFFFFFFFFFFFFFFFF, 0xFFFFFFFFFFFFFFFF) + resp_body
    send_frame(s, resp)

    # 4. ChannelEncryptResult
    payload = recv_frame(s)
    eresult = struct.unpack('<I', payload[20:24])[0]
    print(f"EncryptResult: eresult={eresult}")
    if eresult != 1:
        print("Encryption handshake FAILED")
        return 1

    # 5. Build CMsgClientLogon for anonymous
    # CMsgIPAddress for obfuscated_private_ip (tag 11): just an ipv4 field
    obfuscated_ip = field_varint(1, 0)  # CMsgIPAddress.ip (tag 1)

    # SteamID: universe=1, type=AnonUser(10), instance=0, id=0
    anon_steam_id = (1 << 56) | (10 << 52)

    logon_body = (
        field_varint(1, 65580) +        # protocol_version
        field_varint(3, 0) +             # cell_id
        field_varint(7, 20) +            # client_os_type
        field_string(6, "english") +     # client_language
        field_string(11, obfuscated_ip) + # obfuscated_private_ip (nested msg)
        field_fixed64(22, anon_steam_id) + # client_supplied_steam_id
        field_varint(31, 0) +            # launcher_type
        field_varint(32, 0) +            # ui_mode
        field_varint(33, 2) +            # chat_mode = 2
        field_string(96, "")             # machine_name (empty)
    )

    # Proto header
    proto_hdr = field_fixed64(1, anon_steam_id) + field_varint(2, 0)

    # Full packet
    raw_emsg = 5514 | 0x80000000
    packet = struct.pack('<I', raw_emsg)
    packet += struct.pack('<I', len(proto_hdr))
    packet += proto_hdr + logon_body

    print(f"Logon packet: {len(packet)} bytes")
    print(f"  hex: {packet.hex()}")

    encrypted = session_encrypt(packet, session_key)
    send_frame(s, encrypted)
    print(f"Sent encrypted logon ({len(encrypted)} bytes), waiting...")

    try:
        resp_data = recv_frame(s)
        print(f"Got response! {len(resp_data)} bytes")
        decrypted = session_decrypt(resp_data, session_key)
        raw_emsg = struct.unpack('<I', decrypted[:4])[0]
        emsg = raw_emsg & 0x7FFFFFFF
        is_proto = (raw_emsg >> 31) & 1
        print(f"Response: EMsg={emsg}, proto={is_proto}, {len(decrypted)} bytes")
        if emsg == 751:
            hdr_len = struct.unpack('<I', decrypted[4:8])[0]
            body = decrypted[8 + hdr_len:]
            # Parse eresult (field 1, varint)
            if body[0] == 0x08:
                er = body[1] if body[1] < 0x80 else -1
                print(f"LogonResponse eresult={er}")
        return 0
    except socket.timeout:
        print("TIMEOUT - server did not respond!")
        return 1
    finally:
        s.close()

if __name__ == '__main__':
    sys.exit(main())
