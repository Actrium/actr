//! 依赖注入容器
//!
//! 管理所有组件的生命周期和依赖关系

use anyhow::Result;
use std::collections::HashMap;
use std::sync::Arc;

use super::components::*;
use super::pipelines::*;

/// 组件类型枚举
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub enum ComponentType {
    ConfigManager,
    DependencyResolver,
    ServiceDiscovery,
    NetworkValidator,
    FingerprintValidator,
    ProtoProcessor,
    CacheManager,
    UserInterface,
}

/// 服务容器
pub struct ServiceContainer {
    config_manager: Option<Arc<dyn ConfigManager>>,
    dependency_resolver: Option<Arc<dyn DependencyResolver>>,
    service_discovery: Option<Arc<dyn ServiceDiscovery>>,
    network_validator: Option<Arc<dyn NetworkValidator>>,
    fingerprint_validator: Option<Arc<dyn FingerprintValidator>>,
    proto_processor: Option<Arc<dyn ProtoProcessor>>,
    cache_manager: Option<Arc<dyn CacheManager>>,
    user_interface: Option<Arc<dyn UserInterface>>,

    // 缓存的管道实例
    validation_pipeline: Option<Arc<ValidationPipeline>>,
    install_pipeline: Option<Arc<InstallPipeline>>,
    generation_pipeline: Option<Arc<GenerationPipeline>>,
}

impl ServiceContainer {
    /// 创建新的服务容器
    pub fn new() -> Self {
        Self {
            config_manager: None,
            dependency_resolver: None,
            service_discovery: None,
            network_validator: None,
            fingerprint_validator: None,
            proto_processor: None,
            cache_manager: None,
            user_interface: None,
            validation_pipeline: None,
            install_pipeline: None,
            generation_pipeline: None,
        }
    }

    /// 注册组件
    pub fn register_config_manager(mut self, component: Arc<dyn ConfigManager>) -> Self {
        self.config_manager = Some(component);
        self
    }

    pub fn register_dependency_resolver(mut self, component: Arc<dyn DependencyResolver>) -> Self {
        self.dependency_resolver = Some(component);
        self
    }

    pub fn register_service_discovery(mut self, component: Arc<dyn ServiceDiscovery>) -> Self {
        self.service_discovery = Some(component);
        self
    }

    pub fn register_network_validator(mut self, component: Arc<dyn NetworkValidator>) -> Self {
        self.network_validator = Some(component);
        self
    }

    pub fn register_fingerprint_validator(
        mut self,
        component: Arc<dyn FingerprintValidator>,
    ) -> Self {
        self.fingerprint_validator = Some(component);
        self
    }

    pub fn register_proto_processor(mut self, component: Arc<dyn ProtoProcessor>) -> Self {
        self.proto_processor = Some(component);
        self
    }

    pub fn register_cache_manager(mut self, component: Arc<dyn CacheManager>) -> Self {
        self.cache_manager = Some(component);
        self
    }

    pub fn register_user_interface(mut self, component: Arc<dyn UserInterface>) -> Self {
        self.user_interface = Some(component);
        self
    }

    /// 获取组件
    pub fn get_config_manager(&self) -> Result<Arc<dyn ConfigManager>> {
        self.config_manager
            .clone()
            .ok_or_else(|| anyhow::anyhow!("ConfigManager not registered"))
    }

    pub fn get_dependency_resolver(&self) -> Result<Arc<dyn DependencyResolver>> {
        self.dependency_resolver
            .clone()
            .ok_or_else(|| anyhow::anyhow!("DependencyResolver not registered"))
    }

    pub fn get_service_discovery(&self) -> Result<Arc<dyn ServiceDiscovery>> {
        self.service_discovery
            .clone()
            .ok_or_else(|| anyhow::anyhow!("ServiceDiscovery not registered"))
    }

    pub fn get_network_validator(&self) -> Result<Arc<dyn NetworkValidator>> {
        self.network_validator
            .clone()
            .ok_or_else(|| anyhow::anyhow!("NetworkValidator not registered"))
    }

    pub fn get_fingerprint_validator(&self) -> Result<Arc<dyn FingerprintValidator>> {
        self.fingerprint_validator
            .clone()
            .ok_or_else(|| anyhow::anyhow!("FingerprintValidator not registered"))
    }

    pub fn get_proto_processor(&self) -> Result<Arc<dyn ProtoProcessor>> {
        self.proto_processor
            .clone()
            .ok_or_else(|| anyhow::anyhow!("ProtoProcessor not registered"))
    }

    pub fn get_cache_manager(&self) -> Result<Arc<dyn CacheManager>> {
        self.cache_manager
            .clone()
            .ok_or_else(|| anyhow::anyhow!("CacheManager not registered"))
    }

    pub fn get_user_interface(&self) -> Result<Arc<dyn UserInterface>> {
        self.user_interface
            .clone()
            .ok_or_else(|| anyhow::anyhow!("UserInterface not registered"))
    }

