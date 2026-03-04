//! Discovery 命令实现
//!
//! 展示多重复用模式：服务发现 → 验证 → 可选安装

use anyhow::Result;
use async_trait::async_trait;

use crate::core::{
    ActrCliError, Command, CommandContext, CommandResult, ComponentType, DependencySpec,
    ServiceInfo,
};

/// Discovery 命令
pub struct DiscoveryCommand {
    filter: Option<String>,
    #[allow(dead_code)]
    verbose: bool,
    auto_install: bool,
}

#[async_trait]
impl Command for DiscoveryCommand {
    async fn execute(&self, context: &CommandContext) -> Result<CommandResult> {
        // 获取复用组件
        let (service_discovery, user_interface, _config_manager) = {
            let container = context.container.lock().unwrap();
            (
                container.get_service_discovery()?,
                container.get_user_interface()?,
                container.get_config_manager()?,
            )
        };

        // 🔍 阶段1: 服务发现
        println!("🔍 正在扫描网络中的 Actor 服务...");

        let filter = self.create_service_filter();
        let services = service_discovery.discover_services(filter.as_ref()).await?;

        if services.is_empty() {
            println!("ℹ️ 当前网络中没有发现可用的 Actor 服务");
            return Ok(CommandResult::Success("No services discovered".to_string()));
        }

        // 显示发现的服务
        self.display_services_table(&services);

        // 🎯 阶段2: 用户交互选择
        let selected_index = user_interface
            .select_service_from_list(&services, |s| format!("{} ({})", s.name, s.version))
            .await?;

        let selected_service = &services[selected_index];

        // 显示服务详情和操作菜单
        self.display_service_details(selected_service).await?;

        // 询问用户操作
        let action_menu = vec![
            "查看服务详情".to_string(),
            "导出 proto 文件".to_string(),
            "添加到配置文件".to_string(),
        ];

        let action_choice = user_interface
            .select_string_from_list(&action_menu, |s| s.clone())
            .await?;

        match action_choice {
            0 => {
                // 查看详情
                self.show_detailed_service_info(selected_service, &service_discovery)
                    .await?;
                Ok(CommandResult::Success(
                    "Service details displayed".to_string(),
                ))
            }
            1 => {
                // 导出 proto 文件
                self.export_proto_files(selected_service, &service_discovery)
                    .await?;
                Ok(CommandResult::Success("Proto files exported".to_string()))
            }
            2 => {
                // 添加到配置文件 - 复用架构的核心流程
                self.add_to_config_with_validation(selected_service, context)
                    .await
            }
            _ => Ok(CommandResult::Success("Invalid choice".to_string())),
        }
    }

    fn required_components(&self) -> Vec<ComponentType> {
        // Discovery 命令需要的组件（支持完整的复用流程）
        vec![
            ComponentType::ServiceDiscovery,     // 核心服务发现
            ComponentType::UserInterface,        // 交互界面
            ComponentType::ConfigManager,        // 配置管理
            ComponentType::DependencyResolver,   // 依赖解析（验证阶段）
            ComponentType::NetworkValidator,     // 网络验证（验证阶段）
            ComponentType::FingerprintValidator, // 指纹验证（验证阶段）
            ComponentType::CacheManager,         // 缓存管理（安装阶段）
            ComponentType::ProtoProcessor,       // Proto处理（安装阶段）
        ]
    }

    fn name(&self) -> &str {
        "discovery"
    }

    fn description(&self) -> &str {
        "发现网络中可用的 Actor 服务 (复用架构 + check-first)"
    }
}

impl DiscoveryCommand {
    pub fn new(filter: Option<String>, verbose: bool, auto_install: bool) -> Self {
        Self {
            filter,
            verbose,
            auto_install,
        }
    }

    /// 创建服务过滤器
    fn create_service_filter(&self) -> Option<crate::core::ServiceFilter> {
        self.filter
            .as_ref()
            .map(|pattern| crate::core::ServiceFilter {
                name_pattern: Some(pattern.clone()),
                version_range: None,
                tags: None,
            })
    }

