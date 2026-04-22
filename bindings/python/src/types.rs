use actr_framework::Dest as RtDest;
use actr_protocol::prost::Message as ProstMessage;
use actr_protocol::{
    ActrId as RtActrId, ActrType as RtActrType, PayloadType as RpPayloadType,
};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use std::collections::HashMap;

/// Python wrapper for Dest (destination identifier)
#[pyclass(name = "Dest")]
#[derive(Clone)]
pub struct Dest {
    inner: RtDest,
}

#[pymethods]
impl Dest {
    #[staticmethod]
    fn shell() -> Self {
        Dest { inner: RtDest::Shell }
    }

    #[staticmethod]
    fn local() -> Self {
        Dest { inner: RtDest::Local }
    }

    #[staticmethod]
    fn actor(actr_id: ActrId) -> PyResult<Self> {
        Ok(Dest {
            inner: RtDest::Actor(actr_id.inner().clone()),
        })
    }

    fn is_shell(&self) -> bool {
        self.inner.is_shell()
    }

    fn is_local(&self) -> bool {
        self.inner.is_local()
    }

    fn is_actor(&self) -> bool {
        self.inner.is_actor()
    }

    fn as_actor_id(&self) -> Option<ActrId> {
        self.inner.as_actor_id().cloned().map(ActrId::from_rust)
    }
}

impl Dest {
    pub(crate) fn inner(&self) -> &RtDest {
        &self.inner
    }
}

/// Python wrapper for ActrId
#[pyclass(name = "ActrId")]
#[derive(Clone)]
pub struct ActrId {
    inner: RtActrId,
}

#[pymethods]
impl ActrId {
    #[staticmethod]
    fn from_bytes(bytes: &[u8]) -> PyResult<Self> {
        let inner = RtActrId::decode(bytes)
            .map_err(|e| PyValueError::new_err(format!("Failed to decode ActrId: {e}")))?;
        Ok(ActrId { inner })
    }

    fn to_bytes(&self) -> Vec<u8> {
        self.inner.encode_to_vec()
    }

    fn __repr__(&self) -> String {
        self.inner.to_string_repr()
    }
}

impl ActrId {
    pub(crate) fn inner(&self) -> &RtActrId {
        &self.inner
    }

    pub(crate) fn from_rust(id: RtActrId) -> Self {
        ActrId { inner: id }
    }
}

/// Python wrapper for ActrType
#[pyclass(name = "ActrType")]
#[derive(Clone)]
pub struct ActrType {
    inner: RtActrType,
}

#[pymethods]
impl ActrType {
    #[new]
    fn new(manufacturer: String, name: String, version: String) -> PyResult<Self> {
        if version.is_empty() {
            return Err(PyValueError::new_err(
                "ActrType.version must be a non-empty string",
            ));
        }

        Ok(ActrType {
            inner: RtActrType {
                manufacturer,
                name,
                version,
            },
        })
    }

    fn to_bytes(&self) -> Vec<u8> {
        self.inner.encode_to_vec()
    }

    #[staticmethod]
    fn from_bytes(bytes: Vec<u8>) -> PyResult<Self> {
        let inner = RtActrType::decode(&bytes[..])
            .map_err(|e| PyValueError::new_err(format!("Failed to decode ActrType: {e}")))?;
        Ok(ActrType { inner })
    }

    fn manufacturer(&self) -> String {
        self.inner.manufacturer.clone()
    }

    fn name(&self) -> String {
        self.inner.name.clone()
    }

    fn version(&self) -> String {
        self.inner.version.clone()
    }

    fn __repr__(&self) -> String {
        format!(
            "ActrType(manufacturer={}, name={}, version={})",
            self.inner.manufacturer, self.inner.name, self.inner.version
        )
    }
}

impl ActrType {
    pub(crate) fn inner(&self) -> &RtActrType {
        &self.inner
    }
}

/// Python wrapper for DataStream protobuf message
#[pyclass(name = "DataStream")]
#[derive(Clone)]
pub struct DataStream {
    inner: actr_protocol::DataStream,
}

#[pymethods]
impl DataStream {
    #[new]
    #[pyo3(signature = (stream_id, sequence, payload, timestamp_ms=None, metadata=None))]
    fn new(
        stream_id: String,
        sequence: u64,
        payload: Vec<u8>,
        timestamp_ms: Option<i64>,
        metadata: Option<HashMap<String, String>>,
    ) -> PyResult<Self> {
        let metadata = metadata
            .unwrap_or_default()
            .into_iter()
            .map(|(key, value)| actr_protocol::MetadataEntry { key, value })
            .collect();
        Ok(DataStream {
            inner: actr_protocol::DataStream {
                stream_id,
                sequence,
                payload: payload.into(),
                timestamp_ms,
                metadata,
            },
        })
    }

    #[staticmethod]
    fn from_bytes(bytes: &[u8]) -> PyResult<Self> {
        let inner = actr_protocol::DataStream::decode(bytes)
            .map_err(|e| PyValueError::new_err(format!("Failed to decode DataStream: {e}")))?;
        Ok(DataStream { inner })
    }

    fn to_bytes(&self) -> Vec<u8> {
        self.inner.encode_to_vec()
    }

    fn stream_id(&self) -> String {
        self.inner.stream_id.clone()
    }

    fn sequence(&self) -> u64 {
        self.inner.sequence
    }

    fn payload(&self) -> Vec<u8> {
        self.inner.payload.to_vec()
    }

    fn timestamp_ms(&self) -> Option<i64> {
        self.inner.timestamp_ms
    }

    fn metadata(&self) -> HashMap<String, String> {
        self.inner
            .metadata
            .iter()
            .map(|entry| (entry.key.clone(), entry.value.clone()))
            .collect()
    }
}

impl DataStream {
    pub(crate) fn inner(&self) -> &actr_protocol::DataStream {
        &self.inner
    }

    pub(crate) fn from_rust(ds: actr_protocol::DataStream) -> Self {
        DataStream { inner: ds }
    }
}

#[pyclass(frozen)]
#[derive(Clone, Copy, Debug)]
pub enum PayloadType {
    RpcReliable,
    RpcSignal,
    StreamReliable,
    StreamLatencyFirst,
}

impl PayloadType {
    pub(crate) fn to_rust(self) -> RpPayloadType {
        match self {
            PayloadType::RpcReliable => RpPayloadType::RpcReliable,
            PayloadType::RpcSignal => RpPayloadType::RpcSignal,
            PayloadType::StreamReliable => RpPayloadType::StreamReliable,
            PayloadType::StreamLatencyFirst => RpPayloadType::StreamLatencyFirst,
        }
    }
}
