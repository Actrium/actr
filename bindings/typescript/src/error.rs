// Error conversion utilities
// Note: We can't implement From traits for foreign types, so we use helper functions

pub fn actr_error_to_napi(e: actr_protocol::ActrError) -> napi::Error {
    napi::Error::from_reason(format!("Actr error: {}", e))
}

#[allow(dead_code)]
pub fn protocol_error_to_napi(e: actr_protocol::ActrError) -> napi::Error {
    actr_error_to_napi(e)
}

pub fn config_error_to_napi(e: actr_config::ConfigError) -> napi::Error {
    napi::Error::from_reason(format!("Config error: {}", e))
}

pub fn hyper_error_to_napi(e: actr_hyper::HyperError) -> napi::Error {
    napi::Error::from_reason(format!("Hyper error: {}", e))
}
