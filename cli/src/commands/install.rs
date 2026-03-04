//! Install 命令实现
//!
//! 基于复用架构实现 check-first 原则的安装流程

use anyhow::Result;
use async_trait::async_trait;

use crate::core::{
    ActrCliError, Command, CommandContext, CommandResult, ComponentType, DependencySpec,
    ErrorReporter, InstallResult,
};

/// Install 命令
pub struct InstallCommand {
    packages: Vec<String>,
    #[allow(dead_code)]
    force: bool,
    force_update: bool,
    #[allow(dead_code)]
    skip_verification: bool,
}

#[async_trait]
impl Command for InstallCommand {
    async fn execute(&self, context: &CommandContext) -> Result<CommandResult> {
        // 🔍 Check-First 原则：先验证项目状态
        if !self.is_actr_project() {
            return Err(ActrCliError::InvalidProject {
                message: "Not an Actor-RTC project. Run 'actr init' to initialize.".to_string(),
            }
            .into());
        }

        // 确定安装模式
        let dependency_specs = if !self.packages.is_empty() {
            // 模式1: 添加新依赖 (npm install <package>)
            println!("📦 添加 {} 个新的服务依赖", self.packages.len());
            self.parse_new_packages()?
        } else {
            // 模式2: 安装配置中的依赖 (npm install)
            if self.force_update {
                println!("📦 强制更新配置中的所有服务依赖");
            } else {
                println!("📦 安装配置中的服务依赖");
            }
            self.load_dependencies_from_config(context).await?
        };

        if dependency_specs.is_empty() {
            println!("ℹ️ 没有需要安装的依赖");
            return Ok(CommandResult::Success(
                "No dependencies to install".to_string(),
            ));
        }

        // 获取安装管道（自动包含 ValidationPipeline）
        let install_pipeline = {
            let mut container = context.container.lock().unwrap();
            container.get_install_pipeline()?
        };

        // 🚀 执行 check-first 安装流程
        match install_pipeline
            .install_dependencies(&dependency_specs)
            .await
        {
            Ok(install_result) => {
                self.display_install_success(&install_result);
                Ok(CommandResult::Install(install_result))
            }
            Err(e) => {
                // 友好的错误显示
                let cli_error = ActrCliError::InstallFailed {
                    reason: e.to_string(),
                };
                eprintln!("{}", ErrorReporter::format_error(&cli_error));
                Err(e)
            }
        }
    }

    fn required_components(&self) -> Vec<ComponentType> {
        // Install 命令需要完整的安装管道组件
        vec![
            ComponentType::ConfigManager,
            ComponentType::DependencyResolver,
            ComponentType::ServiceDiscovery,
            ComponentType::NetworkValidator,
            ComponentType::FingerprintValidator,
            ComponentType::ProtoProcessor,
            ComponentType::CacheManager,
        ]
    }

    fn name(&self) -> &str {
        "install"
    }

    fn description(&self) -> &str {
        "npm风格的服务级依赖管理 (check-first 架构)"
    }
}

impl InstallCommand {
    pub fn new(
        packages: Vec<String>,
        force: bool,
        force_update: bool,
        skip_verification: bool,
    ) -> Self {
        Self {
            packages,
            force,
            force_update,
            skip_verification,
        }
    }

    /// 检查是否在 Actor-RTC 项目中
    fn is_actr_project(&self) -> bool {
        std::path::Path::new("Actr.toml").exists()
    }

    /// 解析新包规范
    fn parse_new_packages(&self) -> Result<Vec<DependencySpec>> {
        let mut specs = Vec::new();

        for package_spec in &self.packages {
            let spec = self.parse_package_spec(package_spec)?;
            specs.push(spec);
        }

        Ok(specs)
    }

    /// 解析单个包规范
    fn parse_package_spec(&self, package_spec: &str) -> Result<DependencySpec> {
        if package_spec.starts_with("actr://") {
            // 直接 actr:// URI
            self.parse_actr_uri(package_spec)
        } else if package_spec.contains('@') {
            // service-name@version 格式
            self.parse_versioned_spec(package_spec)
        } else {
            // 简单服务名
            self.parse_simple_spec(package_spec)
        }
    }

    /// 解析 actr:// URI
    fn parse_actr_uri(&self, uri: &str) -> Result<DependencySpec> {
        // 简化的URI解析，实际实现应该更严格
        if !uri.starts_with("actr://") {
            return Err(anyhow::anyhow!("Invalid actr:// URI: {uri}"));
        }

        let uri_part = &uri[7..]; // Remove "actr://"
        let service_name = if let Some(pos) = uri_part.find('/') {
            uri_part[..pos].to_string()
        } else {
            uri_part.to_string()
        };

        // 提取查询参数（简化版本）
        let (version, fingerprint) = if uri.contains('?') {
            self.parse_query_params(uri)?
        } else {
            (None, None)
        };

        Ok(DependencySpec {
            name: service_name,
            uri: uri.to_string(),
            version,
            fingerprint,
        })
    }

