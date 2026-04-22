use actr_config::ManifestConfig;
use anyhow::Result;
use async_trait::async_trait;

use super::{
    ConflictReport, ConflictType, DependencyGraph, DependencyResolver, DependencySpec,
    ResolvedDependency, ServiceDetails,
};

pub struct DefaultDependencyResolver;

impl DefaultDependencyResolver {
    pub fn new() -> Self {
        Self
    }
}

impl Default for DefaultDependencyResolver {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl DependencyResolver for DefaultDependencyResolver {
    async fn resolve_spec(&self, config: &ManifestConfig) -> Result<Vec<DependencySpec>> {
        let specs: Vec<DependencySpec> = config
            .dependencies
            .iter()
            .map(|dependency| DependencySpec {
                alias: dependency.alias.clone(),
                name: dependency
                    .service
                    .as_ref()
                    .map(|service| service.name.clone())
                    .unwrap_or_else(|| dependency.alias.clone()),
                actr_type: dependency.actr_type.clone(),
                fingerprint: dependency
                    .service
                    .as_ref()
                    .map(|service| service.fingerprint.clone()),
            })
            .collect();

        Ok(specs)
    }

    async fn resolve_dependencies(
        &self,
        specs: &[DependencySpec],
        service_details: &[ServiceDetails],
    ) -> Result<Vec<ResolvedDependency>> {
        let mut resolved = Vec::with_capacity(specs.len());

        for spec in specs {
            // Find matching service details
            let matching_details = service_details.iter().find(|details| {
                details.info.name == spec.name
                    || details.info.actr_type.to_string_repr() == spec.name
                    || spec
                        .actr_type
                        .as_ref()
                        .is_some_and(|ty| details.info.actr_type == *ty)
            });

            let (fingerprint, proto_files) = match matching_details {
                Some(details) => (
                    details.info.fingerprint.clone(),
                    details.proto_files.clone(),
                ),
                None => (spec.fingerprint.clone().unwrap_or_default(), Vec::new()),
            };

            resolved.push(ResolvedDependency {
                spec: spec.clone(),
                fingerprint,
                proto_files,
            });
        }

        Ok(resolved)
    }

    async fn check_conflicts(&self, deps: &[ResolvedDependency]) -> Result<Vec<ConflictReport>> {
        let mut conflicts = Vec::new();

        for i in 0..deps.len() {
            for j in (i + 1)..deps.len() {
                // Conflict if same alias is used
                if deps[i].spec.alias == deps[j].spec.alias {
                    // Same alias is always a conflict if they point to different things
                    if deps[i].spec.name != deps[j].spec.name
                        || deps[i].fingerprint != deps[j].fingerprint
                    {
                        conflicts.push(ConflictReport {
                            dependency_a: deps[i].spec.alias.clone(),
                            dependency_b: deps[j].spec.alias.clone(),
                            conflict_type: ConflictType::VersionConflict,
                            description: format!(
                                "Dependency alias '{}' is duplicated with different targets",
                                deps[i].spec.alias
                            ),
                        });
                        continue;
                    }
                }

                // Conflict if same package name has different fingerprints
                if deps[i].spec.name != deps[j].spec.name {
                    continue;
                }

                if !deps[i].fingerprint.is_empty()
                    && !deps[j].fingerprint.is_empty()
                    && deps[i].fingerprint != deps[j].fingerprint
                {
                    conflicts.push(ConflictReport {
                        dependency_a: format!("{} ({})", deps[i].spec.name, deps[i].spec.alias),
                        dependency_b: format!("{} ({})", deps[j].spec.name, deps[j].spec.alias),
                        conflict_type: ConflictType::FingerprintMismatch,
                        description: format!(
                            "Dependency {} has conflicting fingerprints",
                            deps[i].spec.name
                        ),
                    });
                }
            }
        }

        Ok(conflicts)
    }

    async fn build_dependency_graph(&self, deps: &[ResolvedDependency]) -> Result<DependencyGraph> {
        let nodes: Vec<String> = deps.iter().map(|d| d.spec.alias.clone()).collect();

        Ok(DependencyGraph {
            nodes,
            edges: Vec::new(),
            has_cycles: false,
        })
    }
}
