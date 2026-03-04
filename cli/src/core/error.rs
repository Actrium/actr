//! 统一错误处理
//!
//! 定义了CLI工具的统一错误类型和处理策略

use thiserror::Error;

/// CLI 统一错误类型
#[derive(Debug, Error)]
pub enum ActrCliError {
    #[error("配置错误: {message}")]
    Config { message: String },

    #[error("无效项目: {message}")]
    InvalidProject { message: String },

    #[error("网络错误: {message}")]
    Network { message: String },

    #[error("依赖错误: {message}")]
    Dependency { message: String },

    #[error("服务发现错误: {message}")]
    ServiceDiscovery { message: String },

    #[error("指纹验证错误: {message}")]
    FingerprintValidation { message: String },

    #[error("代码生成错误: {message}")]
    CodeGeneration { message: String },

    #[error("缓存错误: {message}")]
    Cache { message: String },

    #[error("用户交互错误: {message}")]
    UserInterface { message: String },

    #[error("命令执行错误: {message}")]
    Command { message: String },

    #[error("验证失败: {details}")]
    ValidationFailed { details: String },

    #[error("安装失败: {reason}")]
    InstallFailed { reason: String },

    #[error("组件未注册: {component}")]
    ComponentNotRegistered { component: String },

    #[error("IO 错误")]
    Io(#[from] std::io::Error),

    #[error("序列化错误")]
    Serialization(#[from] toml::de::Error),

    #[error("HTTP 错误")]
    Http(#[from] reqwest::Error),

    #[error("其他错误: {0}")]
    Other(#[from] anyhow::Error),
}

/// 安装错误
#[derive(Debug, Error)]
pub enum InstallError {
    #[error("依赖解析失败: {dependency}")]
    DependencyResolutionFailed { dependency: String },

    #[error("服务不可用: {service}")]
    ServiceUnavailable { service: String },

    #[error("网络连接失败: {uri}")]
    NetworkConnectionFailed { uri: String },

    #[error("指纹验证失败: {service} - 期望: {expected}, 实际: {actual}")]
    FingerprintMismatch {
        service: String,
        expected: String,
        actual: String,
    },

    #[error("版本冲突: {details}")]
    VersionConflict { details: String },

    #[error("缓存操作失败: {operation}")]
    CacheOperationFailed { operation: String },

    #[error("配置更新失败: {reason}")]
    ConfigUpdateFailed { reason: String },

    #[error("前置验证失败: {failures:?}")]
    PreCheckFailed { failures: Vec<String> },
}

/// 验证错误
#[derive(Debug, Error)]
pub enum ValidationError {
    #[error("配置文件语法错误: {file}")]
    ConfigSyntaxError { file: String },

    #[error("依赖不存在: {dependency}")]
    DependencyNotFound { dependency: String },

    #[error("网络不可达: {uri}")]
    NetworkUnreachable { uri: String },

    #[error("指纹不匹配: {service}")]
    FingerprintMismatch { service: String },

    #[error("循环依赖: {cycle}")]
    CircularDependency { cycle: String },

    #[error("权限不足: {resource}")]
    InsufficientPermissions { resource: String },
}

/// 用户友好的错误显示
impl ActrCliError {
    /// 获取用户友好的错误消息
    pub fn user_message(&self) -> String {
        match self {
            ActrCliError::Config { message } => {
                format!("⚠️  配置文件错误：{message}\n💡 提示：请检查 Actr.toml 文件的语法和内容")
            }
            ActrCliError::Network { message } => {
                format!("🌐 网络连接错误：{message}\n💡 提示：请检查网络连接和服务地址")
            }
            ActrCliError::Dependency { message } => {
                format!("📦 依赖错误：{message}\n💡 提示：运行 'actr check' 检查依赖状态")
            }
            ActrCliError::ValidationFailed { details } => {
                format!("❌ 验证失败：{details}\n💡 提示：请解决上述问题后重试")
            }
            ActrCliError::InstallFailed { reason } => {
                format!("📥 安装失败：{reason}\n💡 提示：运行 'actr check' 检查环境状态")
            }
            _ => self.to_string(),
        }
    }

    /// 获取可能的解决方案
    pub fn suggested_actions(&self) -> Vec<String> {
        match self {
            ActrCliError::Config { .. } => vec![
                "检查 Actr.toml 文件语法".to_string(),
                "运行 'actr config test' 验证配置".to_string(),
                "参考文档中的配置示例".to_string(),
            ],
            ActrCliError::Network { .. } => vec![
                "检查网络连接".to_string(),
                "确认服务地址正确".to_string(),
                "检查防火墙设置".to_string(),
                "运行 'actr check --verbose' 获取详细信息".to_string(),
            ],
            ActrCliError::Dependency { .. } => vec![
                "运行 'actr check' 检查依赖状态".to_string(),
                "运行 'actr install' 安装缺失的依赖".to_string(),
                "运行 'actr discovery' 查找可用服务".to_string(),
            ],
            ActrCliError::ValidationFailed { .. } => vec![
                "检查并修复报告中的问题".to_string(),
                "运行 'actr check --verbose' 获取详细诊断".to_string(),
                "确保所有依赖服务可用".to_string(),
            ],
            ActrCliError::InstallFailed { .. } => vec![
                "检查磁盘空间".to_string(),
                "检查网络连接".to_string(),
                "运行 'actr check' 验证环境".to_string(),
                "尝试清理缓存后重试".to_string(),
            ],
            _ => vec!["查看详细错误信息".to_string()],
        }
    }

    /// 获取相关文档链接
    pub fn documentation_links(&self) -> Vec<(&str, &str)> {
        match self {
            ActrCliError::Config { .. } => vec![
                ("配置文档", "https://docs.actor-rtc.com/config"),
                ("Actr.toml 参考", "https://docs.actor-rtc.com/actr-toml"),
            ],
            ActrCliError::Dependency { .. } => vec![
                ("依赖管理", "https://docs.actor-rtc.com/dependencies"),
                ("故障排除", "https://docs.actor-rtc.com/troubleshooting"),
            ],
            _ => vec![("用户指南", "https://docs.actor-rtc.com/guide")],
        }
    }
}

/// 将验证报告转换为错误
impl From<super::components::ValidationReport> for ActrCliError {
    fn from(report: super::components::ValidationReport) -> Self {
        let mut details = Vec::new();

        if !report.config_validation.is_valid {
            details.extend(
                report
                    .config_validation
                    .errors
                    .iter()
                    .map(|e| format!("配置错误: {e}")),
            );
        }

        for dep in &report.dependency_validation {
            if !dep.is_available {
                details.push(format!(
                    "依赖不可用: {} - {}",
                    dep.dependency,
                    dep.error.as_deref().unwrap_or("未知错误")
                ));
            }
        }

        for net in &report.network_validation {
            if !net.is_reachable {
                details.push(format!(
                    "网络不可达: {} - {}",
                    net.uri,
                    net.error.as_deref().unwrap_or("连接失败")
                ));
            }
        }

        for fp in &report.fingerprint_validation {
            if !fp.is_valid {
                details.push(format!(
                    "指纹验证失败: {} - {}",
                    fp.dependency,
                    fp.error.as_deref().unwrap_or("指纹不匹配")
                ));
            }
        }

        for conflict in &report.conflicts {
            details.push(format!("依赖冲突: {}", conflict.description));
        }

        ActrCliError::ValidationFailed {
            details: details.join("; "),
        }
    }
}

/// 错误报告格式化器
pub struct ErrorReporter;

impl ErrorReporter {
    /// 格式化错误报告
    pub fn format_error(error: &ActrCliError) -> String {
        let mut output = Vec::new();

        // 主要错误信息
        output.push(error.user_message());
        output.push(String::new());

        // 建议的解决方案
        let actions = error.suggested_actions();
        if !actions.is_empty() {
            output.push("🔧 建议的解决方案：".to_string());
            for (i, action) in actions.iter().enumerate() {
                output.push(format!("   {}. {}", i + 1, action));
            }
            output.push(String::new());
        }

        // 文档链接
        let docs = error.documentation_links();
        if !docs.is_empty() {
            output.push("📚 相关文档：".to_string());
            for (title, url) in docs {
                output.push(format!("   • {title}: {url}"));
            }
            output.push(String::new());
        }

        output.join("\n")
    }

    /// 格式化验证报告
    pub fn format_validation_report(report: &super::components::ValidationReport) -> String {
        let mut output = vec![
            "🔍 依赖验证报告".to_string(),
            "=".repeat(50),
            String::new(),
            "📋 配置文件验证：".to_string(),
        ];

        // 配置验证
        if report.config_validation.is_valid {
            output.push("   ✅ 通过".to_string());
        } else {
            output.push("   ❌ 失败".to_string());
            for error in &report.config_validation.errors {
                output.push(format!("      • {error}"));
            }
        }
        output.push(String::new());

        // 依赖验证
        output.push("📦 依赖可用性验证：".to_string());
        for dep in &report.dependency_validation {
            if dep.is_available {
                output.push(format!("   ✅ {} - 可用", dep.dependency));
            } else {
                output.push(format!(
                    "   ❌ {} - {}",
                    dep.dependency,
                    dep.error.as_deref().unwrap_or("不可用")
                ));
            }
        }
        output.push(String::new());

        // 网络验证
        output.push("🌐 网络连通性验证：".to_string());
        for net in &report.network_validation {
            if net.is_reachable {
                let latency = net
                    .latency_ms
                    .map(|ms| format!(" ({ms}ms)"))
                    .unwrap_or_default();
                output.push(format!("   ✅ {}{}", net.uri, latency));
            } else {
                output.push(format!(
                    "   ❌ {} - {}",
                    net.uri,
                    net.error.as_deref().unwrap_or("不可达")
                ));
            }
        }
        output.push(String::new());

        // 指纹验证
        if !report.fingerprint_validation.is_empty() {
            output.push("🔐 指纹验证：".to_string());
            for fp in &report.fingerprint_validation {
                if fp.is_valid {
                    output.push(format!("   ✅ {} - 验证通过", fp.dependency));
                } else {
                    output.push(format!(
                        "   ❌ {} - {}",
                        fp.dependency,
                        fp.error.as_deref().unwrap_or("验证失败")
                    ));
                }
            }
            output.push(String::new());
        }

        // 冲突报告
        if !report.conflicts.is_empty() {
            output.push("⚠️ 依赖冲突：".to_string());
            for conflict in &report.conflicts {
                output.push(format!(
                    "   • {} vs {}: {}",
                    conflict.dependency_a, conflict.dependency_b, conflict.description
                ));
            }
            output.push(String::new());
        }

        // 总结
        if report.is_success() {
            output.push("✨ 总体状态：所有验证通过".to_string());
        } else {
            output.push("❌ 总体状态：存在问题需要解决".to_string());
        }

        output.join("\n")
    }
}