    /// 解析查询参数
    fn parse_query_params(&self, uri: &str) -> Result<(Option<String>, Option<String>)> {
        if let Some(query_start) = uri.find('?') {
            let query = &uri[query_start + 1..];
            let mut version = None;
            let mut fingerprint = None;

            for param in query.split('&') {
                if let Some((key, value)) = param.split_once('=') {
                    match key {
                        "version" => version = Some(value.to_string()),
                        "fingerprint" => fingerprint = Some(value.to_string()),
                        _ => {} // 忽略未知参数
                    }
                }
            }

            Ok((version, fingerprint))
        } else {
            Ok((None, None))
        }
    }

    /// 解析版本化规范 (service@version)
    fn parse_versioned_spec(&self, spec: &str) -> Result<DependencySpec> {
        let parts: Vec<&str> = spec.split('@').collect();
        if parts.len() != 2 {
            return Err(anyhow::anyhow!(
                "Invalid package specification: {spec}. Use 'service-name@version'"
            ));
        }

        let service_name = parts[0].to_string();
        let version = parts[1].to_string();
        let uri = format!("actr://{service_name}/?version={version}");

        Ok(DependencySpec {
            name: service_name,
            uri,
            version: Some(version),
            fingerprint: None,
        })
    }

    /// 解析简单规范 (service-name)
    fn parse_simple_spec(&self, spec: &str) -> Result<DependencySpec> {
        let service_name = spec.to_string();
        let uri = format!("actr://{service_name}/");

        Ok(DependencySpec {
            name: service_name,
            uri,
            version: None,
            fingerprint: None,
        })
    }

    /// 从配置文件加载依赖
    async fn load_dependencies_from_config(
        &self,
        context: &CommandContext,
    ) -> Result<Vec<DependencySpec>> {
        let config_manager = {
            let container = context.container.lock().unwrap();
            container.get_config_manager()?
        };
        let config = config_manager
            .load_config(
                config_manager
                    .get_project_root()
                    .join("Actr.toml")
                    .as_path(),
            )
            .await?;

        let mut specs = Vec::new();

        if let Some(dependencies) = &config.dependencies {
            for (name, dep_config) in dependencies {
                let spec = match dep_config {
                    crate::core::DependencyConfig::Simple(uri) => DependencySpec {
                        name: name.clone(),
                        uri: uri.clone(),
                        version: None,
                        fingerprint: None,
                    },
                    crate::core::DependencyConfig::Complex {
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

    /// 显示安装成功信息
    fn display_install_success(&self, result: &InstallResult) {
        println!();
        println!("✅ 安装成功！");
        println!("   📦 安装的依赖: {}", result.installed_dependencies.len());
        println!("   🗂️  缓存更新: {}", result.cache_updates);

        if result.updated_config {
            println!("   📝 已更新配置文件");
        }

        if result.updated_lock_file {
            println!("   🔒 已更新锁文件");
        }

        if !result.warnings.is_empty() {
            println!();
            println!("⚠️ 警告:");
            for warning in &result.warnings {
                println!("   • {warning}");
            }
        }

        println!();
        println!("💡 建议: 运行 'actr gen' 生成最新的代码");
    }
}

impl Default for InstallCommand {
    fn default() -> Self {
        Self::new(Vec::new(), false, false, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_spec() {
        let cmd = InstallCommand::default();
        let spec = cmd.parse_simple_spec("user-service").unwrap();

        assert_eq!(spec.name, "user-service");
        assert_eq!(spec.uri, "actr://user-service/");
        assert_eq!(spec.version, None);
        assert_eq!(spec.fingerprint, None);
    }

    #[test]
    fn test_parse_versioned_spec() {
        let cmd = InstallCommand::default();
        let spec = cmd.parse_versioned_spec("user-service@1.2.0").unwrap();

        assert_eq!(spec.name, "user-service");
        assert_eq!(spec.uri, "actr://user-service/?version=1.2.0");
        assert_eq!(spec.version, Some("1.2.0".to_string()));
        assert_eq!(spec.fingerprint, None);
    }

    #[test]
    fn test_parse_actr_uri_simple() {
        let cmd = InstallCommand::default();
        let spec = cmd.parse_actr_uri("actr://user-service/").unwrap();

        assert_eq!(spec.name, "user-service");
        assert_eq!(spec.uri, "actr://user-service/");
        assert_eq!(spec.version, None);
        assert_eq!(spec.fingerprint, None);
    }

    #[test]
    fn test_parse_actr_uri_with_params() {
        let cmd = InstallCommand::default();
        let spec = cmd
            .parse_actr_uri("actr://user-service/?version=1.2.0&fingerprint=sha256:abc123")
            .unwrap();

        assert_eq!(spec.name, "user-service");
        assert_eq!(
            spec.uri,
            "actr://user-service/?version=1.2.0&fingerprint=sha256:abc123"
        );
        assert_eq!(spec.version, Some("1.2.0".to_string()));
        assert_eq!(spec.fingerprint, Some("sha256:abc123".to_string()));
    }
}
