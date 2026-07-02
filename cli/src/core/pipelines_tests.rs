use super::*;
use crate::core::DependencySpec;

#[test]
fn dependency_lookup_key_prefers_actr_type_over_name() {
    let spec = DependencySpec {
        alias: "echo".into(),
        name: "echo-service".into(),
        actr_type: Some(actr_protocol::ActrType::from_string_repr("acme:Echo:1.0.0").unwrap()),
        fingerprint: None,
    };
    assert_eq!(
        ValidationPipeline::dependency_lookup_key(&spec),
        "acme:Echo:1.0.0"
    );

    let spec = DependencySpec {
        alias: "echo".into(),
        name: "echo-service".into(),
        actr_type: None,
        fingerprint: None,
    };
    assert_eq!(
        ValidationPipeline::dependency_lookup_key(&spec),
        "echo-service"
    );
}

#[test]
fn install_result_success_is_empty() {
    let result = InstallResult::success();
    assert!(result.installed_dependencies.is_empty());
    assert!(!result.updated_config);
    assert!(!result.updated_lock_file);
    assert_eq!(result.cache_updates, 0);
}

#[test]
fn install_result_summary_counts_deps_and_cache() {
    let result = InstallResult {
        installed_dependencies: vec![],
        updated_config: true,
        updated_lock_file: true,
        cache_updates: 5,
        warnings: vec![],
    };
    let s = result.summary();
    assert!(s.contains("Installed 0 dependencies"));
    assert!(s.contains("updated 5 cache entries"));
}

// ── mock trait implementations for pipeline testing ─────────────────

use crate::core::components::{
    ConfigManager, DependencyResolver, FingerprintValidator, NetworkValidator, ServiceDiscovery,
};
use crate::core::{ConfigValidation, NetworkCheckOptions};
use actr_config::ManifestConfig;
use async_trait::async_trait;
use std::path::Path;
use std::sync::Arc;

struct MockConfig {
    is_valid: bool,
}

#[async_trait]
impl ConfigManager for MockConfig {
    async fn load_config(&self, _path: &Path) -> anyhow::Result<ManifestConfig> {
        unreachable!()
    }
    async fn save_config(&self, _config: &ManifestConfig, _path: &Path) -> anyhow::Result<()> {
        unreachable!()
    }
    async fn update_dependency(&self, _spec: &DependencySpec) -> anyhow::Result<()> {
        Ok(())
    }
    async fn validate_config(&self) -> anyhow::Result<ConfigValidation> {
        Ok(ConfigValidation {
            is_valid: self.is_valid,
            errors: if self.is_valid {
                vec![]
            } else {
                vec!["invalid".into()]
            },
            warnings: vec![],
        })
    }
    fn get_project_root(&self) -> &Path {
        Path::new(".")
    }
    async fn backup_config(&self) -> anyhow::Result<crate::core::ConfigBackup> {
        Ok(crate::core::ConfigBackup {
            original_path: Path::new("manifest.toml").into(),
            backup_path: Path::new("manifest.toml.bak").into(),
            timestamp: std::time::SystemTime::now(),
        })
    }
    async fn restore_backup(&self, _backup: crate::core::ConfigBackup) -> anyhow::Result<()> {
        Ok(())
    }
    async fn remove_backup(&self, _backup: crate::core::ConfigBackup) -> anyhow::Result<()> {
        Ok(())
    }
}

struct MockDepResolver;
#[async_trait]
impl DependencyResolver for MockDepResolver {
    async fn resolve_spec(&self, _config: &ManifestConfig) -> anyhow::Result<Vec<DependencySpec>> {
        Ok(vec![])
    }
    async fn resolve_dependencies(
        &self,
        _specs: &[DependencySpec],
        _service_details: &[crate::core::ServiceDetails],
    ) -> anyhow::Result<Vec<ResolvedDependency>> {
        Ok(vec![])
    }
    async fn check_conflicts(
        &self,
        _deps: &[ResolvedDependency],
    ) -> anyhow::Result<Vec<crate::core::ConflictReport>> {
        Ok(vec![])
    }
    async fn build_dependency_graph(
        &self,
        _deps: &[ResolvedDependency],
    ) -> anyhow::Result<crate::core::DependencyGraph> {
        unreachable!()
    }
}

