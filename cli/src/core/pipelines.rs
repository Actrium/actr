//! 操作管道定义
//!
//! 定义了三个核心操作管道，实现命令间的逻辑复用

use actr_config::{LockFile, LockedDependency, ProtoFileMeta, ServiceSpecMeta};
use actr_protocol::ActrTypeExt;
use anyhow::Result;
use std::sync::Arc;

use super::components::*;

// ============================================================================
// 管道结果类型
// ============================================================================

/// 安装结果
#[derive(Debug, Clone)]
pub struct InstallResult {
    pub installed_dependencies: Vec<ResolvedDependency>,
    pub updated_config: bool,
    pub updated_lock_file: bool,
    pub cache_updates: usize,
    pub warnings: Vec<String>,
}

impl InstallResult {
    pub fn success() -> Self {
        Self {
            installed_dependencies: Vec::new(),
            updated_config: false,
            updated_lock_file: false,
            cache_updates: 0,
            warnings: Vec::new(),
        }
    }

    pub fn summary(&self) -> String {
        format!(
            "Installed {} dependencies, updated {} cache entries",
            self.installed_dependencies.len(),
            self.cache_updates
        )
    }
}

/// 安装计划
#[derive(Debug, Clone)]
pub struct InstallPlan {
    pub dependencies_to_install: Vec<DependencySpec>,
    pub resolved_dependencies: Vec<ResolvedDependency>,
    pub estimated_cache_size: u64,
    pub required_permissions: Vec<String>,
}

/// 生成选项
#[derive(Debug, Clone)]
pub struct GenerationOptions {
    pub input_path: std::path::PathBuf,
    pub output_path: std::path::PathBuf,
    pub clean_before_generate: bool,
    pub generate_scaffold: bool,
    pub format_code: bool,
    pub run_checks: bool,
}

// ============================================================================
// 1. 验证管道 (ValidationPipeline)
// ============================================================================

/// 核心验证管道 - 被多个命令复用
#[derive(Clone)]
pub struct ValidationPipeline {
    config_manager: Arc<dyn ConfigManager>,
    dependency_resolver: Arc<dyn DependencyResolver>,
    service_discovery: Arc<dyn ServiceDiscovery>,
    network_validator: Arc<dyn NetworkValidator>,
    fingerprint_validator: Arc<dyn FingerprintValidator>,
}

impl ValidationPipeline {
    pub fn new(
        config_manager: Arc<dyn ConfigManager>,
        dependency_resolver: Arc<dyn DependencyResolver>,
        service_discovery: Arc<dyn ServiceDiscovery>,
        network_validator: Arc<dyn NetworkValidator>,
        fingerprint_validator: Arc<dyn FingerprintValidator>,
    ) -> Self {
        Self {
            config_manager,
            dependency_resolver,
            service_discovery,
            network_validator,
            fingerprint_validator,
        }
    }

    /// Get service discovery component
    pub fn service_discovery(&self) -> &Arc<dyn ServiceDiscovery> {
        &self.service_discovery
    }

    /// Get network validator component
    pub fn network_validator(&self) -> &Arc<dyn NetworkValidator> {
        &self.network_validator
    }

    /// Get config manager component
    pub fn config_manager(&self) -> &Arc<dyn ConfigManager> {
        &self.config_manager
    }

    fn dependency_lookup_key(spec: &DependencySpec) -> String {
        spec.actr_type
            .as_ref()
            .map(|actr_type| actr_type.to_string_repr())
            .unwrap_or_else(|| spec.name.clone())
    }

