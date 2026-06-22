//! 三层配置解析器
//!
//! L0 默认值 → L1 配置文件 → L2 动态覆盖（API 写入）
//! 最终有效值 = L2 ?? L1 ?? L0

use super::config_store::ConfigOverride;
use super::registry::{self, ConfigFieldDef, ConfigValueType};
use serde::Serialize;
use std::collections::HashMap;

/// A resolved config field with all three layers visible.
#[derive(Debug, Clone, Serialize)]
pub struct ResolvedField {
    pub key: String,
    pub value_type: String,
    pub description: String,
    pub dynamic: bool,
    pub reloadable: bool,
    /// L0: always present (from registry default)
    pub default_value: String,
    /// L1: value from config.toml if explicitly set
    pub config_file_value: Option<String>,
    /// L2: dynamic override from SQLite
    pub override_value: Option<String>,
    /// Effective value: L2 ?? L1 ?? L0
    pub effective_value: String,
    /// Source of the effective value: "default", "config_file", or "override"
    pub source: String,
    /// Valid choices for enum fields (empty for other types)
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub choices: Vec<String>,
}

/// Navigate a `toml::Value` tree by dot-separated path.
fn navigate_toml<'a>(root: &'a toml::Value, path: &str) -> Option<&'a toml::Value> {
    let mut current = root;
    for segment in path.split('.') {
        current = current.as_table()?.get(segment)?;
    }
    Some(current)
}

/// Extract a scalar `toml::Value` as a string representation.
fn toml_value_to_string(v: &toml::Value) -> Option<String> {
    match v {
        toml::Value::String(s) => Some(s.clone()),
        toml::Value::Integer(i) => Some(i.to_string()),
        toml::Value::Float(f) => Some(f.to_string()),
        toml::Value::Boolean(b) => Some(b.to_string()),
        _ => None, // tables/arrays are not scalar config values
    }
}

/// Resolve all config fields for a given service.
///
/// - `toml_content`: raw config.toml text (for L1 detection)
/// - `overrides`: all L2 overrides (from `ConfigOverrideStore::list_all()`)
pub fn resolve_for_service(
    service: &str,
    toml_content: &str,
    overrides: &[ConfigOverride],
) -> Vec<ResolvedField> {
    let fields = registry::fields_for_service(service);
    let toml_tree: Option<toml::Value> = toml::from_str(toml_content).ok();
    let override_map: HashMap<&str, &str> = overrides
        .iter()
        .map(|o| (o.key_path.as_str(), o.value.as_str()))
        .collect();

    fields
        .into_iter()
        .map(|field| resolve_field(field, &toml_tree, &override_map))
        .collect()
}

/// Resolve all config fields across all services.
pub fn resolve_all(toml_content: &str, overrides: &[ConfigOverride]) -> Vec<ResolvedField> {
    let fields = registry::all_fields();
    let toml_tree: Option<toml::Value> = toml::from_str(toml_content).ok();
    let override_map: HashMap<&str, &str> = overrides
        .iter()
        .map(|o| (o.key_path.as_str(), o.value.as_str()))
        .collect();

    fields
        .iter()
        .map(|field| resolve_field(field, &toml_tree, &override_map))
        .collect()
}

fn resolve_field(
    field: &ConfigFieldDef,
    toml_tree: &Option<toml::Value>,
    override_map: &HashMap<&str, &str>,
) -> ResolvedField {
    // L1: check if the field exists in the TOML tree
    let config_file_value = toml_tree
        .as_ref()
        .and_then(|tree| navigate_toml(tree, field.toml_path))
        .and_then(toml_value_to_string);

    // L2: check override map
    let override_value = override_map.get(field.key).map(|v| v.to_string());

    // Effective value: L2 ?? L1 ?? L0
    let (effective_value, source) = if let Some(ref ov) = override_value {
        (ov.clone(), "override")
    } else if let Some(ref cv) = config_file_value {
        (cv.clone(), "config_file")
    } else {
        (field.default_value.to_string(), "default")
    };

    ResolvedField {
        key: field.key.to_string(),
        value_type: field.value_type.to_string(),
        description: field.description.to_string(),
        dynamic: field.dynamic,
        reloadable: field.reloadable,
        default_value: field.default_value.to_string(),
        config_file_value,
        override_value,
        effective_value,
        source: source.to_string(),
        choices: field.choices.iter().map(|s| s.to_string()).collect(),
    }
}

/// Apply L2 overrides onto a TOML value tree, then deserialize to `ActrixConfig`.
///
/// This is used during reload to merge dynamic overrides into the config.
pub fn apply_overrides(
    toml_content: &str,
    overrides: &[ConfigOverride],
) -> Result<super::ActrixConfig, String> {
    let mut tree: toml::Value =
        toml::from_str(toml_content).map_err(|e| format!("TOML parse error: {e}"))?;

    for ov in overrides {
        // Only apply if the field is registered and dynamic
        let Some(field) = registry::get_field(&ov.key_path) else {
            continue;
        };
        if !field.dynamic {
            continue;
        }

        set_toml_value(&mut tree, field.toml_path, &ov.value, field.value_type)
            .map_err(|e| format!("Failed to apply override '{}': {e}", ov.key_path))?;
    }

    // Deserialize back to ActrixConfig
    tree.try_into()
        .map_err(|e| format!("Config deserialization error after applying overrides: {e}"))
}