struct MockServiceDiscovery {
    service_available: bool,
}
#[async_trait]
impl ServiceDiscovery for MockServiceDiscovery {
    async fn discover_services(
        &self,
        _filter: Option<&crate::core::ServiceFilter>,
    ) -> anyhow::Result<Vec<crate::core::ServiceInfo>> {
        unreachable!()
    }
    async fn get_service_details(
        &self,
        _name: &str,
    ) -> anyhow::Result<crate::core::ServiceDetails> {
        if self.service_available {
            Ok(crate::core::ServiceDetails {
                info: crate::core::ServiceInfo {
                    name: "echo".into(),
                    tags: vec![],
                    fingerprint: "fp-echo".into(),
                    actr_type: actr_protocol::ActrType::from_string_repr("acme:Echo:1.0.0")
                        .unwrap(),
                    published_at: None,
                    description: None,
                    methods: vec![],
                },
                proto_files: vec![],
                dependencies: vec![],
            })
        } else {
            anyhow::bail!("service not found")
        }
    }
    async fn check_service_availability(
        &self,
        _name: &str,
    ) -> anyhow::Result<crate::core::AvailabilityStatus> {
        Ok(crate::core::AvailabilityStatus {
            is_available: self.service_available,
            last_seen: None,
            health: crate::core::HealthStatus::Healthy,
        })
    }
    async fn get_service_proto(&self, _name: &str) -> anyhow::Result<Vec<crate::core::ProtoFile>> {
        unreachable!()
    }
}

struct MockNetworkValidator {
    reachable: bool,
}
#[async_trait]
impl NetworkValidator for MockNetworkValidator {
    async fn check_connectivity(
        &self,
        _name: &str,
        _opts: &crate::core::NetworkCheckOptions,
    ) -> anyhow::Result<crate::core::ConnectivityStatus> {
        Ok(crate::core::ConnectivityStatus {
            is_reachable: self.reachable,
            response_time_ms: if self.reachable { Some(5) } else { None },
            error: if self.reachable {
                None
            } else {
                Some("unreachable".into())
            },
        })
    }
    async fn verify_service_health(
        &self,
        _name: &str,
        _opts: &crate::core::NetworkCheckOptions,
    ) -> anyhow::Result<crate::core::HealthStatus> {
        unreachable!()
    }
    async fn test_latency(
        &self,
        _name: &str,
        _opts: &crate::core::NetworkCheckOptions,
    ) -> anyhow::Result<crate::core::LatencyInfo> {
        unreachable!()
    }
    async fn batch_check(
        &self,
        names: &[String],
        _opts: &crate::core::NetworkCheckOptions,
    ) -> anyhow::Result<Vec<crate::core::NetworkCheckResult>> {
        Ok(names
            .iter()
            .map(|_| crate::core::NetworkCheckResult {
                connectivity: crate::core::ConnectivityStatus {
                    is_reachable: self.reachable,
                    response_time_ms: if self.reachable { Some(5) } else { None },
                    error: if self.reachable {
                        None
                    } else {
                        Some("unreachable".into())
                    },
                },
                health: if self.reachable {
                    crate::core::HealthStatus::Healthy
                } else {
                    crate::core::HealthStatus::Unhealthy
                },
                latency: None,
            })
            .collect())
    }
}