    /// 完整的项目验证流程
    pub async fn validate_project(&self) -> Result<ValidationReport> {
        // 1. 配置文件验证
        let config_validation = self.config_manager.validate_config().await?;

        // 如果配置文件都有问题，直接返回
        if !config_validation.is_valid {
            return Ok(ValidationReport {
                is_valid: false,
                config_validation,
                dependency_validation: vec![],
                network_validation: vec![],
                fingerprint_validation: vec![],
                conflicts: vec![],
            });
        }

        // 2. 依赖解析和验证
        let config = self
            .config_manager
            .load_config(
                self.config_manager
                    .get_project_root()
                    .join("actr.toml")
                    .as_path(),
            )
            .await?;
        let dependency_specs = self.dependency_resolver.resolve_spec(&config).await?;

        let mut service_details = Vec::new();
        for spec in &dependency_specs {
            let lookup_key = Self::dependency_lookup_key(spec);
            match self
                .service_discovery
                .get_service_details(&lookup_key)
                .await
            {
                Ok(details) => service_details.push(details),
                Err(_) => {
                    // Service might not be available, continue without details
                }
            }
        }

        let resolved_dependencies = self
            .dependency_resolver
            .resolve_dependencies(&dependency_specs, &service_details)
            .await?;

        // 3. 冲突检查
        let conflicts = self
            .dependency_resolver
            .check_conflicts(&resolved_dependencies)
            .await?;

        let dependency_validation = self.validate_dependencies(&dependency_specs).await?;
        let network_validation = self
            .validate_network_connectivity(&resolved_dependencies, &NetworkCheckOptions::default())
            .await?;
        let fingerprint_validation = self.validate_fingerprints(&resolved_dependencies).await?;

        let is_valid = config_validation.is_valid
            && dependency_validation.iter().all(|d| d.is_available)
            && network_validation
                .iter()
                .all(|n| !n.is_applicable || n.is_reachable)
            && fingerprint_validation.iter().all(|f| f.is_valid)
            && conflicts.is_empty();

        Ok(ValidationReport {
            is_valid,
            config_validation,
            dependency_validation,
            network_validation,
            fingerprint_validation,
            conflicts,
        })
    }

    /// 验证特定依赖列表
    /// Note: Multiple aliases pointing to the same service name will be deduplicated
    pub async fn validate_dependencies(
        &self,
        specs: &[DependencySpec],
    ) -> Result<Vec<DependencyValidation>> {
        use std::collections::HashMap;

        let mut results = Vec::new();
        // Cache validation results by service name to avoid duplicate checks
        let mut validation_cache: HashMap<String, (bool, Option<String>)> = HashMap::new();

        for spec in specs {
            let lookup_key = Self::dependency_lookup_key(spec);
            // Check cache first - if we already validated this service name, reuse the result
            let (is_available, error) = if let Some(cached) = validation_cache.get(&lookup_key) {
                cached.clone()
            } else {
                // Perform validation
                let (available, err) = match self
                    .service_discovery
                    .check_service_availability(&lookup_key)
                    .await
                {
                    Ok(status) => {
                        if status.is_available {
                            (true, None)
                        } else {
                            // Provide meaningful error when service is not found
                            (
                                false,
                                Some(format!("Service '{}' not found in registry", lookup_key)),
                            )
                        }
                    }
                    Err(e) => (false, Some(e.to_string())),
                };

                // Cache the result for this service name
                validation_cache.insert(lookup_key, (available, err.clone()));
                (available, err)
            };

            results.push(DependencyValidation {
                dependency: spec.alias.clone(),
                is_available,
                error,
            });
        }

        Ok(results)
    }

    /// 网络连通性验证
    pub async fn validate_network_connectivity(
        &self,
        deps: &[ResolvedDependency],
        options: &NetworkCheckOptions,
    ) -> Result<Vec<NetworkValidation>> {
        let names = deps.iter().map(|d| d.spec.name.clone()).collect::<Vec<_>>();
        let network_results = self.network_validator.batch_check(&names, options).await?;

        Ok(network_results
            .into_iter()
            .map(|result| {
                let mut is_applicable = true;
                let mut error = result.connectivity.error;
                let mut health = result.health;
                let mut latency_ms = result.connectivity.response_time_ms;

                if let Some(ref message) = error
                    && message.starts_with("Address resolution failed: Invalid address format")
                {
                    is_applicable = false;
                    error =
                        Some("Network check skipped: no endpoint address available".to_string());
                    health = HealthStatus::Unknown;
                    latency_ms = None;
                }

                NetworkValidation {
                    is_reachable: result.connectivity.is_reachable,
                    health,
                    latency_ms,
                    error,
                    is_applicable,
                }
            })
            .collect())
    }

