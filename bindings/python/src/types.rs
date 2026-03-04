use actr_framework::Dest;
use actr_protocol::prost::Message as ProstMessage;
use actr_protocol::{ActrId, ActrIdExt, ActrType, PayloadType as RpPayloadType};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use std::collections::HashMap;

/// Python wrapper for Dest (destination identifier)
#[pyclass(name = "Dest")]
#[derive(Clone)]
pub struct DestPy {
    inner: Dest,
}

#[pymethods]
impl DestPy {
    #[staticmethod]
    fn shell() -> Self {
        DestPy { inner: Dest::Shell }
    }

    #[staticmethod]
    fn local() -> Self {
        DestPy { inner: Dest::Local }
    }

    #[staticmethod]
    fn actor(actr_id: ActrIdPy) -> PyResult<Self> {
        Ok(DestPy {
            inner: Dest::Actor(actr_id.inner().clone()),
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

    fn as_actor_id(&self) -> Option<ActrIdPy> {
        self.inner.as_actor_id().cloned().map(ActrIdPy::from_rust)
    }
}

impl DestPy {
    pub fn inner(&self) -> &Dest {
        &self.inner
    }

    pub fn from_rust(dest: Dest) -> Self {
        DestPy { inner: dest }
    }
}

/// Python wrapper for ActrId
#[pyclass(name = "ActrId")]
#[derive(Clone)]
pub struct ActrIdPy {
    inner: ActrId,
}

#[pymethods]
impl ActrIdPy {
    #[staticmethod]
    fn from_bytes(bytes: &[u8]) -> PyResult<Self> {
        let inner = ActrId::decode(bytes)
            .map_err(|e| PyValueError::new_err(format!("Failed to decode ActrId: {e}")))?;
        Ok(ActrIdPy { inner })
    }

    fn to_bytes(&self) -> Vec<u8> {
        self.inner.encode_to_vec()
    }

    fn __repr__(&self) -> String {
        format!("{}", self.inner.to_string_repr())
    }
}

impl ActrIdPy {
    pub fn inner(&self) -> &ActrId {
        &self.inner
    }

    pub fn from_rust(id: ActrId) -> Self {
        ActrIdPy { inner: id }
    }
}

/// Python wrapper for ActrType
#[pyclass(name = "ActrType")]
#[derive(Clone)]
pub struct ActrTypePy {
    inner: ActrType,
}

#[pymethods]
impl ActrTypePy {
    #[new]
    #[pyo3(signature = (manufacturer, name))]
    fn new(manufacturer: String, name: String) -> PyResult<Self> {
        Ok(ActrTypePy {
            inner: ActrType { manufacturer, name },
        })
    }

    fn to_bytes(&self) -> Vec<u8> {
        self.inner.encode_to_vec()
    }

    #[staticmethod]
    fn from_bytes(bytes: Vec<u8>) -> PyResult<Self> {
        let inner = ActrType::decode(&bytes[..])
            .map_err(|e| PyValueError::new_err(format!("Failed to decode ActrType: {e}")))?;
        Ok(ActrTypePy { inner })
    }

    fn manufacturer(&self) -> String {
        self.inner.manufacturer.clone()
    }

    fn name(&self) -> String {
        self.inner.name.clone()
    }

    fn __repr__(&self) -> String {
        format!(
            "ActrType(manufacturer={}, name={})",
            self.inner.manufacturer, self.inner.name
        )
    }
}

impl ActrTypePy {
    pub fn inner(&self) -> &ActrType {
        &self.inner
    }

    pub fn from_rust(actr_type: ActrType) -> Self {
        ActrTypePy { inner: actr_type }
    }
}

/// Python wrapper for DataStream protobuf message
#[pyclass(name = "DataStream")]
#[derive(Clone)]
pub struct DataStreamPy {
    inner: actr_protocol::DataStream,
}

#[pymethods]
impl DataStreamPy {
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
        Ok(DataStreamPy {
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
        Ok(DataStreamPy { inner })
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

impl DataStreamPy {
    pub fn inner(&self) -> &actr_protocol::DataStream {
        &self.inner
    }

    pub fn from_rust(ds: actr_protocol::DataStream) -> Self {
        DataStreamPy { inner: ds }
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
    pub fn to_rust(self) -> RpPayloadType {
        match self {
            PayloadType::RpcReliable => RpPayloadType::RpcReliable,
            PayloadType::RpcSignal => RpPayloadType::RpcSignal,
            PayloadType::StreamReliable => RpPayloadType::StreamReliable,
            PayloadType::StreamLatencyFirst => RpPayloadType::StreamLatencyFirst,
        }
    }
}