struct MockFingerprintValidator;
#[async_trait]
impl FingerprintValidator for MockFingerprintValidator {
    async fn compute_service_fingerprint(
        &self,
        svc: &crate::core::ServiceInfo,
    ) -> anyhow::Result<crate::core::Fingerprint> {
        Ok(crate::core::Fingerprint {
            algorithm: "sha256".into(),
            value: svc.fingerprint.clone(),
        })
    }
    async fn verify_fingerprint(
        &self,
        expected: &crate::core::Fingerprint,
        actual: &crate::core::Fingerprint,
    ) -> anyhow::Result<bool> {
        Ok(expected.algorithm == actual.algorithm && expected.value == actual.value)
    }
    async fn compute_project_fingerprint(
        &self,
        _path: &Path,
    ) -> anyhow::Result<crate::core::Fingerprint> {
        unreachable!()
    }
    async fn generate_lock_fingerprint(
        &self,
        _deps: &[ResolvedDependency],
    ) -> anyhow::Result<crate::core::Fingerprint> {
        unreachable!()
    }
}

struct MockCacheManager;
#[async_trait]
impl crate::core::CacheManager for MockCacheManager {
    async fn get_cached_proto(
        &self,
        _name: &str,
    ) -> anyhow::Result<Option<crate::core::CachedProto>> {
        unreachable!()
    }
    async fn cache_proto(
        &self,
        _name: &str,
        _protos: &[crate::core::ProtoFile],
    ) -> anyhow::Result<()> {
        Ok(())
    }
    async fn invalidate_cache(&self, _name: &str) -> anyhow::Result<()> {
        unreachable!()
    }
    async fn clear_cache(&self) -> anyhow::Result<()> {
        unreachable!()
    }
    async fn get_cache_stats(&self) -> anyhow::Result<crate::core::CacheStats> {
        unreachable!()
    }
}

struct MockProtoProcessor {
    proto_is_valid: bool,
}
#[async_trait]
impl crate::core::ProtoProcessor for MockProtoProcessor {
    async fn discover_proto_files(
        &self,
        _path: &Path,
    ) -> anyhow::Result<Vec<crate::core::ProtoFile>> {
        Ok(vec![])
    }
    async fn parse_proto_services(
        &self,
        _files: &[crate::core::ProtoFile],
    ) -> anyhow::Result<Vec<crate::core::ServiceDefinition>> {
        unreachable!()
    }
    async fn generate_code(
        &self,
        _input: &Path,
        output: &Path,
    ) -> anyhow::Result<crate::core::GenerationResult> {
        Ok(crate::core::GenerationResult {
            generated_files: vec![output.to_path_buf()],
            warnings: vec![],
            errors: vec![],
        })
    }
    async fn validate_proto_syntax(
        &self,
        _files: &[crate::core::ProtoFile],
    ) -> anyhow::Result<crate::core::ValidationReport> {
        Ok(crate::core::ValidationReport {
            is_valid: self.proto_is_valid,
            config_validation: crate::core::ConfigValidation {
                is_valid: true,
                errors: vec![],
                warnings: vec![],
            },
            dependency_validation: vec![],
            network_validation: vec![],
            fingerprint_validation: vec![],
            conflicts: vec![],
        })
    }
}

#[test]
fn validation_pipeline_constructs_and_exposes_getters() {
    let config: Arc<dyn ConfigManager> = Arc::new(MockConfig { is_valid: true });
    let dep: Arc<dyn DependencyResolver> = Arc::new(MockDepResolver);
    let sd: Arc<dyn ServiceDiscovery> = Arc::new(MockServiceDiscovery {
        service_available: true,
    });
    let net: Arc<dyn NetworkValidator> = Arc::new(MockNetworkValidator { reachable: true });
    let fp: Arc<dyn FingerprintValidator> = Arc::new(MockFingerprintValidator);

    let pipeline = ValidationPipeline::new(
        config.clone(),
        dep.clone(),
        sd.clone(),
        net.clone(),
        fp.clone(),
    );
    // Getters return the stored Arcs.
    let _ = pipeline.config_manager();
    let _ = pipeline.dependency_resolver();
    let _ = pipeline.service_discovery();
    let _ = pipeline.network_validator();
}