    /// 获取验证管道 (延迟创建)
    pub fn get_validation_pipeline(&mut self) -> Result<Arc<ValidationPipeline>> {
        if self.validation_pipeline.is_none() {
            let pipeline = ValidationPipeline::new(
                self.get_config_manager()?,
                self.get_dependency_resolver()?,
                self.get_service_discovery()?,
                self.get_network_validator()?,
                self.get_fingerprint_validator()?,
            );
            self.validation_pipeline = Some(Arc::new(pipeline));
        }

        Ok(self.validation_pipeline.clone().unwrap())
    }

    /// 获取安装管道 (延迟创建)
    pub fn get_install_pipeline(&mut self) -> Result<Arc<InstallPipeline>> {
        if self.install_pipeline.is_none() {
            let validation_pipeline = (*self.get_validation_pipeline()?).clone();
            let pipeline = InstallPipeline::new(
                validation_pipeline,
                self.get_config_manager()?,
                self.get_cache_manager()?,
                self.get_proto_processor()?,
            );
            self.install_pipeline = Some(Arc::new(pipeline));
        }

        Ok(self.install_pipeline.clone().unwrap())
    }

    /// 获取生成管道 (延迟创建)
    pub fn get_generation_pipeline(&mut self) -> Result<Arc<GenerationPipeline>> {
        if self.generation_pipeline.is_none() {
            let pipeline = GenerationPipeline::new(
                self.get_config_manager()?,
                self.get_proto_processor()?,
                self.get_cache_manager()?,
            );
            self.generation_pipeline = Some(Arc::new(pipeline));
        }

        Ok(self.generation_pipeline.clone().unwrap())
    }

    /// 验证所有必需的组件是否已注册
    pub fn validate(&self, required_components: &[ComponentType]) -> Result<()> {
        for component_type in required_components {
            match component_type {
                ComponentType::ConfigManager => {
                    if self.config_manager.is_none() {
                        return Err(anyhow::anyhow!(
                            "ConfigManager is required but not registered"
                        ));
                    }
                }
                ComponentType::DependencyResolver => {
                    if self.dependency_resolver.is_none() {
                        return Err(anyhow::anyhow!(
                            "DependencyResolver is required but not registered"
                        ));
                    }
                }
                ComponentType::ServiceDiscovery => {
                    if self.service_discovery.is_none() {
                        return Err(anyhow::anyhow!(
                            "ServiceDiscovery is required but not registered"
                        ));
                    }
                }
                ComponentType::NetworkValidator => {
                    if self.network_validator.is_none() {
                        return Err(anyhow::anyhow!(
                            "NetworkValidator is required but not registered"
                        ));
                    }
                }
                ComponentType::FingerprintValidator => {
                    if self.fingerprint_validator.is_none() {
                        return Err(anyhow::anyhow!(
                            "FingerprintValidator is required but not registered"
                        ));
                    }
                }
                ComponentType::ProtoProcessor => {
                    if self.proto_processor.is_none() {
                        return Err(anyhow::anyhow!(
                            "ProtoProcessor is required but not registered"
                        ));
                    }
                }
                ComponentType::CacheManager => {
                    if self.cache_manager.is_none() {
                        return Err(anyhow::anyhow!(
                            "CacheManager is required but not registered"
                        ));
                    }
                }
                ComponentType::UserInterface => {
                    if self.user_interface.is_none() {
                        return Err(anyhow::anyhow!(
                            "UserInterface is required but not registered"
                        ));
                    }
                }
            }
        }
        Ok(())
    }
}

impl Default for ServiceContainer {
    fn default() -> Self {
        Self::new()
    }
}

/// 容器构建器
pub struct ContainerBuilder {
    container: ServiceContainer,
    config_path: Option<std::path::PathBuf>,
}

impl ContainerBuilder {
    /// 创建新的构建器
    pub fn new() -> Self {
        Self {
            container: ServiceContainer::new(),
            config_path: None,
        }
    }

    /// 设置配置文件路径
    pub fn config_path<P: Into<std::path::PathBuf>>(mut self, path: P) -> Self {
        self.config_path = Some(path.into());
        self
    }

    /// 构建容器
    pub fn build(self) -> Result<ServiceContainer> {
        // TODO: 这里应该根据配置创建默认的组件实现
        // 现在先返回空容器，实际实现时需要创建具体的组件实例

        Ok(self.container)
    }
}

impl Default for ContainerBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// 命令执行上下文
pub struct CommandContext {
    pub container: Arc<std::sync::Mutex<ServiceContainer>>,
    pub args: CommandArgs,
    pub working_dir: std::path::PathBuf,
}

/// 命令参数
#[derive(Debug, Clone)]
pub struct CommandArgs {
    pub command: String,
    pub subcommand: Option<String>,
    pub flags: HashMap<String, String>,
    pub positional: Vec<String>,
}

/// 命令结果
#[derive(Debug, Clone)]
pub enum CommandResult {
    Success(String),
    Install(InstallResult),
    Validation(ValidationReport),
    Generation(GenerationResult),
    Error(String),
}

/// 命令接口
#[async_trait::async_trait]
pub trait Command: Send + Sync {
    /// 执行命令
    async fn execute(&self, context: &CommandContext) -> Result<CommandResult>;

    /// 获取所需的组件类型
    fn required_components(&self) -> Vec<ComponentType>;

    /// 命令名称
    fn name(&self) -> &str;

    /// 命令描述
    fn description(&self) -> &str;
}