/// Set a value in a TOML tree by dot-separated path.
fn set_toml_value(
    root: &mut toml::Value,
    path: &str,
    value: &str,
    value_type: ConfigValueType,
) -> Result<(), String> {
    let segments: Vec<&str> = path.split('.').collect();

    // Navigate/create intermediate tables
    let mut current = root;
    for &segment in &segments[..segments.len() - 1] {
        if !current.is_table() {
            return Err(format!("Path segment '{segment}' parent is not a table"));
        }
        let table = current.as_table_mut().unwrap();
        if !table.contains_key(segment) {
            table.insert(segment.to_string(), toml::Value::Table(Default::default()));
        }
        current = table.get_mut(segment).unwrap();
    }

    let last = *segments.last().ok_or("Empty path")?;
    let table = current
        .as_table_mut()
        .ok_or_else(|| format!("Parent of '{last}' is not a table"))?;

    let typed_value = match value_type {
        ConfigValueType::String
        | ConfigValueType::Enum
        | ConfigValueType::Range16
        | ConfigValueType::Ip
        | ConfigValueType::Fpath
        | ConfigValueType::Domain
        | ConfigValueType::UriPath => toml::Value::String(value.to_string()),
        ConfigValueType::Bool => toml::Value::Boolean(
            value
                .parse()
                .map_err(|_| format!("Cannot parse '{value}' as bool"))?,
        ),
        ConfigValueType::U8
        | ConfigValueType::U16
        | ConfigValueType::U32
        | ConfigValueType::U64 => toml::Value::Integer(
            value
                .parse::<i64>()
                .map_err(|_| format!("Cannot parse '{value}' as integer"))?,
        ),
    };

    table.insert(last.to_string(), typed_value);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_defaults_only() {
        let fields = resolve_for_service("turn", "", &[]);
        assert!(!fields.is_empty());

        let realm = fields.iter().find(|f| f.key == "turn.realm").unwrap();
        assert_eq!(realm.effective_value, "actrix.local");
        assert_eq!(realm.source, "default");
        assert!(realm.config_file_value.is_none());
        assert!(realm.override_value.is_none());
    }

    #[test]
    fn test_resolve_with_config_file() {
        let toml = r#"
[turn]
realm = "production.example.com"
relay_port_range = "49152-65535"
"#;
        let fields = resolve_for_service("turn", toml, &[]);
        let realm = fields.iter().find(|f| f.key == "turn.realm").unwrap();
        assert_eq!(realm.effective_value, "production.example.com");
        assert_eq!(realm.source, "config_file");
        assert_eq!(
            realm.config_file_value.as_deref(),
            Some("production.example.com")
        );
    }

    #[test]
    fn test_resolve_with_override() {
        let toml = r#"
[turn]
realm = "production.example.com"
"#;
        let overrides = vec![ConfigOverride {
            key_path: "turn.realm".to_string(),
            value: "override.example.com".to_string(),
            updated_at: "2024-01-01".to_string(),
            updated_by: "admin".to_string(),
        }];
        let fields = resolve_for_service("turn", toml, &overrides);
        let realm = fields.iter().find(|f| f.key == "turn.realm").unwrap();
        assert_eq!(realm.effective_value, "override.example.com");
        assert_eq!(realm.source, "override");
        assert_eq!(
            realm.config_file_value.as_deref(),
            Some("production.example.com")
        );
        assert_eq!(
            realm.override_value.as_deref(),
            Some("override.example.com")
        );
    }

    #[test]
    fn test_apply_overrides() {
        let toml = r#"
name = "test"
env = "dev"
location_tag = "test"
actrix_shared_key = "XDDYE8d+yMfdXcdWMrXprcUk2uzjnmoX6nCfFw1gGIg="
sqlite_path = "database"

[bind.ice]
ip = "0.0.0.0"
port = 3478
advertised_ip = "127.0.0.1"
advertised_port = 3478

[turn]
realm = "original.com"
relay_port_range = "49152-65535"
"#;
        let overrides = vec![ConfigOverride {
            key_path: "turn.realm".to_string(),
            value: "overridden.com".to_string(),
            updated_at: "2024-01-01".to_string(),
            updated_by: "admin".to_string(),
        }];

        let config = apply_overrides(toml, &overrides).unwrap();
        assert_eq!(config.turn.realm, "overridden.com");
    }

    #[test]
    fn test_navigate_toml() {
        let val: toml::Value = toml::from_str(
            r#"
[services.signaling.server.rate_limit.connection]
enabled = true
per_minute = 10
"#,
        )
        .unwrap();

        let enabled = navigate_toml(
            &val,
            "services.signaling.server.rate_limit.connection.enabled",
        );
        assert!(enabled.is_some());
        assert_eq!(
            toml_value_to_string(enabled.unwrap()),
            Some("true".to_string())
        );

        let per_min = navigate_toml(
            &val,
            "services.signaling.server.rate_limit.connection.per_minute",
        );
        assert!(per_min.is_some());
        assert_eq!(
            toml_value_to_string(per_min.unwrap()),
            Some("10".to_string())
        );
    }
}
