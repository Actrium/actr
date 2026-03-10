use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::error::{HyperError, HyperResult};

/// Hyper 初始化配置
#[derive(Debug, Clone)]
pub struct HyperConfig {
    /// 根数据目录，对应命名空间模板变量 `{data_dir}`
    pub data_dir: PathBuf,

    /// 存储命名空间路径模板，默认 `{data_dir}/{actr_type}`
    ///
    /// 可用变量：
    /// - `{data_dir}`      — 根数据目录
    /// - `{instance_id}`   — Hyper 启动时生成并持久化的本地唯一 ID
    /// - `{hostname}`      — 操作系统主机名
    /// - `{manufacturer}`  — Actor 制造商名
    /// - `{actr_name}`     — Actor 名称
    /// - `{version}`       — Actor 版本
    /// - `{actr_type}`     — 三段式完整类型（`{manufacturer}/{actr_name}/{version}`）
    /// - `{realm_id}`      — Actor 所属 realm（运行时可用）
    /// - `{env.VAR}`       — 任意环境变量
    pub storage_path_template: String,

    /// 信任模式：生产环境使用 actrix 根 CA，开发/测试使用自签名证书
    pub trust_mode: TrustMode,
}

/// MFR 签名信任根配置
#[derive(Debug, Clone)]
pub enum TrustMode {
    /// 生产模式：从 AIS 获取 MFR Ed25519 公钥，本地缓存（TTL 1 小时）
    ///
    /// AIS 端点用于拉取 manufacturer 公钥，格式：`http://ais.example.com:8080`
    Production {
        /// AIS 服务地址（与 bootstrap_credential 的 ais_endpoint 相同）
        ais_endpoint: String,
    },
    /// 开发/测试模式：信任本地自签名公钥（`actr dev sign` 生成）
    ///
    /// 与生产模式走完全相同的验证代码路径，信任锚不同。
    Development {
        /// 自签名证书的 DER 编码公钥（Ed25519 verifying key，32 字节）
        self_signed_pubkey: Vec<u8>,
    },
}

impl Default for HyperConfig {
    fn default() -> Self {
        Self {
            data_dir: PathBuf::from("/var/lib/actr"),
            storage_path_template: "{data_dir}/{actr_type}".to_string(),
            trust_mode: TrustMode::Production {
                ais_endpoint: "http://localhost:8080".to_string(),
            },
        }
    }
}

impl HyperConfig {
    pub fn new(data_dir: impl AsRef<Path>) -> Self {
        Self {
            data_dir: data_dir.as_ref().to_path_buf(),
            ..Default::default()
        }
    }

    pub fn with_storage_template(mut self, template: impl Into<String>) -> Self {
        self.storage_path_template = template.into();
        self
    }

    pub fn with_trust_mode(mut self, mode: TrustMode) -> Self {
        self.trust_mode = mode;
        self
    }
}

/// 命名空间模板解析器
///
/// 持有运行时已知的变量，按需解析路径模板。
/// 模板在 Hyper 初始化时解析一次，之后固定不变。
pub(crate) struct NamespaceResolver {
    vars: HashMap<String, String>,
}

impl NamespaceResolver {
    pub fn new(config: &HyperConfig, instance_id: &str) -> HyperResult<Self> {
        let mut vars = HashMap::new();

        vars.insert(
            "data_dir".to_string(),
            config
                .data_dir
                .to_str()
                .ok_or_else(|| HyperError::Config("data_dir 路径包含非 UTF-8 字符".to_string()))?
                .to_string(),
        );
        vars.insert("instance_id".to_string(), instance_id.to_string());

        if let Ok(hostname) = std::env::var("HOSTNAME").or_else(|_| {
            // fallback: 读取系统 hostname
            std::fs::read_to_string("/etc/hostname")
                .map(|s| s.trim().to_string())
                .map_err(|_| std::env::VarError::NotPresent)
        }) {
            vars.insert("hostname".to_string(), hostname);
        }

        Ok(Self { vars })
    }

    /// 注入 Actor 类型相关变量（从已验证的 manifest 中提取）
    pub fn with_actor_type(
        mut self,
        manufacturer: &str,
        actr_name: &str,
        version: &str,
    ) -> Self {
        self.vars
            .insert("manufacturer".to_string(), manufacturer.to_string());
        self.vars
            .insert("actr_name".to_string(), actr_name.to_string());
        self.vars
            .insert("version".to_string(), version.to_string());
        self.vars.insert(
            "actr_type".to_string(),
            format!("{manufacturer}/{actr_name}/{version}"),
        );
        self
    }

    /// 注入运行时 realm_id
    pub fn with_realm(mut self, realm_id: u64) -> Self {
        self.vars
            .insert("realm_id".to_string(), realm_id.to_string());
        self
    }

    /// 解析模板字符串，返回最终路径
    pub fn resolve(&self, template: &str) -> HyperResult<PathBuf> {
        let mut result = template.to_string();

        // 处理 {env.VAR} 变量
        let env_prefix = "{env.";
        let mut pos = 0;
        while let Some(start) = result[pos..].find(env_prefix) {
            let abs_start = pos + start;
            if let Some(end) = result[abs_start..].find('}') {
                let var_name = &result[abs_start + env_prefix.len()..abs_start + end];
                let value = std::env::var(var_name).map_err(|_| {
                    HyperError::TemplateVariable(format!("env.{var_name}"))
                })?;
                let placeholder = format!("{{env.{var_name}}}");
                result = result.replacen(&placeholder, &value, 1);
                // 位置不前移，重新扫描替换后的字符串
            } else {
                pos = abs_start + 1;
            }
        }

        // 处理普通变量
        for (key, value) in &self.vars {
            result = result.replace(&format!("{{{key}}}"), value);
        }

        // 检查是否还有未解析的变量
        if let Some(start) = result.find('{') {
            if let Some(end) = result[start..].find('}') {
                let var = &result[start + 1..start + end];
                return Err(HyperError::TemplateVariable(var.to_string()));
            }
        }

        Ok(PathBuf::from(result))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_basic_template() {
        let config = HyperConfig::new("/var/lib/actr");
        let resolver = NamespaceResolver::new(&config, "abc123")
            .unwrap()
            .with_actor_type("acme", "Sensor", "1.0.0");

        let path = resolver
            .resolve("{data_dir}/{actr_type}")
            .unwrap();
        assert_eq!(path, PathBuf::from("/var/lib/actr/acme/Sensor/1.0.0"));
    }

    #[test]
    fn resolve_missing_var_returns_error() {
        let config = HyperConfig::new("/tmp");
        let resolver = NamespaceResolver::new(&config, "id1").unwrap();
        let result = resolver.resolve("{data_dir}/{realm_id}");
        assert!(matches!(result, Err(HyperError::TemplateVariable(_))));
    }

    #[test]
    fn resolve_with_realm() {
        let config = HyperConfig::new("/tmp");
        let resolver = NamespaceResolver::new(&config, "id1")
            .unwrap()
            .with_actor_type("acme", "Worker", "2.0")
            .with_realm(42);
        let path = resolver
            .resolve("{data_dir}/{actr_type}/{realm_id}")
            .unwrap();
        assert_eq!(path, PathBuf::from("/tmp/acme/Worker/2.0/42"));
    }
}
