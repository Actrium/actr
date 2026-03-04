//! 操作管道定义
//!
//! 定义了三个核心操作管道，实现命令间的逻辑复用

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
            "安装了 {} 个依赖，更新了 {} 个缓存项",
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
                    .join("Actr.toml")
                    .as_path(),
            )
            .await?;
        let dependency_specs = self.extract_dependency_specs(&config)?;
        let resolved_dependencies = self
            .dependency_resolver
            .resolve_dependencies(&dependency_specs)
            .await?;

        // 3. 冲突检查
        let conflicts = self
            .dependency_resolver
            .check_conflicts(&resolved_dependencies)
            .await?;

        // 4. 串行执行网络验证和指纹验证（简化版，实际可以并行）
        let dependency_validation = self.validate_dependencies(&dependency_specs).await?;
        let network_validation = self
            .validate_network_connectivity(&resolved_dependencies)
            .await?;
        let fingerprint_validation = self.validate_fingerprints(&resolved_dependencies).await?;

        let is_valid = config_validation.is_valid
            && dependency_validation.iter().all(|d| d.is_available)
            && network_validation.iter().all(|n| n.is_reachable)
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
    pub async fn validate_dependencies(
        &self,
        specs: &[DependencySpec],
    ) -> Result<Vec<DependencyValidation>> {
        let mut results = Vec::new();

        for spec in specs {
            let validation = match self
                .service_discovery
                .check_service_availability(&spec.uri)
                .await
            {
                Ok(status) => DependencyValidation {
                    dependency: spec.name.clone(),
                    is_available: status.is_available,
                    resolved_uri: Some(spec.uri.clone()),
                    error: None,
                },
                Err(e) => DependencyValidation {
                    dependency: spec.name.clone(),
                    is_available: false,
                    resolved_uri: None,
                    error: Some(e.to_string()),
                },
            };
            results.push(validation);
        }

        Ok(results)
    }

    /// 网络连通性验证
    async fn validate_network_connectivity(
        &self,
        deps: &[ResolvedDependency],
    ) -> Result<Vec<NetworkValidation>> {
        let uris: Vec<String> = deps.iter().map(|d| d.uri.clone()).collect();
        let network_results = self.network_validator.batch_check(&uris).await?;

        Ok(network_results
            .into_iter()
            .map(|result| NetworkValidation {
                uri: result.uri,
                is_reachable: result.connectivity.is_reachable,
                latency_ms: result.connectivity.response_time_ms,
                error: result.connectivity.error,
            })
            .collect())
    }

    /// 指纹验证
    async fn validate_fingerprints(
        &self,
        deps: &[ResolvedDependency],
    ) -> Result<Vec<FingerprintValidation>> {
        let mut results = Vec::new();

        for dep in deps {
            let expected = Fingerprint {
                algorithm: "sha256".to_string(),
                value: dep.fingerprint.clone(),
            };

            // 计算实际指纹
            let service_info = match self.service_discovery.get_service_details(&dep.uri).await {
                Ok(details) => details.info,
                Err(e) => {
                    results.push(FingerprintValidation {
                        dependency: dep.spec.name.clone(),
                        expected,
                        actual: None,
                        is_valid: false,
                        error: Some(e.to_string()),
                    });
                    continue;
                }
            };

            match self
                .fingerprint_validator
                .compute_service_fingerprint(&service_info)
                .await
            {
                Ok(actual) => {
                    let is_valid = self
                        .fingerprint_validator
                        .verify_fingerprint(&expected, &actual)
                        .await
                        .unwrap_or(false);
                    results.push(FingerprintValidation {
                        dependency: dep.spec.name.clone(),
                        expected,
                        actual: Some(actual),
                        is_valid,
                        error: None,
                    });
                }
                Err(e) => {
                    results.push(FingerprintValidation {
                        dependency: dep.spec.name.clone(),
                        expected,
                        actual: None,
                        is_valid: false,
                        error: Some(e.to_string()),
                    });
                }
            }
        }

        Ok(results)
    }

    /// 从配置中提取依赖规范
    fn extract_dependency_specs(&self, config: &ActrConfig) -> Result<Vec<DependencySpec>> {
        let mut specs = Vec::new();

        if let Some(dependencies) = &config.dependencies {
            for (name, dep_config) in dependencies {
                let spec = match dep_config {
                    DependencyConfig::Simple(uri) => DependencySpec {
                        name: name.clone(),
                        uri: uri.clone(),
                        version: None,
                        fingerprint: None,
                    },
                    DependencyConfig::Complex {
                        uri,
                        version,
                        fingerprint,
                    } => DependencySpec {
                        name: name.clone(),
                        uri: uri.clone(),
                        version: version.clone(),
                        fingerprint: fingerprint.clone(),
                    },
                };
                specs.push(spec);
            }
        }

        Ok(specs)
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
    async fn execute_atomic_install(&self, specs: &[DependencySpec]) -> Result<InstallResult> {
        let mut result = InstallResult::success();

        for spec in specs {
            // 1. 更新配置文件
            self.config_manager.update_dependency(spec).await?;
            result.updated_config = true;

            // 2. 获取服务详情并缓存Proto文件
            let service_details = self
                .validation_pipeline
                .service_discovery
                .get_service_details(&spec.uri)
                .await?;

            self.cache_manager
                .cache_proto(&spec.uri, &service_details.proto_files)
                .await?;
            result.cache_updates += 1;

            // 3. 记录已安装的依赖
            let resolved_dep = ResolvedDependency {
                spec: spec.clone(),
                uri: spec.uri.clone(),
                resolved_version: service_details.info.version,
                fingerprint: service_details.info.fingerprint,
                proto_files: service_details.proto_files,
            };
            result.installed_dependencies.push(resolved_dep);
        }

        // 4. 更新锁文件
        self.update_lock_file(&result.installed_dependencies)
            .await?;
        result.updated_lock_file = true;

        Ok(result)
    }

    /// 更新锁文件
    async fn update_lock_file(&self, dependencies: &[ResolvedDependency]) -> Result<()> {
        // TODO: 实现锁文件更新逻辑
        // 这里应该读取现有的锁文件，合并新的依赖信息，然后写回
        println!("更新锁文件: {} 个依赖", dependencies.len());
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
            return Err(anyhow::anyhow!("Proto文件语法验证失败"));
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
                    eprintln!("rustfmt 警告: {}", String::from_utf8_lossy(&output.stderr));
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
