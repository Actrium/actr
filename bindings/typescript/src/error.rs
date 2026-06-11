//! Structured error payload for napi-rs consumers.
//!
//! napi-rs's `Error` type is flat (status + string message). To preserve the
//! 10-variant classification from `actr_protocol::ActrError` through the JS
//! boundary without losing structure, we serialize a small JSON object into
//! the message string:
//!
//! ```json
//! {
//!   "kind": "Client",
//!   "code": "DependencyNotFound",
//!   "message": "dependency 'a' not found: missing",
//!   "service_name": "a"
//! }
//! ```
//!
//! A companion `ActrError` class is declared in `index.d.ts`; the JS wrapper
//! in `typescript/*.ts` parses the JSON and re-throws a typed instance.

use actr_protocol::{
    ActrError, ActrId, Classify, DeliveryState, ErrorKind, RecoveryCode, RecoveryInfo,
};

/// Discriminate a protocol error into `(variant_code, user_message,
/// optional service_name, optional recovery_info)`.
///
/// `recovery_info` is present when the error is `Recovering`; `None` otherwise.
fn discriminate(e: &ActrError) -> (&'static str, String, Option<String>, Option<&RecoveryInfo>) {
    match e {
        ActrError::Unavailable(msg) => ("Unavailable", msg.clone(), None, None),
        ActrError::Recovering(info) => ("Recovering", info.to_string(), None, Some(info)),
        ActrError::TimedOut => ("TimedOut", "operation timed out".to_string(), None, None),
        ActrError::NotFound(msg) => ("NotFound", msg.clone(), None, None),
        ActrError::PermissionDenied(msg) => ("PermissionDenied", msg.clone(), None, None),
        ActrError::InvalidArgument(msg) => ("InvalidArgument", msg.clone(), None, None),
        ActrError::UnknownRoute(msg) => ("UnknownRoute", msg.clone(), None, None),
        ActrError::DependencyNotFound {
            service_name,
            message,
        } => (
            "DependencyNotFound",
            message.clone(),
            Some(service_name.clone()),
            None,
        ),
        ActrError::DecodeFailure(msg) => ("DecodeFailure", msg.clone(), None, None),
        ActrError::NotImplemented(msg) => ("NotImplemented", msg.clone(), None, None),
        ActrError::Internal(msg) => ("Internal", msg.clone(), None, None),
    }
}

fn recovery_code(code: RecoveryCode) -> &'static str {
    match code {
        RecoveryCode::PeerDisconnected => "PeerDisconnected",
        RecoveryCode::PeerFailed => "PeerFailed",
        RecoveryCode::IceNetworkStarted => "IceNetworkStarted",
        RecoveryCode::RecoveryTimeout => "RecoveryTimeout",
        RecoveryCode::TransportClosing => "TransportClosing",
    }
}

fn delivery_state(state: DeliveryState) -> &'static str {
    match state {
        DeliveryState::NotSent => "NotSent",
        DeliveryState::DeliveryUncertain => "DeliveryUncertain",
    }
}

fn kind_str(kind: ErrorKind) -> &'static str {
    match kind {
        ErrorKind::Transient => "Transient",
        ErrorKind::Client => "Client",
        ErrorKind::Internal => "Internal",
        ErrorKind::Corrupt => "Corrupt",
    }
}

/// Escape a string so it is safe to embed in a JSON double-quoted literal.
///
/// We only need a minimal subset here — backslash, double-quote, and the
/// common control chars. Anything more exotic falls through to
/// `\uXXXX`-style escaping via `char::escape_unicode` to keep the payload
/// valid for downstream `JSON.parse`.
fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\x08' => out.push_str("\\b"),
            '\x0c' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out
}

fn option_u64_json(value: Option<u64>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "null".to_string())
}