    /// 显示服务列表表格
    fn display_services_table(&self, services: &[ServiceInfo]) {
        println!();
        println!("🔍 发现的 Actor 服务：");
        println!();
        println!("┌─────────────────┬─────────┬─────────────────────────────────┐");
        println!("│ 服务名称        │ 版本    │ 简介                            │");
        println!("├─────────────────┼─────────┼─────────────────────────────────┤");

        for service in services {
            let description = service
                .description
                .as_deref()
                .unwrap_or("无描述")
                .chars()
                .take(28)
                .collect::<String>();

            println!(
                "│ {:15} │ {:7} │ {:31} │",
                service.name.chars().take(15).collect::<String>(),
                service.version.chars().take(7).collect::<String>(),
                description
            );
        }

        println!("└─────────────────┴─────────┴─────────────────────────────────┘");
        println!();
        println!("→ 使用 ↑↓ 选择服务，回车查看选项，q 退出");
        println!();
    }

    /// 显示服务详情
    async fn display_service_details(&self, service: &ServiceInfo) -> Result<()> {
        println!("📋 选择的服务: {} ({})", service.name, service.version);
        if let Some(desc) = &service.description {
            println!("📝 描述: {desc}");
        }
        println!("🔗 URI: {}", service.uri);
        println!("🔐 指纹: {}", service.fingerprint);
        println!("📊 方法数量: {}", service.methods.len());
        println!();
        Ok(())
    }

    /// 显示详细服务信息
    async fn show_detailed_service_info(
        &self,
        service: &ServiceInfo,
        service_discovery: &std::sync::Arc<dyn crate::core::ServiceDiscovery>,
    ) -> Result<()> {
        println!("📖 {} 详细信息:", service.name);
        println!("════════════════════════════════════════");

        let details = service_discovery.get_service_details(&service.uri).await?;

        println!("🏷️ 服务名称: {}", details.info.name);
        println!("📦 版本: {}", details.info.version);
        println!("🔗 URI: {}", details.info.uri);
        println!("🔐 指纹: {}", details.info.fingerprint);

        if let Some(desc) = &details.info.description {
            println!("📝 描述: {desc}");
        }

        println!();
        println!("📋 可用方法:");
        for method in &details.info.methods {
            println!(
                "  • {}: {} → {}",
                method.name, method.input_type, method.output_type
            );
        }

        if !details.dependencies.is_empty() {
            println!();
            println!("🔗 依赖服务:");
            for dep in &details.dependencies {
                println!("  • {dep}");
            }
        }

        println!();
        println!("📁 Proto 文件:");
        for proto in &details.proto_files {
            println!("  • {} ({} 个服务)", proto.name, proto.services.len());
        }

        Ok(())
    }

    /// 导出 proto 文件
    async fn export_proto_files(
        &self,
        service: &ServiceInfo,
        service_discovery: &std::sync::Arc<dyn crate::core::ServiceDiscovery>,
    ) -> Result<()> {
        println!("📤 正在导出 {} 的 proto 文件...", service.name);

        let proto_files = service_discovery.get_service_proto(&service.uri).await?;

        for proto in &proto_files {
            let file_path = format!("./exported_{}", proto.name);
            std::fs::write(&file_path, &proto.content)?;
            println!("✅ 已导出: {file_path}");
        }

        println!("🎉 导出完成，共 {} 个文件", proto_files.len());
        Ok(())
    }

