use super::*;

#[test]
fn user_message_formats_known_and_fallback_variants() {
    assert!(
        ActrCliError::Config {
            message: "bad".into()
        }
        .user_message()
        .contains("Config file error: bad")
    );
    assert!(
        ActrCliError::Network {
            message: "down".into()
        }
        .user_message()
        .contains("Network connection error: down")
    );
    assert!(
        ActrCliError::Dependency {
            message: "missing".into()
        }
        .user_message()
        .contains("Dependency error: missing")
    );
    assert!(
        ActrCliError::ValidationFailed {
            details: "x".into()
        }
        .user_message()
        .contains("Validation failed: x")
    );
    assert!(
        ActrCliError::InstallFailed { reason: "r".into() }
            .user_message()
            .contains("Install failed: r")
    );
    // Fallback branch falls through to Display.
    assert!(
        ActrCliError::ServiceNotFound { name: "svc".into() }
            .user_message()
            .contains("Service not found: svc")
    );
    assert_eq!(
        ActrCliError::OperationCancelled.user_message(),
        "Operation cancelled"
    );
}

#[test]
fn suggested_actions_counts_match_per_variant() {
    assert_eq!(
        ActrCliError::Config {
            message: "x".into()
        }
        .suggested_actions()
        .len(),
        3
    );
    assert_eq!(
        ActrCliError::Network {
            message: "x".into()
        }
        .suggested_actions()
        .len(),
        4
    );
    assert_eq!(
        ActrCliError::Dependency {
            message: "x".into()
        }
        .suggested_actions()
        .len(),
        3
    );
    assert_eq!(
        ActrCliError::ValidationFailed {
            details: "x".into()
        }
        .suggested_actions()
        .len(),
        3
    );
    assert_eq!(
        ActrCliError::InstallFailed { reason: "x".into() }
            .suggested_actions()
            .len(),
        4
    );
    assert_eq!(
        ActrCliError::OperationCancelled.suggested_actions().len(),
        1
    );
}

#[test]
fn documentation_links_for_config_dependency_and_default() {
    assert_eq!(
        ActrCliError::Config {
            message: "x".into()
        }
        .documentation_links()
        .len(),
        2
    );
    assert_eq!(
        ActrCliError::Dependency {
            message: "x".into()
        }
        .documentation_links()
        .len(),
        2
    );
    assert_eq!(
        ActrCliError::OperationCancelled.documentation_links().len(),
        1
    );
}

#[test]
fn format_error_renders_message_suggestions_and_docs() {
    let formatted = ErrorReporter::format_error(&ActrCliError::Config {
        message: "boom".into(),
    });
    assert!(formatted.contains("Config file error: boom"));
    assert!(formatted.contains("Suggested solutions"));
    assert!(formatted.contains("Related documentation"));
}

#[test]
fn format_validation_report_renders_all_sections_and_converts_to_error() {
    use crate::core::components::Fingerprint as Fp;
    use crate::core::components::{
        ConfigValidation, ConflictReport, ConflictType, DependencyValidation,
        FingerprintValidation, HealthStatus, NetworkValidation, ValidationReport,
    };

    let failing = ValidationReport {
        is_valid: false,
        config_validation: ConfigValidation {
            is_valid: false,
            errors: vec!["bad syntax".into()],
            warnings: vec![],
        },
        dependency_validation: vec![
            DependencyValidation {
                dependency: "dep-a".into(),
                is_available: true,
                error: None,
            },
            DependencyValidation {
                dependency: "dep-b".into(),
                is_available: false,
                error: Some("offline".into()),
            },
        ],
        network_validation: vec![
            NetworkValidation {
                is_reachable: true,
                health: HealthStatus::Healthy,
                latency_ms: Some(12),
                error: None,
                is_applicable: true,
            },
            NetworkValidation {
                is_reachable: false,
                health: HealthStatus::Unhealthy,
                latency_ms: None,
                error: Some("timeout".into()),
                is_applicable: true,
            },
        ],
        fingerprint_validation: vec![FingerprintValidation {
            dependency: "dep-a".into(),
            expected: Fp {
                algorithm: "sha".into(),
                value: "abc".into(),
            },
            actual: None,
            is_valid: false,
            error: Some("mismatch".into()),
        }],
        conflicts: vec![ConflictReport {
            dependency_a: "dep-a".into(),
            dependency_b: "dep-b".into(),
            conflict_type: ConflictType::VersionConflict,
            description: "versions clash".into(),
        }],
    };

    let out = ErrorReporter::format_validation_report(&failing);
    assert!(out.contains("❌ Failed"));
    assert!(out.contains("bad syntax"));
    assert!(out.contains("dep-b - offline"));
    assert!(out.contains("Connected (12ms)"));
    assert!(out.contains("Connection failed - timeout"));
    assert!(out.contains("dep-a - mismatch"));
    assert!(out.contains("dep-a vs dep-b: versions clash"));
    assert!(out.contains("Issues need to be resolved"));

    let err = ActrCliError::from(failing);
    let ActrCliError::ValidationFailed { details } = err else {
        panic!("expected ValidationFailed, got {err:?}");
    };
    assert!(details.contains("Config error: bad syntax"));
    assert!(details.contains("Dependency unavailable"));
    assert!(details.contains("Network unreachable"));
    assert!(details.contains("Fingerprint validation failed"));
    assert!(details.contains("Dependency conflict"));

    let success = ValidationReport {
        is_valid: true,
        config_validation: ConfigValidation {
            is_valid: true,
            errors: vec![],
            warnings: vec![],
        },
        dependency_validation: vec![],
        network_validation: vec![],
        fingerprint_validation: vec![],
        conflicts: vec![],
    };
    assert!(success.is_success());
    let ok_out = ErrorReporter::format_validation_report(&success);
    assert!(ok_out.contains("✅ Passed"));
    assert!(ok_out.contains("All validations passed"));
}
