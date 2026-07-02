use super::*;

#[test]
fn test_transport_message_decode() {
    // Manually construct encoded message:
    // [payload_type: 1 byte][data_len: 4 bytes][data: N bytes]
    let mut encoded = Vec::new();
    encoded.push(PayloadType::RpcReliable as u8); // payload_type = 0
    encoded.extend_from_slice(&11u32.to_be_bytes()); // length = 11
    encoded.extend_from_slice(b"hello world"); // data

    let decoded =
        TransportMessage::decode(&encoded).expect("Should decode valid TransportMessage in test");

    assert_eq!(decoded.payload_type as u8, PayloadType::RpcReliable as u8);
    assert_eq!(decoded.data, b"hello world");
}

#[test]
fn test_transport_message_decode_invalid() {
    // message too short
    let data = vec![1, 0, 0];
    assert!(TransportMessage::decode(&data).is_err());

    // no effect 's payload_type
    let data = vec![99, 0, 0, 0, 5, 1, 2, 3, 4, 5];
    assert!(TransportMessage::decode(&data).is_err());
}