#[tokio::test]
async fn validate_project_returns_early_on_invalid_config() {
    let config: Arc<dyn ConfigManager> = Arc::new(MockConfig { is_valid: false });
    let dep: Arc<dyn DependencyResolver> = Arc::new(MockDepResolver);
    let sd: Arc<dyn ServiceDiscovery> = Arc::new(MockServiceDiscovery {
        service_available: true,
    });
    let net: Arc<dyn NetworkValidator> = Arc::new(MockNetworkValidator { reachable: true });
    let fp: Arc<dyn FingerprintValidator> = Arc::new(MockFingerprintValidator);

    let pipeline = ValidationPipeline::new(config, dep, sd, net, fp);
    let report = pipeline.validate_project().await.unwrap();
    assert!(!report.is_valid);
    assert!(!report.config_validation.is_valid);
    assert!(
        report
            .config_validation
            .errors
            .contains(&"invalid".to_string())
    );
}

#[tokio::test]
async fn validate_dependencies_reports_service_availability() {
    let config: Arc<dyn ConfigManager> = Arc::new(MockConfig { is_valid: true });
    let dep: Arc<dyn DependencyResolver> = Arc::new(MockDepResolver);
    let sd: Arc<dyn ServiceDiscovery> = Arc::new(MockServiceDiscovery {
        service_available: true,
    });
    let net: Arc<dyn NetworkValidator> = Arc::new(MockNetworkValidator { reachable: true });
    let fp: Arc<dyn FingerprintValidator> = Arc::new(MockFingerprintValidator);
    let pipeline = ValidationPipeline::new(config, dep, sd, net, fp);

    let specs = vec![
        DependencySpec {
            alias: "echo".into(),
            name: "echo-service".into(),
            actr_type: None,
            fingerprint: None,
        },
        DependencySpec {
            alias: "other".into(),
            name: "other-service".into(),
            actr_type: None,
            fingerprint: None,
        },
    ];
    let results = pipeline.validate_dependencies(&specs).await.unwrap();
    assert_eq!(results.len(), 2);
    assert!(results[0].is_available);
    assert!(results[1].is_available);
}

#[tokio::test]
async fn validate_network_connectivity_maps_reachable_and_unreachable() {
    let config: Arc<dyn ConfigManager> = Arc::new(MockConfig { is_valid: true });
    let dep: Arc<dyn DependencyResolver> = Arc::new(MockDepResolver);
    let sd: Arc<dyn ServiceDiscovery> = Arc::new(MockServiceDiscovery {
        service_available: true,
    });
    let fp: Arc<dyn FingerprintValidator> = Arc::new(MockFingerprintValidator);

    let deps = vec![ResolvedDependency {
        spec: DependencySpec {
            alias: "echo".into(),
            name: "echo".into(),
            actr_type: None,
            fingerprint: None,
        },
        fingerprint: "".into(),
        proto_files: vec![],
    }];

    // Reachable.
    let net_r: Arc<dyn NetworkValidator> = Arc::new(MockNetworkValidator { reachable: true });
    let pipeline =
        ValidationPipeline::new(config.clone(), dep.clone(), sd.clone(), net_r, fp.clone());
    let results = pipeline
        .validate_network_connectivity(&deps, &NetworkCheckOptions::default())
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
    assert!(results[0].is_reachable);

    // Unreachable.
    let net_u: Arc<dyn NetworkValidator> = Arc::new(MockNetworkValidator { reachable: false });
    let pipeline2 = ValidationPipeline::new(config, dep, sd, net_u, fp);
    let results2 = pipeline2
        .validate_network_connectivity(&deps, &NetworkCheckOptions::default())
        .await
        .unwrap();
    assert!(!results2[0].is_reachable);
}

