use actr_protocol::prost::Message;
use actr_protocol::{SIGNALING_ENVELOPE_VERSION, SignalingEnvelope, prost_types::Timestamp};

#[derive(Clone, PartialEq, Message)]
struct LegacySignalingEnvelope {
    #[prost(uint32, required, tag = "1")]
    envelope_version: u32,
    #[prost(string, required, tag = "2")]
    envelope_id: String,
    #[prost(message, required, tag = "4")]
    timestamp: Timestamp,
}

#[derive(Clone, PartialEq, Message)]
struct RetiredTimestampProbe {
    #[prost(message, optional, tag = "4")]
    timestamp: Option<Timestamp>,
}

#[test]
fn new_reader_ignores_v1_timestamp() {
    let legacy = LegacySignalingEnvelope {
        envelope_version: 1,
        envelope_id: "legacy-envelope".to_string(),
        timestamp: Timestamp {
            seconds: 1_700_000_000,
            nanos: 123_000_000,
        },
    };

    let decoded = SignalingEnvelope::decode(legacy.encode_to_vec().as_slice())
        .expect("v2 reader should ignore the retired v1 timestamp");

    assert_eq!(decoded.envelope_version, 1);
    assert_eq!(decoded.envelope_id, "legacy-envelope");
}

#[test]
fn v2_writer_omits_retired_timestamp_tag() {
    assert_eq!(SIGNALING_ENVELOPE_VERSION, 2);

    let envelope = SignalingEnvelope {
        envelope_version: SIGNALING_ENVELOPE_VERSION,
        envelope_id: "v2-envelope".to_string(),
        reply_for: None,
        traceparent: None,
        tracestate: None,
        flow: None,
    };

    let probe = RetiredTimestampProbe::decode(envelope.encode_to_vec().as_slice())
        .expect("probe should decode the v2 envelope");

    assert!(probe.timestamp.is_none());
}
