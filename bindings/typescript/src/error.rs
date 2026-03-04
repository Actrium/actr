// Error conversion utilities
// Note: We can't implement From traits for foreign types, so we use helper functions

pub fn protocol_error_to_napi(e: actr_protocol::ProtocolError) -> napi::Error {
    napi::Error::from_reason(format!("Protocol error: {}", e))
}

#[allow(dead_code)]
pub fn runtime_error_to_napi(e: actr_runtime::RuntimeError) -> napi::Error {
    napi::Error::from_reason(format!("Runtime error: {}", e))
}

pub fn config_error_to_napi(e: actr_config::ConfigError) -> napi::Error {
    napi::Error::from_reason(format!("Config error: {}", e))
}
