use super::*;
use crate::core::{DependencySpec, ProtoFile, ServiceDetails, ServiceInfo};

fn actr_type(s: &str) -> actr_protocol::ActrType {
    actr_protocol::ActrType::from_string_repr(s).unwrap()
}

fn service_info(name: &str, fp_val: &str, type_repr: &str) -> ServiceInfo {
    ServiceInfo {
        name: name.into(),
        tags: vec![],
        fingerprint: fp_val.into(),
        actr_type: actr_type(type_repr),
        published_at: None,
        description: None,
        methods: vec![],
    }
}

fn spec(alias: &str, name: &str, actr: Option<&str>, fp: Option<&str>) -> DependencySpec {
    DependencySpec {
        alias: alias.into(),
        name: name.into(),
        actr_type: actr.map(actr_type),
        fingerprint: fp.map(str::to_string),
    }
}

fn resolved(spec: DependencySpec, fp_val: &str) -> ResolvedDependency {
    ResolvedDependency {
        spec,
        fingerprint: fp_val.into(),
        proto_files: vec![],
    }
}

#[tokio::test]
async fn resolve_dependencies_matches_by_name_actr_type_or_falls_back() {
    let resolver = DefaultDependencyResolver::new();
    let specs = vec![
        spec("echo", "echo", None, None),
        spec("other", "different", Some("acme:Other:1.0.0"), None),
        spec("orphan", "orphan", None, Some("manual-fp")),
    ];
    let details = vec![
        ServiceDetails {
            info: service_info("echo", "echo-fp", "acme:Echo:1.0.0"),
            proto_files: vec![ProtoFile {
                name: "echo.proto".into(),
                path: "echo.proto".into(),
                content: "syntax = \"proto3\";".into(),
                services: vec![],
            }],
            dependencies: vec![],
        },
        ServiceDetails {
            info: service_info("other-service", "other-fp", "acme:Other:1.0.0"),
            proto_files: vec![],
            dependencies: vec![],
        },
    ];

    let result = resolver
        .resolve_dependencies(&specs, &details)
        .await
        .unwrap();
    assert_eq!(result.len(), 3);
    // Matched by name.
    assert_eq!(result[0].fingerprint, "echo-fp");
    assert_eq!(result[0].proto_files.len(), 1);
    // Matched by actr_type.
    assert_eq!(result[1].fingerprint, "other-fp");
    // No match → falls back to spec fingerprint, empty protos.
    assert_eq!(result[2].fingerprint, "manual-fp");
    assert!(result[2].proto_files.is_empty());
}

#[tokio::test]
async fn resolve_dependencies_no_match_uses_empty_fingerprint() {
    let resolver = DefaultDependencyResolver::new();
    let specs = vec![spec("ghost", "ghost", None, None)];
    let result = resolver.resolve_dependencies(&specs, &[]).await.unwrap();
    assert_eq!(result[0].fingerprint, "");
    assert!(result[0].proto_files.is_empty());
}

#[tokio::test]
async fn check_conflicts_reports_alias_dup_and_fingerprint_mismatch() {
    let resolver = DefaultDependencyResolver::new();
    // Same alias, different names → version conflict.
    let deps = vec![
        resolved(spec("dup", "a", None, None), "fp-a"),
        resolved(spec("dup", "b", None, None), "fp-b"),
    ];
    let conflicts = resolver.check_conflicts(&deps).await.unwrap();
    assert_eq!(conflicts.len(), 1);
    assert!(matches!(
        conflicts[0].conflict_type,
        ConflictType::VersionConflict
    ));

    // Same name, different non-empty fingerprints → fingerprint mismatch.
    let deps = vec![
        resolved(spec("alias1", "shared", None, None), "fp-1"),
        resolved(spec("alias2", "shared", None, None), "fp-2"),
    ];
    let conflicts = resolver.check_conflicts(&deps).await.unwrap();
    assert_eq!(conflicts.len(), 1);
    assert!(matches!(
        conflicts[0].conflict_type,
        ConflictType::FingerprintMismatch
    ));

    // No conflicts when deps are clean.
    let deps = vec![
        resolved(spec("a", "svc-a", None, None), "fp-a"),
        resolved(spec("b", "svc-b", None, None), "fp-b"),
    ];
    assert!(resolver.check_conflicts(&deps).await.unwrap().is_empty());
}

#[tokio::test]
async fn build_dependency_graph_collects_aliases() {
    let resolver = DefaultDependencyResolver::new();
    let deps = vec![
        resolved(spec("alpha", "a", None, None), "1"),
        resolved(spec("beta", "b", None, None), "2"),
    ];
    let graph = resolver.build_dependency_graph(&deps).await.unwrap();
    assert_eq!(graph.nodes, vec!["alpha".to_string(), "beta".to_string()]);
    assert!(graph.edges.is_empty());
    assert!(!graph.has_cycles);
}