#[tokio::test]
async fn validate_fingerprints_covers_present_and_missing_fingerprints() {
    let config: Arc<dyn ConfigManager> = Arc::new(MockConfig { is_valid: true });
    let dep: Arc<dyn DependencyResolver> = Arc::new(MockDepResolver);
    let sd: Arc<dyn ServiceDiscovery> = Arc::new(MockServiceDiscovery {
        service_available: true,
    });
    let net: Arc<dyn NetworkValidator> = Arc::new(MockNetworkValidator { reachable: true });
    let fp: Arc<dyn FingerprintValidator> = Arc::new(MockFingerprintValidator);

    let deps = vec![
        // Dep with matching (pre-computed) fingerprint → passes without recompute.
        ResolvedDependency {
            spec: DependencySpec {
                alias: "a".into(),
                name: "a".into(),
                actr_type: None,
                fingerprint: Some("fp-a".into()),
            },
            fingerprint: "fp-a".into(),
            proto_files: vec![],
        },
        // Dep with empty computed fingerprint → fetches from service discovery,
        // then verifies against spec fingerprint (mismatch → invalid).
        ResolvedDependency {
            spec: DependencySpec {
                alias: "b".into(),
                name: "b".into(),
                actr_type: None,
                fingerprint: Some("fp-b".into()),
            },
            fingerprint: "".into(),
            proto_files: vec![],
        },
        // No spec fingerprint → empty expected → is_valid = true.
        ResolvedDependency {
            spec: DependencySpec {
                alias: "c".into(),
                name: "c".into(),
                actr_type: None,
                fingerprint: None,
            },
            fingerprint: "fp-c".into(),
            proto_files: vec![],
        },
    ];

    let pipeline = ValidationPipeline::new(config, dep, sd, net, fp);
    let results = pipeline.validate_fingerprints(&deps).await.unwrap();
    assert_eq!(results.len(), 3);
    // a: pre-computed matches → valid.
    assert!(results[0].is_valid);
    // b: fetched from service discovery (fingerprint="fp-echo"), spec expects "fp-b" → mismatch.
    assert!(!results[1].is_valid);
    // c: expected empty → valid regardless.
    assert!(results[2].is_valid);
}

#[test]
fn install_and_generation_pipelines_construct() {
    let config: Arc<dyn ConfigManager> = Arc::new(MockConfig { is_valid: true });
    let dep: Arc<dyn DependencyResolver> = Arc::new(MockDepResolver);
    let sd: Arc<dyn ServiceDiscovery> = Arc::new(MockServiceDiscovery {
        service_available: true,
    });
    let net: Arc<dyn NetworkValidator> = Arc::new(MockNetworkValidator { reachable: true });
    let fp: Arc<dyn FingerprintValidator> = Arc::new(MockFingerprintValidator);
    let cache: Arc<dyn crate::core::CacheManager> = Arc::new(MockCacheManager);
    let proto: Arc<dyn crate::core::ProtoProcessor> = Arc::new(MockProtoProcessor {
        proto_is_valid: true,
    });

    let vp = ValidationPipeline::new(
        config.clone(),
        dep.clone(),
        sd.clone(),
        net.clone(),
        fp.clone(),
    );
    let ip = InstallPipeline::new(vp, config.clone(), cache.clone(), proto.clone());
    let _ = ip.validation_pipeline();
    let _ = ip.config_manager();

    std::mem::drop(GenerationPipeline::new(config, proto, cache));
}

