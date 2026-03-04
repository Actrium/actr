use napi::bindgen_prelude::*;
use napi_derive::napi;

// Realm
#[napi(object)]
pub struct Realm {
    #[napi(js_name = "realmId")]
    pub realm_id: u32,
}

impl From<actr_protocol::Realm> for Realm {
    fn from(realm: actr_protocol::Realm) -> Self {
        Self {
            realm_id: realm.realm_id,
        }
    }
}

impl From<Realm> for actr_protocol::Realm {
    fn from(realm: Realm) -> Self {
        Self {
            realm_id: realm.realm_id,
        }
    }
}

// ActrType
#[napi(object)]
pub struct ActrType {
    pub manufacturer: String,
    pub name: String,
}

impl From<actr_protocol::ActrType> for ActrType {
    fn from(t: actr_protocol::ActrType) -> Self {
        Self {
            manufacturer: t.manufacturer,
            name: t.name,
        }
    }
}

impl From<ActrType> for actr_protocol::ActrType {
    fn from(t: ActrType) -> Self {
        Self {
            manufacturer: t.manufacturer,
            name: t.name,
        }
    }
}

// ActrId
#[napi(object)]
pub struct ActrId {
    pub realm: Realm,
    #[napi(js_name = "serialNumber")]
    pub serial_number: i64, // i64 for JavaScript number safety
    pub r#type: ActrType,
}

impl From<actr_protocol::ActrId> for ActrId {
    fn from(id: actr_protocol::ActrId) -> Self {
        Self {
            realm: id.realm.into(),
            serial_number: id.serial_number as i64,
            r#type: id.r#type.into(),
        }
    }
}

impl From<ActrId> for actr_protocol::ActrId {
    fn from(id: ActrId) -> Self {
        Self {
            realm: id.realm.into(),
            serial_number: id.serial_number as u64,
            r#type: id.r#type.into(),
        }
    }
}

// PayloadType
#[napi]
pub enum PayloadType {
    RpcReliable,
    RpcSignal,
    StreamReliable,
    StreamLatencyFirst,
    MediaRtp,
}

impl From<actr_protocol::PayloadType> for PayloadType {
    fn from(t: actr_protocol::PayloadType) -> Self {
        match t {
            actr_protocol::PayloadType::RpcReliable => PayloadType::RpcReliable,
            actr_protocol::PayloadType::RpcSignal => PayloadType::RpcSignal,
            actr_protocol::PayloadType::StreamReliable => PayloadType::StreamReliable,
            actr_protocol::PayloadType::StreamLatencyFirst => PayloadType::StreamLatencyFirst,
            actr_protocol::PayloadType::MediaRtp => PayloadType::MediaRtp,
        }
    }
}

impl From<PayloadType> for actr_protocol::PayloadType {
    fn from(t: PayloadType) -> Self {
        match t {
            PayloadType::RpcReliable => actr_protocol::PayloadType::RpcReliable,
            PayloadType::RpcSignal => actr_protocol::PayloadType::RpcSignal,
            PayloadType::StreamReliable => actr_protocol::PayloadType::StreamReliable,
            PayloadType::StreamLatencyFirst => actr_protocol::PayloadType::StreamLatencyFirst,
            PayloadType::MediaRtp => actr_protocol::PayloadType::MediaRtp,
        }
    }
}

// MetadataEntry
#[napi(object)]
pub struct MetadataEntry {
    pub key: String,
    pub value: String,
}

// DataStream
#[napi(object)]
pub struct DataStream {
    #[napi(js_name = "streamId")]
    pub stream_id: String,
    pub sequence: i64,
    pub payload: Buffer,
    pub metadata: Vec<MetadataEntry>,
    #[napi(js_name = "timestampMs")]
    pub timestamp_ms: Option<i64>,
}

#[napi(object)]
pub struct StreamSignal {
    pub chunk: DataStream,
    pub sender: ActrId,
}

impl From<actr_protocol::DataStream> for DataStream {
    fn from(stream: actr_protocol::DataStream) -> Self {
        Self {
            stream_id: stream.stream_id,
            sequence: stream.sequence as i64,
            payload: stream.payload.to_vec().into(),
            metadata: stream
                .metadata
                .into_iter()
                .map(|e| MetadataEntry {
                    key: e.key,
                    value: e.value,
                })
                .collect(),
            timestamp_ms: stream.timestamp_ms,
        }
    }
}

impl From<DataStream> for actr_protocol::DataStream {
    fn from(stream: DataStream) -> Self {
        Self {
            stream_id: stream.stream_id,
            sequence: stream.sequence as u64,
            payload: bytes::Bytes::from(stream.payload.to_vec()),
            metadata: stream
                .metadata
                .into_iter()
                .map(|e| actr_protocol::MetadataEntry {
                    key: e.key,
                    value: e.value,
                })
                .collect(),
            timestamp_ms: stream.timestamp_ms,
        }
    }
}

// RpcEnvelopeBridge
#[napi(object)]
pub struct RpcEnvelopeBridge {
    #[napi(js_name = "routeKey")]
    pub route_key: String,
    pub payload: Buffer,
    #[napi(js_name = "requestId")]
    pub request_id: String,
}

impl From<actr_protocol::RpcEnvelope> for RpcEnvelopeBridge {
    fn from(envelope: actr_protocol::RpcEnvelope) -> Self {
        Self {
            route_key: envelope.route_key,
            payload: envelope
                .payload
                .map(|p| p.to_vec().into())
                .unwrap_or_else(|| Buffer::from(vec![])),
            request_id: envelope.request_id,
        }
    }
}
