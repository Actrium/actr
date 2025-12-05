//! 系统操作抽象层
//!
//! 定义跨平台的系统操作接口，支持不同 Linux 发行版和操作系统

use anyhow::Result;
use std::path::Path;

/// 系统操作提供者 trait
///
/// 抽象了不同平台的系统级操作，使 deploy 工具能够支持多种操作系统和发行版
pub trait SystemProvider {
    /// 获取系统提供者的名称
    fn name(&self) -> &'static str;

    /// 获取系统描述
    fn description(&self) -> &'static str;

    // ========== 依赖检查 ==========
    /// 检查系统依赖是否满足
    fn check_dependencies(&self, dependencies: &[&str]) -> Result<Vec<DependencyStatus>>;

    /// 检查命令是否存在
    fn command_exists(&self, command: &str) -> bool;

    /// 检查是否有包管理器
    fn has_package_manager(&self) -> bool;

    /// 检查是否支持 systemd
    fn has_systemd(&self) -> bool;

    // ========== 包管理 ==========
    /// 安装系统包
    fn install_packages(&self, packages: &[&str]) -> Result<()>;

    /// 更新包索引
    fn update_package_index(&self) -> Result<()>;

    /// 检查包是否已安装
    fn is_package_installed(&self, package: &str) -> bool;

    // ========== 用户和组管理 ==========
    /// 创建系统用户
    fn create_system_user(&self, username: &str, home_dir: Option<&str>) -> Result<()>;

    /// 创建系统组
    fn create_system_group(&self, groupname: &str) -> Result<()>;

    /// 检查用户是否存在
    fn user_exists(&self, username: &str) -> bool;

    /// 检查组是否存在
    fn group_exists(&self, groupname: &str) -> bool;

    /// 将用户添加到组
    fn add_user_to_group(&self, username: &str, groupname: &str) -> Result<()>;

    // ========== 服务管理 ==========
    /// 安装 systemd 服务
    fn install_systemd_service(&self, service_name: &str, service_content: &str) -> Result<()>;

    /// 启用服务
    fn enable_service(&self, service_name: &str) -> Result<()>;

    /// 启动服务
    fn start_service(&self, service_name: &str) -> Result<()>;

    /// 停止服务
    fn stop_service(&self, service_name: &str) -> Result<()>;

    /// 重启服务
    fn restart_service(&self, service_name: &str) -> Result<()>;

    /// 获取服务状态
    fn service_status(&self, service_name: &str) -> Result<ServiceStatus>;

    // ========== 文件和权限管理 ==========
    /// 创建目录（带权限）
    fn create_directory(&self, path: &Path, mode: Option<u32>) -> Result<()>;

    /// 设置文件所有者
    fn set_file_owner(&self, path: &Path, user: &str, group: &str) -> Result<()>;

    /// 设置文件权限
    fn set_file_permissions(&self, path: &Path, mode: u32) -> Result<()>;

    /// 复制文件（保持权限）
    fn copy_file(&self, src: &Path, dst: &Path) -> Result<()>;

    // ========== 网络和防火墙 ==========
    /// 检查端口是否可用
    fn is_port_available(&self, port: u16) -> bool;

    /// 配置防火墙规则（可选实现）
    fn configure_firewall(&self, _port: u16, _protocol: FirewallProtocol) -> Result<()> {
        // 默认实现：什么都不做
        println!(
            "⚠️  Firewall configuration not implemented for {}",
            self.name()
        );
        Ok(())
    }

    // ========== 系统信息 ==========
    /// 获取系统架构
    fn system_arch(&self) -> String;

    /// 获取系统版本
    fn system_version(&self) -> String;

    /// 检查是否为容器环境
    fn is_container(&self) -> bool;

    /// 检查是否有 sudo 权限
    fn has_sudo_access(&self) -> bool;
}

/// 依赖状态
#[derive(Debug, Clone, PartialEq)]
pub struct DependencyStatus {
    pub name: String,
    pub available: bool,
    pub version: Option<String>,
    pub required: bool,
}

impl DependencyStatus {
    pub fn available(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            available: true,
            version: None,
            required: true,
        }
    }

    pub fn missing(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            available: false,
            version: None,
            required: true,
        }
    }

    pub fn with_version(mut self, version: impl Into<String>) -> Self {
        self.version = Some(version.into());
        self
    }

    pub fn optional(mut self) -> Self {
        self.required = false;
        self
    }
}

/// 服务状态
#[derive(Debug, Clone, PartialEq)]
pub enum ServiceStatus {
    /// 服务正在运行
    Running,
    /// 服务已停止
    Stopped,
    /// 服务状态未知
    Unknown,
    /// 服务不存在
    NotFound,
    /// 服务有错误
    Failed(String),
}

/// 防火墙协议
#[derive(Debug, Clone, PartialEq)]
pub enum FirewallProtocol {
    Tcp,
    Udp,
    Both,
}

/// 系统提供者工厂
pub struct SystemProviderFactory;

impl SystemProviderFactory {
    /// 检测当前系统并返回相应的系统提供者
    pub fn detect() -> Result<Box<dyn SystemProvider>> {
        // 检测操作系统类型
        if cfg!(target_os = "linux") {
            // 尝试检测 Linux 发行版
            if Self::is_debian_based() {
                Ok(Box::new(
                    crate::system::providers::debian::DebianProvider::new(),
                ))
            } else if Self::is_redhat_based() {
                // 未来可以添加 RedHat 支持
                Err(anyhow::anyhow!("RedHat-based systems not yet supported"))
            } else {
                // 默认使用 Debian 提供者
                Ok(Box::new(
                    crate::system::providers::debian::DebianProvider::new(),
                ))
            }
        } else if cfg!(target_os = "macos") {
            // 未来可以添加 macOS 支持
            Err(anyhow::anyhow!("macOS not yet supported"))
        } else {
            Err(anyhow::anyhow!("Unsupported operating system"))
        }
    }

    /// 检查是否为 Debian 系发行版
    fn is_debian_based() -> bool {
        std::fs::read_to_string("/etc/os-release")
            .map(|content| {
                content.contains("debian") || content.contains("ubuntu") || content.contains("mint")
            })
            .unwrap_or(false)
    }

    /// 检查是否为 RedHat 系发行版
    fn is_redhat_based() -> bool {
        std::fs::read_to_string("/etc/os-release")
            .map(|content| {
                content.contains("rhel") || content.contains("centos") || content.contains("fedora")
            })
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dependency_status_creation() {
        let dep = DependencyStatus::available("test")
            .with_version("1.0.0")
            .optional();

        assert_eq!(dep.name, "test");
        assert!(dep.available);
        assert_eq!(dep.version, Some("1.0.0".to_string()));
        assert!(!dep.required);
    }

    #[test]
    fn test_service_status() {
        assert_eq!(ServiceStatus::Running, ServiceStatus::Running);
        assert_ne!(ServiceStatus::Running, ServiceStatus::Stopped);
    }
}