    /// 添加到配置文件 - 复用架构的核心流程
    async fn add_to_config_with_validation(
        &self,
        service: &ServiceInfo,
        context: &CommandContext,
    ) -> Result<CommandResult> {
        let (config_manager, user_interface) = {
            let container = context.container.lock().unwrap();
            (
                container.get_config_manager()?,
                container.get_user_interface()?,
            )
        };

        // 转换为依赖规范
        let dependency_spec = DependencySpec {
            name: service.name.clone(),
            uri: service.uri.clone(),
            version: Some(service.version.clone()),
            fingerprint: Some(service.fingerprint.clone()),
        };

        println!("📝 正在添加 {} 到配置文件...", service.name);

        // 备份配置
        let backup = config_manager.backup_config().await?;

        // 更新配置
        match config_manager.update_dependency(&dependency_spec).await {
            Ok(_) => {
                println!("✅ 已添加 {} 到配置文件", service.name);
            }
            Err(e) => {
                config_manager.restore_backup(backup).await?;
                return Err(ActrCliError::Config {
                    message: format!("配置更新失败: {e}"),
                }
                .into());
            }
        }

        // 🔍 复用 check 流程验证新依赖
        println!();
        println!("🔍 正在验证新依赖...");

        let validation_pipeline = {
            let mut container = context.container.lock().unwrap();
            container.get_validation_pipeline()?
        };

        match validation_pipeline
            .validate_dependencies(&[dependency_spec.clone()])
            .await
        {
            Ok(validation_results) => {
                let all_passed = validation_results.iter().all(|v| v.is_available);

                if !all_passed {
                    // 验证失败，回滚配置
                    println!("❌ 依赖验证失败，正在回滚配置修改...");
                    config_manager.restore_backup(backup).await?;

                    // 显示验证失败的详细信息
                    for validation in &validation_results {
                        if !validation.is_available {
                            println!(
                                "  • {}: {}",
                                validation.dependency,
                                validation.error.as_deref().unwrap_or("验证失败")
                            );
                        }
                    }

                    return Err(ActrCliError::ValidationFailed {
                        details: "依赖验证失败".to_string(),
                    }
                    .into());
                } else {
                    // 验证成功
                    println!("  ├─ 📋 服务存在性检查 ✅");
                    println!("  ├─ 🌐 网络连通性测试 ✅");
                    println!("  └─ 🔐 指纹完整性验证 ✅");

                    // 清理备份
                    config_manager.remove_backup(backup).await?;
                }
            }
            Err(e) => {
                // 验证出错，回滚配置
                println!("❌ 验证过程出错，正在回滚配置修改...");
                config_manager.restore_backup(backup).await?;
                return Err(e);
            }
        }

        // 🤔 询问是否立即安装
        println!();
        let should_install = if self.auto_install {
            true
        } else {
            user_interface.confirm("🤔 是否立即安装此依赖？").await?
        };

        if should_install {
            // 📦 复用 install 流程
            println!();
            println!("📦 正在安装 {}...", service.name);

            let install_pipeline = {
                let mut container = context.container.lock().unwrap();
                container.get_install_pipeline()?
            };

            match install_pipeline
                .install_dependencies(&[dependency_spec])
                .await
            {
                Ok(install_result) => {
                    println!("  ├─ 📦 缓存 proto 文件 ✅");
                    println!("  ├─ 🔒 更新锁文件 ✅");
                    println!("  └─ ✅ 安装完成");
                    println!();
                    println!("💡 建议: 运行 'actr gen' 生成最新代码");

                    Ok(CommandResult::Install(install_result))
                }
                Err(e) => {
                    eprintln!("❌ 安装失败: {e}");
                    Ok(CommandResult::Success(
                        "Dependency added but installation failed".to_string(),
                    ))
                }
            }
        } else {
            println!("✅ 依赖已添加到配置文件");
            println!("💡 运行 'actr install' 来安装依赖");
            Ok(CommandResult::Success(
                "Dependency added to configuration".to_string(),
            ))
        }
    }
}

impl Default for DiscoveryCommand {
    fn default() -> Self {
        Self::new(None, false, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_service_filter() {
        let cmd = DiscoveryCommand::new(Some("user-*".to_string()), false, false);
        let filter = cmd.create_service_filter();

        assert!(filter.is_some());
        let filter = filter.unwrap();
        assert_eq!(filter.name_pattern, Some("user-*".to_string()));
    }

    #[test]
    fn test_create_service_filter_none() {
        let cmd = DiscoveryCommand::new(None, false, false);
        let filter = cmd.create_service_filter();

        assert!(filter.is_none());
    }

    #[test]
    fn test_required_components() {
        let cmd = DiscoveryCommand::default();
        let components = cmd.required_components();

        // Discovery 命令需要支持完整的复用流程
        assert!(components.contains(&ComponentType::ServiceDiscovery));
        assert!(components.contains(&ComponentType::UserInterface));
        assert!(components.contains(&ComponentType::ConfigManager));
        assert!(components.contains(&ComponentType::DependencyResolver));
        assert!(components.contains(&ComponentType::NetworkValidator));
        assert!(components.contains(&ComponentType::FingerprintValidator));
        assert!(components.contains(&ComponentType::CacheManager));
        assert!(components.contains(&ComponentType::ProtoProcessor));
    }
}
