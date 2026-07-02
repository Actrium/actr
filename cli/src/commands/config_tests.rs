use super::*;

#[test]
fn parses_full_toml_document_as_value_table() {
    let value = parse_toml_document_value(
        r#"
[mfr]
manufacturer = "demo1"

[network]
realm_id = 2368266035
"#,
        ".actr/config.toml",
    )
    .expect("config TOML should parse");

    assert_eq!(
        ConfigCommand::get_nested_value(&value, "mfr.manufacturer"),
        Some(&Value::String("demo1".to_string()))
    );
}

#[test]
fn parse_toml_document_value_errors_on_invalid_input() {
    assert!(parse_toml_document_value("invalid = {{{", "bad.toml").is_err());
}

#[test]
fn get_nested_value_returns_none_for_missing_or_non_table_path() {
    let value = parse_toml_document_value("[mfr]\nmanufacturer = \"x\"\n", "f").unwrap();
    assert!(ConfigCommand::get_nested_value(&value, "mfr.missing").is_none());
    assert!(ConfigCommand::get_nested_value(&value, "nonexistent.key").is_none());
    // Non-table intermediate stops traversal.
    assert!(ConfigCommand::get_nested_value(&value, "mfr.manufacturer.nested").is_none());
}

#[test]
fn apply_key_to_config_sets_every_known_string_field() {
    let mut config = CliConfig::default();
    for (key, val) in [
        ("mfr.manufacturer", "acme"),
        ("mfr.keychain", "/keys/k.json"),
        ("codegen.language", "rust"),
        ("codegen.output", "src/gen"),
        ("cache.dir", "/tmp/cache"),
        ("network.signaling_url", "ws://localhost"),
        ("network.ais_endpoint", "http://ais"),
        ("network.realm_secret", "secret"),
        ("storage.hyper_data_dir", "/hyper"),
        ("ui.format", "json"),
        ("ui.color", "auto"),
    ] {
        ConfigCommand::apply_key_to_config(&mut config, key, val)
            .unwrap_or_else(|e| panic!("apply {key} failed: {e}"));
    }
    assert_eq!(config.mfr.manufacturer.as_deref(), Some("acme"));
    assert_eq!(
        config.network.signaling_url.as_deref(),
        Some("ws://localhost")
    );
    assert_eq!(config.storage.hyper_data_dir.as_deref(), Some("/hyper"));
    assert_eq!(config.ui.format.as_deref(), Some("json"));
}

#[test]
fn apply_key_to_config_parses_bool_and_integer_fields() {
    let mut config = CliConfig::default();
    for key in [
        "codegen.clean_before_generate",
        "cache.auto_lock",
        "cache.prefer_cache",
        "ui.verbose",
        "ui.non_interactive",
    ] {
        ConfigCommand::apply_key_to_config(&mut config, key, "true")
            .unwrap_or_else(|e| panic!("apply {key}=true failed: {e}"));
        assert_eq!(
            ConfigCommand::apply_key_to_config(&mut CliConfig::default(), key, "false").unwrap(),
            ()
        );
    }
    assert_eq!(config.cache.auto_lock, Some(true));
    assert_eq!(config.ui.non_interactive, Some(true));

    // realm_id accepts both bare number and quoted string.
    ConfigCommand::apply_key_to_config(&mut config, "network.realm_id", "4242").unwrap();
    assert_eq!(config.network.realm_id, Some(4242));
    ConfigCommand::apply_key_to_config(&mut config, "network.realm_id", "\"9999\"").unwrap();
    assert_eq!(config.network.realm_id, Some(9999));
}

#[test]
fn apply_key_to_config_rejects_unknown_key_and_bad_values() {
    let mut config = CliConfig::default();
    let err = ConfigCommand::apply_key_to_config(&mut config, "nope.nope", "x").unwrap_err();
    assert!(format!("{err}").contains("Unknown configuration key 'nope.nope'"));

    let err =
        ConfigCommand::apply_key_to_config(&mut config, "cache.auto_lock", "maybe").unwrap_err();
    assert!(format!("{err}").contains("expects a boolean"));

    let err =
        ConfigCommand::apply_key_to_config(&mut config, "network.realm_id", "abc").unwrap_err();
    assert!(format!("{err}").contains("expects a positive integer"));
}

#[test]
fn value_to_bool_accepts_native_and_string_forms() {
    assert!(value_to_bool(&Value::Boolean(true), "k").unwrap());
    assert!(!value_to_bool(&Value::String("false".into()), "k").unwrap());
    assert!(value_to_bool(&Value::Integer(1), "k").is_err());
}

#[test]
fn value_to_string_extracts_inner_or_displays() {
    assert_eq!(value_to_string(&Value::String("x".into())).unwrap(), "x");
    assert_eq!(value_to_string(&Value::Integer(7)).unwrap(), "7");
}

#[test]
fn unset_key_from_config_clears_and_reports_presence() {
    let mut config = CliConfig::default();
    ConfigCommand::apply_key_to_config(&mut config, "mfr.manufacturer", "acme").unwrap();
    assert!(ConfigCommand::unset_key_from_config(&mut config, "mfr.manufacturer").unwrap());
    assert!(config.mfr.manufacturer.is_none());
    // Already unset → reports not set.
    assert!(!ConfigCommand::unset_key_from_config(&mut config, "mfr.manufacturer").unwrap());

    let err =
        ConfigCommand::unset_key_from_config(&mut CliConfig::default(), "bogus.key").unwrap_err();
    assert!(format!("{err}").contains("Unknown configuration key 'bogus.key'"));
}

#[test]
fn merge_values_overlays_tables_and_scalars() {
    let mut base = parse_toml_document_value(
        "[mfr]\nmanufacturer = \"old\"\n[network]\nrealm_id = 1\n",
        "base",
    )
    .unwrap();
    let overlay = parse_toml_document_value(
        "[mfr]\nkeychain = \"/k\"\nmanufacturer = \"new\"\n",
        "overlay",
    )
    .unwrap();
    ConfigCommand::merge_values(&mut base, overlay);
    // Overlay scalar wins, new sub-key inserted.
    assert_eq!(
        ConfigCommand::get_nested_value(&base, "mfr.manufacturer"),
        Some(&Value::String("new".into()))
    );
    assert_eq!(
        ConfigCommand::get_nested_value(&base, "mfr.keychain"),
        Some(&Value::String("/k".into()))
    );
    // Untouched table preserved.
    assert_eq!(
        ConfigCommand::get_nested_value(&base, "network.realm_id"),
        Some(&Value::Integer(1))
    );
}