fn actr_id_json(id: &ActrId) -> String {
    format!(
        r#"{{"realm_id":{realm_id},"serial_number":{serial_number},"type":{{"manufacturer":"{manufacturer}","name":"{name}","version":"{version}"}}}}"#,
        realm_id = id.realm.realm_id,
        serial_number = id.serial_number,
        manufacturer = json_escape(&id.r#type.manufacturer),
        name = json_escape(&id.r#type.name),
        version = json_escape(&id.r#type.version),
    )
}

/// Build a JSON payload carrying kind / code / message / optional service_name
/// and recovery metadata — the structure the `ActrError` JS class expects.
fn build_payload(
    kind: ErrorKind,
    code: &str,
    message: &str,
    service_name: Option<&str>,
    recovery_info: Option<&RecoveryInfo>,
) -> String {
    let mut fields = vec![
        format!(r#""kind":"{}""#, kind_str(kind)),
        format!(r#""code":"{}""#, code),
        format!(r#""message":"{}""#, json_escape(message)),
    ];

    if let Some(svc) = service_name {
        fields.push(format!(r#""service_name":"{}""#, json_escape(svc)));
    }

    if let Some(info) = recovery_info {
        fields.push(format!(r#""recovery_code":"{}""#, recovery_code(info.code)));
        fields.push(format!(r#""peer":{}"#, actr_id_json(&info.peer)));
        fields.push(format!(
            r#""session_id":{}"#,
            option_u64_json(info.session_id)
        ));
        fields.push(format!(r#""reason":"{}""#, json_escape(&info.reason)));
        fields.push(format!(r#""elapsed_ms":{}"#, info.elapsed_ms));
        fields.push(format!(r#""timeout_ms":{}"#, info.timeout_ms));
        fields.push(format!(
            r#""retry_after_ms":{}"#,
            option_u64_json(info.retry_after_ms)
        ));
        fields.push(format!(r#""delivery":"{}""#, delivery_state(info.delivery)));
    }

    format!("{{{}}}", fields.join(","))
}

/// Convert a protocol-level error into a `napi::Error` carrying the
/// structured JSON payload.
pub(crate) fn actr_error_to_napi(e: actr_protocol::ActrError) -> napi::Error {
    let kind = e.kind();
    let (code, message, service_name, recovery_info) = discriminate(&e);
    let payload = build_payload(kind, code, &message, service_name.as_deref(), recovery_info);
    napi::Error::new(napi::Status::GenericFailure, payload)
}

/// Same shape as [`actr_error_to_napi`] — retained so existing call sites
/// don't need to migrate in one giant patch.
pub(crate) fn protocol_error_to_napi(e: actr_protocol::ActrError) -> napi::Error {
    actr_error_to_napi(e)
}

/// Pre-protocol config failure. Classified as `Client` (the caller gave us
/// a bad manifest / config file).
pub(crate) fn config_error_to_napi(e: actr_config::ConfigError) -> napi::Error {
    let payload = build_payload(ErrorKind::Client, "Config", &e.to_string(), None, None);
    napi::Error::new(napi::Status::GenericFailure, payload)
}

/// Hyper-layer initialization error. Maps to Client/Internal depending on
/// the underlying failure — we lean `Internal` because hyper bootstrap
/// failures almost always indicate a platform/runtime problem rather than
/// bad caller input.
pub(crate) fn hyper_error_to_napi(e: actr_hyper::HyperError) -> napi::Error {
    let payload = build_payload(
        ErrorKind::Internal,
        "HyperBootstrap",
        &e.to_string(),
        None,
        None,
    );
    napi::Error::new(napi::Status::GenericFailure, payload)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dependency_not_found_includes_service_name() {
        let err = actr_error_to_napi(actr_protocol::ActrError::DependencyNotFound {
            service_name: "echo".to_string(),
            message: "missing".to_string(),
        });
        let msg = err.reason.as_str();
        assert!(msg.contains(r#""kind":"Client""#), "kind: {msg}");
        assert!(
            msg.contains(r#""code":"DependencyNotFound""#),
            "code: {msg}"
        );
        assert!(msg.contains(r#""service_name":"echo""#), "svc: {msg}");
    }

    #[test]
    fn timed_out_classifies_as_transient() {
        let err = actr_error_to_napi(actr_protocol::ActrError::TimedOut);
        let msg = err.reason.as_str();
        assert!(msg.contains(r#""kind":"Transient""#));
        assert!(msg.contains(r#""code":"TimedOut""#));
    }

    #[test]
    fn decode_failure_classifies_as_corrupt() {
        let err = actr_error_to_napi(actr_protocol::ActrError::DecodeFailure("bad".into()));
        let msg = err.reason.as_str();
        assert!(msg.contains(r#""kind":"Corrupt""#));
        assert!(msg.contains(r#""code":"DecodeFailure""#));
    }

    #[test]
    fn json_escapes_embedded_quotes() {
        let err = actr_error_to_napi(actr_protocol::ActrError::InvalidArgument(
            r#"a"b"#.to_string(),
        ));
        let msg = err.reason.as_str();
        assert!(msg.contains(r#""message":"a\"b""#), "escaped: {msg}");
    }

    #[test]
    fn recovering_includes_structured_retry_metadata() {
        let peer = actr_protocol::ActrId {
            realm: actr_protocol::Realm { realm_id: 1 },
            serial_number: 42,
            r#type: actr_protocol::ActrType {
                manufacturer: "acme".to_string(),
                name: "mobile".to_string(),
                version: "1.0.0".to_string(),
            },
        };
        let err = actr_error_to_napi(actr_protocol::ActrError::Recovering(
            actr_protocol::RecoveryInfo::new(
                peer,
                Some(7),
                actr_protocol::RecoveryCode::PeerDisconnected,
                "peer state Disconnected",
                120,
                6000,
            ),
        ));
        let msg = err.reason.as_str();
        assert!(msg.contains(r#""kind":"Transient""#), "kind: {msg}");
        assert!(msg.contains(r#""code":"Recovering""#), "code: {msg}");
        assert!(
            msg.contains(r#""recovery_code":"PeerDisconnected""#),
            "recovery_code: {msg}"
        );
        assert!(msg.contains(r#""serial_number":42"#), "peer: {msg}");
        assert!(msg.contains(r#""session_id":7"#), "session_id: {msg}");
        assert!(msg.contains(r#""elapsed_ms":120"#), "elapsed_ms: {msg}");
        assert!(msg.contains(r#""timeout_ms":6000"#), "timeout_ms: {msg}");
        assert!(
            msg.contains(r#""retry_after_ms":5880"#),
            "retry_after_ms: {msg}"
        );
        assert!(msg.contains(r#""delivery":"NotSent""#), "delivery: {msg}");
    }
}