#[tokio::test]
async fn install_dependencies_reports_failed_validation() {
    let config: Arc<dyn ConfigManager> = Arc::new(MockConfig { is_valid: true });
    let dep: Arc<dyn DependencyResolver> = Arc::new(MockDepResolver);
    // Service unavailable → validate_dependencies returns is_available=false.
    let sd: Arc<dyn ServiceDiscovery> = Arc::new(MockServiceDiscovery {
        service_available: false,
    });
    let net: Arc<dyn NetworkValidator> = Arc::new(MockNetworkValidator { reachable: true });
    let fp: Arc<dyn FingerprintValidator> = Arc::new(MockFingerprintValidator);
    let cache: Arc<dyn crate::core::CacheManager> = Arc::new(MockCacheManager);
    let proto: Arc<dyn crate::core::ProtoProcessor> = Arc::new(MockProtoProcessor {
        proto_is_valid: true,
    });

    let vp = ValidationPipeline::new(
        config.clone(),
        dep.clone(),
        sd.clone(),
        net.clone(),
        fp.clone(),
    );
    let ip = InstallPipeline::new(vp, config, cache, proto);

    let specs = vec![DependencySpec {
        alias: "echo".into(),
        name: "echo".into(),
        actr_type: None,
        fingerprint: None,
    }];
    let err = ip.install_dependencies(&specs).await.unwrap_err();
    assert!(format!("{err}").contains("Dependency validation failed"));
}

#[tokio::test]
async fn install_dependencies_succeeds_with_empty_specs() {
    let config: Arc<dyn ConfigManager> = Arc::new(MockConfig { is_valid: true });
    let dep: Arc<dyn DependencyResolver> = Arc::new(MockDepResolver);
    let sd: Arc<dyn ServiceDiscovery> = Arc::new(MockServiceDiscovery {
        service_available: true,
    });
    let net: Arc<dyn NetworkValidator> = Arc::new(MockNetworkValidator { reachable: true });
    let fp: Arc<dyn FingerprintValidator> = Arc::new(MockFingerprintValidator);
    let cache: Arc<dyn crate::core::CacheManager> = Arc::new(MockCacheManager);
    let proto: Arc<dyn crate::core::ProtoProcessor> = Arc::new(MockProtoProcessor {
        proto_is_valid: true,
    });

    let vp = ValidationPipeline::new(
        config.clone(),
        dep.clone(),
        sd.clone(),
        net.clone(),
        fp.clone(),
    );
    let ip = InstallPipeline::new(vp, config, cache, proto);
    let result = ip.install_dependencies(&[]).await.unwrap();
    assert!(result.installed_dependencies.is_empty());
}

#[tokio::test]
async fn generation_pipeline_rejects_invalid_proto_syntax() {
    let config: Arc<dyn ConfigManager> = Arc::new(MockConfig { is_valid: true });
    let proto_invalid: Arc<dyn crate::core::ProtoProcessor> = Arc::new(MockProtoProcessor {
        proto_is_valid: false,
    });
    let cache: Arc<dyn crate::core::CacheManager> = Arc::new(MockCacheManager);
    let gp = GenerationPipeline::new(config, proto_invalid, cache);
    let options = crate::core::pipelines::GenerationOptions {
        input_path: Path::new("protos").to_path_buf(),
        output_path: Path::new("out").to_path_buf(),
        clean_before_generate: false,
        generate_scaffold: false,
        format_code: false,
        run_checks: false,
    };
    let err = gp.generate_code(&options).await.unwrap_err();
    assert!(format!("{err}").contains("Proto file syntax validation failed"));
}

#[tokio::test]
async fn generation_pipeline_succeeds_with_valid_proto() {
    let config: Arc<dyn ConfigManager> = Arc::new(MockConfig { is_valid: true });
    let proto_valid: Arc<dyn crate::core::ProtoProcessor> = Arc::new(MockProtoProcessor {
        proto_is_valid: true,
    });
    let cache: Arc<dyn crate::core::CacheManager> = Arc::new(MockCacheManager);
    let gp = GenerationPipeline::new(config, proto_valid, cache);
    let options = crate::core::pipelines::GenerationOptions {
        input_path: Path::new("protos").to_path_buf(),
        output_path: Path::new("out").to_path_buf(),
        clean_before_generate: true,
        generate_scaffold: false,
        format_code: false,
        run_checks: false,
    };
    let result = gp.generate_code(&options).await.unwrap();
    assert_eq!(result.generated_files, vec![Path::new("out").to_path_buf()]);
}
