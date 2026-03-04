//! 核心复用组件定义
//!
//! 定义了8个核心组件的trait接口，支持依赖注入和组合使用

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

// ============================================================================
// 核心数据类型
// ============================================================================

/// 依赖规范
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DependencySpec {
    pub name: String,
    pub uri: String,
    pub version: Option<String>,
    pub fingerprint: Option<String>,
}

/// 解析后的依赖信息
#[derive(Debug, Clone)]
pub struct ResolvedDependency {
    pub spec: DependencySpec,
    pub uri: String,
    pub resolved_version: String,
    pub fingerprint: String,
    pub proto_files: Vec<ProtoFile>,
}

/// Proto文件信息
#[derive(Debug, Clone)]
pub struct ProtoFile {
    pub name: String,
    pub path: PathBuf,
    pub content: String,
    pub services: Vec<ServiceDefinition>,
}

/// 服务定义
#[derive(Debug, Clone)]
pub struct ServiceDefinition {
    pub name: String,
    pub methods: Vec<MethodDefinition>,
}

/// 方法定义
#[derive(Debug, Clone)]
pub struct MethodDefinition {
    pub name: String,
    pub input_type: String,
    pub output_type: String,
}

/// 服务信息
#[derive(Debug, Clone)]
pub struct ServiceInfo {
    pub name: String,
    pub uri: String,
    pub version: String,
    pub fingerprint: String,
    pub description: Option<String>,
    pub methods: Vec<MethodDefinition>,
}

/// 服务详情
#[derive(Debug, Clone)]
pub struct ServiceDetails {
    pub info: ServiceInfo,
    pub proto_files: Vec<ProtoFile>,
    pub dependencies: Vec<String>,
}

/// 指纹信息
#[derive(Debug, Clone, PartialEq)]
pub struct Fingerprint {
    pub algorithm: String,
    pub value: String,
}

/// 验证报告
#[derive(Debug, Clone)]
pub struct ValidationReport {
    pub is_valid: bool,
    pub config_validation: ConfigValidation,
    pub dependency_validation: Vec<DependencyValidation>,
    pub network_validation: Vec<NetworkValidation>,
    pub fingerprint_validation: Vec<FingerprintValidation>,
    pub conflicts: Vec<ConflictReport>,
}

