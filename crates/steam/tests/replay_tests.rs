use steam::transport::capture::{CaptureFile, CapturedPacket};
use steam::transport::replay::ReplayTransport;
use steam::transport::Transport;

fn make_echo_capture() -> CaptureFile {
    // Simulated capture: a Multi message containing a service method response
    // This tests the transport abstraction without hitting the network.
    let mut packets = Vec::new();

    // Packet 0: a simple protobuf message (EMsg::Multi with empty body)
    let raw_emsg: u32 = 1 | 0x8000_0000; // EMsg::MULTI with proto flag
    let header_len: u32 = 0;
    let mut payload = Vec::new();
    payload.extend_from_slice(&raw_emsg.to_le_bytes());
    payload.extend_from_slice(&header_len.to_le_bytes());
    // Empty CMsgMulti body
    packets.push(CapturedPacket::new(0, &payload));

    CaptureFile {
        description: "test capture".to_string(),
        packets,
    }
}

#[tokio::test]
async fn replay_transport_delivers_packets() {
    let capture = make_echo_capture();
    let transport = ReplayTransport::from_capture(capture);

    let data = transport.recv().await.unwrap();
    assert!(!data.is_empty());

    // Second recv should fail (no more packets)
    let err = transport.recv().await;
    assert!(err.is_err());
}

#[tokio::test]
async fn replay_transport_send_is_noop() {
    let capture = make_echo_capture();
    let transport = ReplayTransport::from_capture(capture);

    // Send should succeed silently
    transport.send(b"hello").await.unwrap();
}

#[test]
fn capture_file_roundtrip() {
    let capture = make_echo_capture();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.json");

    capture.save(&path).unwrap();
    let loaded = CaptureFile::load(&path).unwrap();

    assert_eq!(loaded.description, "test capture");
    assert_eq!(loaded.packets.len(), 1);
    assert_eq!(loaded.packets[0].seq, 0);

    // Payload roundtrips through base64
    let original = capture.packets[0].decode_payload().unwrap();
    let decoded = loaded.packets[0].decode_payload().unwrap();
    assert_eq!(original, decoded);
}

#[tokio::test]
async fn replay_parses_as_incoming_msg() {
    let capture = make_echo_capture();
    let transport = ReplayTransport::from_capture(capture);

    let data = transport.recv().await.unwrap();

    // Parse as a packet header
    let parsed = steam::messages::header::parse_packet_header(&data).unwrap();
    match parsed {
        steam::messages::header::PacketHeader::Protobuf { header, body } => {
            assert_eq!(header.emsg, steam::messages::EMsg::MULTI);
            assert!(header.is_protobuf);
            assert!(body.is_empty());
        }
        _ => panic!("expected protobuf header"),
    }
}