    /// 指纹验证
    pub async fn validate_fingerprints(
        &self,
        deps: &[ResolvedDependency],
    ) -> Result<Vec<FingerprintValidation>> {
        let mut results = Vec::new();

        for dep in deps {
            let expected_val = dep.spec.fingerprint.clone().unwrap_or_default();
            let expected = Fingerprint {
                algorithm: "sha256".to_string(),
                value: expected_val,
            };

            // 计算实际指纹（如果 resolved_dependencies 中没有指纹，从远程获取）
            let actual_fp = if dep.fingerprint.is_empty() {
                let lookup_key = Self::dependency_lookup_key(&dep.spec);
                match self
                    .service_discovery
                    .get_service_details(&lookup_key)
                    .await
                {
                    Ok(details) => {
                        let computed = self
                            .fingerprint_validator
                            .compute_service_fingerprint(&details.info)
                            .await?;
                        Some(computed)
                    }
                    Err(e) => {
                        results.push(FingerprintValidation {
                            dependency: dep.spec.alias.clone(),
                            expected,
                            actual: None,
                            is_valid: false,
                            error: Some(e.to_string()),
                        });
                        continue;
                    }
                }
            } else {
                // 已有指纹，无需重新计算
                None
            };

            let is_valid = if expected.value.is_empty() {
                true
            } else if let Some(ref computed) = actual_fp {
                self.fingerprint_validator
                    .verify_fingerprint(&expected, computed)
                    .await
                    .unwrap_or(false)
            } else {
                // 指纹已匹配（来自 resolve_dependencies）
                true
            };

            results.push(FingerprintValidation {
                dependency: dep.spec.alias.clone(),
                expected,
                actual: actual_fp,
                is_valid,
                error: None,
            });
        }

        Ok(results)
    }
}

// ============================================================================
// 2. 安装管道 (InstallPipeline)
// ============================================================================

/// 安装管道 - 基于ValidationPipeline构建
pub struct InstallPipeline {
    validation_pipeline: ValidationPipeline,
    config_manager: Arc<dyn ConfigManager>,
    cache_manager: Arc<dyn CacheManager>,
    #[allow(dead_code)]
    proto_processor: Arc<dyn ProtoProcessor>,
}

impl InstallPipeline {
    pub fn new(
        validation_pipeline: ValidationPipeline,
        config_manager: Arc<dyn ConfigManager>,
        cache_manager: Arc<dyn CacheManager>,
        proto_processor: Arc<dyn ProtoProcessor>,
    ) -> Self {
        Self {
            validation_pipeline,
            config_manager,
            cache_manager,
            proto_processor,
        }
    }

    /// Get validation pipeline reference
    pub fn validation_pipeline(&self) -> &ValidationPipeline {
        &self.validation_pipeline
    }

    /// Get config manager reference
    pub fn config_manager(&self) -> &Arc<dyn ConfigManager> {
        &self.config_manager
    }