#[derive(Debug, Clone)]
pub struct ConfigValidation {
    pub is_valid: bool,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct DependencyValidation {
    pub dependency: String,
    pub is_available: bool,
    pub resolved_uri: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct NetworkValidation {
    pub uri: String,
    pub is_reachable: bool,
    pub latency_ms: Option<u64>,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct FingerprintValidation {
    pub dependency: String,
    pub expected: Fingerprint,
    pub actual: Option<Fingerprint>,
    pub is_valid: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ConflictReport {
    pub dependency_a: String,
    pub dependency_b: String,
    pub conflict_type: ConflictType,
    pub description: String,
}

#[derive(Debug, Clone)]
pub enum ConflictType {
    VersionConflict,
    FingerprintMismatch,
    CircularDependency,
}

impl ValidationReport {
    pub fn is_success(&self) -> bool {
        self.is_valid
            && self.config_validation.is_valid
            && self.dependency_validation.iter().all(|d| d.is_available)
            && self.network_validation.iter().all(|n| n.is_reachable)
            && self.fingerprint_validation.iter().all(|f| f.is_valid)
            && self.conflicts.is_empty()
    }
}

// ============================================================================
// 1. 配置管理组件 (ConfigManager)
// ============================================================================

/// 统一的配置管理接口
#[async_trait]
pub trait ConfigManager: Send + Sync {
    /// 加载配置文件
    async fn load_config(&self, path: &Path) -> Result<ActrConfig>;

    /// 保存配置文件
    async fn save_config(&self, config: &ActrConfig, path: &Path) -> Result<()>;

    /// 更新依赖配置
    async fn update_dependency(&self, spec: &DependencySpec) -> Result<()>;

    /// 验证配置文件
    async fn validate_config(&self) -> Result<ConfigValidation>;

    /// 获取项目根目录
    fn get_project_root(&self) -> &Path;

    /// 备份当前配置
    async fn backup_config(&self) -> Result<ConfigBackup>;

    /// 恢复配置备份
    async fn restore_backup(&self, backup: ConfigBackup) -> Result<()>;

    /// 删除配置备份
    async fn remove_backup(&self, backup: ConfigBackup) -> Result<()>;
}

/// 配置文件结构
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActrConfig {
    pub package: PackageConfig,
    pub provides: Option<HashMap<String, String>>,
    pub dependencies: Option<HashMap<String, DependencyConfig>>,
    pub scripts: Option<HashMap<String, String>>,
    pub routing: Option<HashMap<String, String>>,
    pub signaling: Option<SignalingConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageConfig {
    pub name: String,
    pub version: String,
    #[serde(rename = "type")]
    pub package_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DependencyConfig {
    Simple(String),
    Complex {
        uri: String,
        #[serde(default)]
        version: Option<String>,
        #[serde(default)]
        fingerprint: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignalingConfig {
    pub url: String,
    pub auth: Option<AuthConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthConfig {
    pub token: Option<String>,
    pub username: Option<String>,
    pub password: Option<String>,
}

/// 配置备份
#[derive(Debug, Clone)]
pub struct ConfigBackup {
    pub original_path: PathBuf,
    pub backup_path: PathBuf,
    pub timestamp: std::time::SystemTime,
}

// ============================================================================
// 2. 依赖解析组件 (DependencyResolver)
// ============================================================================

/// 依赖解析和冲突检测
#[async_trait]
pub trait DependencyResolver: Send + Sync {
    /// 解析依赖规范字符串
    async fn resolve_spec(&self, spec: &str) -> Result<DependencySpec>;

    /// 解析多个依赖
    async fn resolve_dependencies(
        &self,
        specs: &[DependencySpec],
    ) -> Result<Vec<ResolvedDependency>>;

    /// 检查依赖冲突
    async fn check_conflicts(&self, deps: &[ResolvedDependency]) -> Result<Vec<ConflictReport>>;

    /// 构建依赖图
    async fn build_dependency_graph(&self, deps: &[ResolvedDependency]) -> Result<DependencyGraph>;
}

#[derive(Debug, Clone)]
pub struct DependencyGraph {
    pub nodes: Vec<String>,
    pub edges: Vec<(String, String)>,
    pub has_cycles: bool,
}

// ============================================================================
// 3. 服务发现组件 (ServiceDiscovery)
// ============================================================================

/// 服务发现和网络交互
#[async_trait]
pub trait ServiceDiscovery: Send + Sync {
    /// 发现网络中的服务
    async fn discover_services(&self, filter: Option<&ServiceFilter>) -> Result<Vec<ServiceInfo>>;

    /// 获取服务详细信息
    async fn get_service_details(&self, uri: &str) -> Result<ServiceDetails>;

    /// 检查服务可用性
    async fn check_service_availability(&self, uri: &str) -> Result<AvailabilityStatus>;

    /// 获取服务Proto文件
    async fn get_service_proto(&self, uri: &str) -> Result<Vec<ProtoFile>>;
}

#[derive(Debug, Clone)]
pub struct ServiceFilter {
    pub name_pattern: Option<String>,
    pub version_range: Option<String>,
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Clone)]
pub struct AvailabilityStatus {
    pub is_available: bool,
    pub last_seen: Option<std::time::SystemTime>,
    pub health: HealthStatus,
}

#[derive(Debug, Clone)]
pub enum HealthStatus {
    Healthy,
    Degraded,
    Unhealthy,
    Unknown,
}

// ============================================================================
// 4. 网络验证组件 (NetworkValidator)
// ============================================================================

/// 网络连通性验证
#[async_trait]
pub trait NetworkValidator: Send + Sync {
    /// 检查连通性
    async fn check_connectivity(&self, uri: &str) -> Result<ConnectivityStatus>;

    /// 验证服务健康状态
    async fn verify_service_health(&self, uri: &str) -> Result<HealthStatus>;

    /// 测试延迟
    async fn test_latency(&self, uri: &str) -> Result<LatencyInfo>;

    /// 批量检查
    async fn batch_check(&self, uris: &[String]) -> Result<Vec<NetworkCheckResult>>;
}

#[derive(Debug, Clone)]
pub struct ConnectivityStatus {
    pub is_reachable: bool,
    pub response_time_ms: Option<u64>,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct LatencyInfo {
    pub min_ms: u64,
    pub max_ms: u64,
    pub avg_ms: u64,
    pub samples: u32,
}

#[derive(Debug, Clone)]
pub struct NetworkCheckResult {
    pub uri: String,
    pub connectivity: ConnectivityStatus,
    pub health: HealthStatus,
    pub latency: Option<LatencyInfo>,
}

// ============================================================================
// 5. 指纹验证组件 (FingerprintValidator)
// ============================================================================

/// 指纹计算和验证
#[async_trait]
pub trait FingerprintValidator: Send + Sync {
    /// 计算服务指纹
    async fn compute_service_fingerprint(&self, service: &ServiceInfo) -> Result<Fingerprint>;

    /// 验证指纹匹配
    async fn verify_fingerprint(
        &self,
        expected: &Fingerprint,
        actual: &Fingerprint,
    ) -> Result<bool>;

    /// 计算项目指纹
    async fn compute_project_fingerprint(&self, project_path: &Path) -> Result<Fingerprint>;

    /// 生成锁文件指纹
    async fn generate_lock_fingerprint(&self, deps: &[ResolvedDependency]) -> Result<Fingerprint>;
}

// ============================================================================
// 6. Proto处理组件 (ProtoProcessor)
// ============================================================================

/// Protocol Buffers 文件处理
#[async_trait]
pub trait ProtoProcessor: Send + Sync {
    /// 发现Proto文件
    async fn discover_proto_files(&self, path: &Path) -> Result<Vec<ProtoFile>>;

    /// 解析Proto服务
    async fn parse_proto_services(&self, files: &[ProtoFile]) -> Result<Vec<ServiceDefinition>>;

    /// 生成代码
    async fn generate_code(&self, input: &Path, output: &Path) -> Result<GenerationResult>;

    /// 验证Proto语法
    async fn validate_proto_syntax(&self, files: &[ProtoFile]) -> Result<ValidationReport>;
}

#[derive(Debug, Clone)]
pub struct GenerationResult {
    pub generated_files: Vec<PathBuf>,
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
}

// ============================================================================
// 7. 缓存管理组件 (CacheManager)
// ============================================================================

/// 依赖缓存管理
#[async_trait]
pub trait CacheManager: Send + Sync {
    /// 获取缓存的Proto
    async fn get_cached_proto(&self, uri: &str) -> Result<Option<CachedProto>>;

    /// 缓存Proto文件
    async fn cache_proto(&self, uri: &str, proto: &[ProtoFile]) -> Result<()>;

    /// 失效缓存
    async fn invalidate_cache(&self, uri: &str) -> Result<()>;

    /// 清理缓存
    async fn clear_cache(&self) -> Result<()>;

    /// 获取缓存统计
    async fn get_cache_stats(&self) -> Result<CacheStats>;
}

#[derive(Debug, Clone)]
pub struct CachedProto {
    pub uri: String,
    pub files: Vec<ProtoFile>,
    pub fingerprint: Fingerprint,
    pub cached_at: std::time::SystemTime,
    pub expires_at: Option<std::time::SystemTime>,
}

#[derive(Debug, Clone)]
pub struct CacheStats {
    pub total_entries: usize,
    pub total_size_bytes: u64,
    pub hit_rate: f64,
    pub miss_rate: f64,
}

// ============================================================================
// 8. 用户交互组件 (UserInterface)
// ============================================================================

/// 用户交互界面
#[async_trait]
pub trait UserInterface: Send + Sync {
    /// 提示输入
    async fn prompt_input(&self, prompt: &str) -> Result<String>;

    /// 确认操作
    async fn confirm(&self, message: &str) -> Result<bool>;

    /// 从服务列表中选择
    async fn select_service_from_list(
        &self,
        items: &[ServiceInfo],
        formatter: fn(&ServiceInfo) -> String,
    ) -> Result<usize>;

    /// 从字符串列表中选择
    async fn select_string_from_list(
        &self,
        items: &[String],
        formatter: fn(&String) -> String,
    ) -> Result<usize>;

    /// 显示服务表格
    async fn display_service_table(
        &self,
        items: &[ServiceInfo],
        headers: &[&str],
        formatter: fn(&ServiceInfo) -> Vec<String>,
    );

    /// 显示进度条
    async fn show_progress(&self, message: &str) -> Result<Box<dyn ProgressBar>>;
}

/// 进度条接口
pub trait ProgressBar: Send + Sync {
    fn update(&self, progress: f64);
    fn set_message(&self, message: &str);
    fn finish(&self);
}
