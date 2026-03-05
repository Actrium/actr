use actr_config::{Config, ConfigParser, RawConfig, RawDependency};
use std::path::Path;

/// Temporary CLI-side compatibility layer for legacy `manufacturer+name` strings.
///
/// This keeps backward behavior in CLI without requiring parser changes in core/config.
///
/// TODO(#76): Remove this compatibility path after Actrix fully supports
/// `manufacturer:name[:version]` in integration + e2e flows.
pub fn load_config_with_legacy_actr_type(path: impl AsRef<Path>) -> actr_config::Result<Config> {
    let path = path.as_ref();
    let mut raw = RawConfig::from_file(path)?;
    normalize_legacy_actr_types(&mut raw);
    ConfigParser::parse(raw, path)
}

/// TODO(#76): Delete when old `manufacturer+name` inputs are no longer accepted.
fn normalize_legacy_actr_types(raw: &mut RawConfig) {
    for dep in raw.dependencies.values_mut() {
        if let RawDependency::Specified { actr_type, .. } = dep
            && let Some(normalized) = normalize_legacy_actr_type_value(actr_type)
        {
            *actr_type = normalized;
        }
    }

    if let Some(acl) = raw.acl.as_mut() {
        if let Ok(mut json_acl) = serde_json::to_value(&*acl) {
            normalize_json_acl_value(&mut json_acl);
            if let Ok(normalized_acl) = serde_json::from_value(json_acl) {
                *acl = normalized_acl;
            }
        }
    }
}

/// TODO(#76): Remove JSON ACL fallback conversion once all ACL inputs use
/// canonical `manufacturer:name[:version]`.
fn normalize_json_acl_value(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::String(s) => {
            if let Some(normalized) = normalize_legacy_actr_type_value(s) {
                *s = normalized;
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                normalize_json_acl_value(item);
            }
        }
        serde_json::Value::Object(table) => {
            for nested in table.values_mut() {
                normalize_json_acl_value(nested);
            }
        }
        _ => {}
    }
}

/// TODO(#76): Remove legacy `+` parser branch in next format-unification PR.
fn normalize_legacy_actr_type_value(raw: &str) -> Option<String> {
    let (manufacturer, remainder) = raw.split_once('+')?;
    if manufacturer.is_empty() || remainder.is_empty() {
        return None;
    }
    if manufacturer.contains('/') || remainder.contains('/') {
        return None;
    }
    if manufacturer.contains(char::is_whitespace) || remainder.contains(char::is_whitespace) {
        return None;
    }
    if remainder.split(':').next().is_none_or(str::is_empty) {
        return None;
    }
    Some(format!("{manufacturer}:{remainder}"))
}