    /// Check-First 安装流程
    pub async fn install_dependencies(&self, specs: &[DependencySpec]) -> Result<InstallResult> {
        // 🔍 阶段1: 完整验证 (复用ValidationPipeline)
        let validation_report = self
            .validation_pipeline
            .validate_dependencies(specs)
            .await?;

        // 检查验证结果
        let failed_validations: Vec<_> = validation_report
            .iter()
            .filter(|v| !v.is_available)
            .collect();

        if !failed_validations.is_empty() {
            return Err(anyhow::anyhow!(
                "依赖验证失败: {}",
                failed_validations
                    .iter()
                    .map(|v| format!(
                        "{}: {}",
                        v.dependency,
                        v.error.as_deref().unwrap_or("unknown error")
                    ))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }

        // 📝 阶段2: 原子性安装
        let backup = self.config_manager.backup_config().await?;

        match self.execute_atomic_install(specs).await {
            Ok(result) => {
                // 安装成功，清理备份
                self.config_manager.remove_backup(backup).await?;
                Ok(result)
            }
            Err(e) => {
                // 安装失败，恢复备份
                self.config_manager.restore_backup(backup).await?;
                Err(e)
            }
        }
    }

    /// 原子性安装执行
    /// Note: Multiple aliases pointing to the same service will be deduplicated -
    /// only one entry per unique service name will be installed and recorded in lock file
    async fn execute_atomic_install(&self, specs: &[DependencySpec]) -> Result<InstallResult> {
        use std::collections::HashSet;

        let mut result = InstallResult::success();
        let mut installed_services: HashSet<String> = HashSet::new();

        for spec in specs {
            let lookup_key = ValidationPipeline::dependency_lookup_key(spec);
            // Skip if we already installed this service (by name)
            if installed_services.contains(&lookup_key) {
                tracing::debug!(
                    "Skipping duplicate service '{}' (alias: '{}')",
                    lookup_key,
                    spec.alias
                );
                continue;
            }

            // 1. 获取服务详情（在更新配置之前，确保我们有完整的 actr_type）
            let service_details = self
                .validation_pipeline
                .service_discovery
                .get_service_details(&lookup_key)
                .await?;

            // 2. 构建 resolved_spec，使用发现结果中的 canonical actr_type
            let mut resolved_spec = spec.clone();
            resolved_spec.actr_type = Some(service_details.info.actr_type.clone());

            // 3. 更新配置文件（使用包含 actr_type 的 resolved_spec）
            self.config_manager
                .update_dependency(&resolved_spec)
                .await?;
            result.updated_config = true;

            // 4. 缓存Proto文件
            self.cache_manager
                .cache_proto(&spec.name, &service_details.proto_files)
                .await?;

            result.cache_updates += 1;

            // 5. 记录已安装的依赖

            let resolved_dep = ResolvedDependency {
                spec: resolved_spec,
                fingerprint: service_details.info.fingerprint,
                proto_files: service_details.proto_files,
            };
            result.installed_dependencies.push(resolved_dep);

            // Mark this service as installed
            installed_services.insert(lookup_key);
        }

        // 4. 更新锁文件 (lock file also deduplicates by name)
        self.update_lock_file(&result.installed_dependencies)
            .await?;
        result.updated_lock_file = true;

        Ok(result)
    }

    /// Update lock file with new format (no embedded proto content)
    async fn update_lock_file(&self, dependencies: &[ResolvedDependency]) -> Result<()> {
        let project_root = self.config_manager.get_project_root();
        let lock_file_path = project_root.join("Actr.lock.toml");

        // Load existing lock file or create new one
        let mut lock_file = if lock_file_path.exists() {
            LockFile::from_file(&lock_file_path).unwrap_or_else(|_| LockFile::new())
        } else {
            LockFile::new()
        };

        // Update dependencies
        for dep in dependencies {
            let service_name = dep.spec.name.clone();

            // Create protobuf entries with relative path (no content)
            let protobufs: Vec<ProtoFileMeta> = dep
                .proto_files
                .iter()
                .map(|pf| {
                    let file_name = if pf.name.ends_with(".proto") {
                        pf.name.clone()
                    } else {
                        format!("{}.proto", pf.name)
                    };
                    // Path relative to proto/remote/ (e.g., "service_name/file.proto")
                    let path = format!("{}/{}", service_name, file_name);

                    ProtoFileMeta {
                        path,
                        fingerprint: String::new(), // TODO: compute semantic fingerprint
                    }
                })
                .collect();

            // Create service spec metadata
            let spec = ServiceSpecMeta {
                name: dep.spec.name.clone(),
                description: None,
                fingerprint: dep.fingerprint.clone(),
                protobufs,
                published_at: None,
                tags: Vec::new(),
            };

            // Create locked dependency
            let actr_type = dep.spec.actr_type.clone().ok_or_else(|| {
                anyhow::anyhow!("Actr type is required for dependency: {}", service_name)
            })?;
            let locked_dep = LockedDependency::new(actr_type.to_string_repr(), spec);
            lock_file.add_dependency(locked_dep);
        }

        // Update timestamp and save
        lock_file.update_timestamp();
        lock_file.save_to_file(&lock_file_path)?;

        tracing::info!("Updated lock file: {} dependencies", dependencies.len());
        Ok(())
    }
}

// ============================================================================
// 3. 生成管道 (GenerationPipeline)
// ============================================================================

/// 代码生成管道
pub struct GenerationPipeline {
    #[allow(dead_code)]
    config_manager: Arc<dyn ConfigManager>,
    proto_processor: Arc<dyn ProtoProcessor>,
    #[allow(dead_code)]
    cache_manager: Arc<dyn CacheManager>,
}

impl GenerationPipeline {
    pub fn new(
        config_manager: Arc<dyn ConfigManager>,
        proto_processor: Arc<dyn ProtoProcessor>,
        cache_manager: Arc<dyn CacheManager>,
    ) -> Self {
        Self {
            config_manager,
            proto_processor,
            cache_manager,
        }
    }

    /// 执行代码生成
    pub async fn generate_code(&self, options: &GenerationOptions) -> Result<GenerationResult> {
        // 1. 清理输出目录（如果需要）
        if options.clean_before_generate {
            self.clean_output_directory(&options.output_path).await?;
        }

        // 2. 发现本地Proto文件
        let local_protos = self
            .proto_processor
            .discover_proto_files(&options.input_path)
            .await?;

        // 3. 加载依赖的Proto文件
        let dependency_protos = self.load_dependency_protos().await?;

        // 4. 验证Proto语法
        let all_protos = [local_protos, dependency_protos].concat();
        let validation = self
            .proto_processor
            .validate_proto_syntax(&all_protos)
            .await?;

        if !validation.is_valid {
            return Err(anyhow::anyhow!("Proto file syntax validation failed"));
        }

        // 5. 执行代码生成
        let mut generation_result = self
            .proto_processor
            .generate_code(&options.input_path, &options.output_path)
            .await?;

        // 6. 后处理：格式化和检查
        if options.format_code {
            self.format_generated_code(&generation_result.generated_files)
                .await?;
        }

        if options.run_checks {
            let check_result = self
                .run_code_checks(&generation_result.generated_files)
                .await?;
            generation_result.warnings.extend(check_result.warnings);
            generation_result.errors.extend(check_result.errors);
        }

        Ok(generation_result)
    }

    /// 清理输出目录
    async fn clean_output_directory(&self, output_path: &std::path::Path) -> Result<()> {
        if output_path.exists() {
            std::fs::remove_dir_all(output_path)?;
        }
        std::fs::create_dir_all(output_path)?;
        Ok(())
    }

    /// 加载依赖的Proto文件
    async fn load_dependency_protos(&self) -> Result<Vec<ProtoFile>> {
        // TODO: 从缓存中加载依赖的Proto文件
        Ok(Vec::new())
    }

    /// 格式化生成的代码
    async fn format_generated_code(&self, files: &[std::path::PathBuf]) -> Result<()> {
        for file in files {
            if file.extension().and_then(|s| s.to_str()) == Some("rs") {
                // 运行 rustfmt
                let output = std::process::Command::new("rustfmt").arg(file).output()?;

                if !output.status.success() {
                    eprintln!(
                        "rustfmt warning: {}",
                        String::from_utf8_lossy(&output.stderr)
                    );
                }
            }
        }
        Ok(())
    }

    /// 运行代码检查
    async fn run_code_checks(&self, files: &[std::path::PathBuf]) -> Result<GenerationResult> {
        // TODO: 运行 cargo check 或其他代码检查工具
        Ok(GenerationResult {
            generated_files: files.to_vec(),
            warnings: vec![],
            errors: vec![],
        })
    }
}
