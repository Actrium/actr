use thiserror::Error;

#[derive(Debug, Error)]
pub enum HyperError {
    /// 包中未找到签名 manifest section
    #[error("package manifest section not found")]
    ManifestNotFound,

    /// manifest 数据格式非法
    #[error("invalid manifest: {0}")]
    InvalidManifest(String),

    /// binary_hash 与重算结果不符，包已被篡改
    #[error("binary hash mismatch: package integrity check failed")]
    BinaryHashMismatch,

    /// MFR 签名验证失败
    #[error("signature verification failed: {0}")]
    SignatureVerificationFailed(String),

    /// MFR 证书不可信（未在 actrix 注册或已吊销）
    #[error("untrusted manufacturer: {0}")]
    UntrustedManufacturer(String),

    /// AIS 注册引导失败
    #[error("AIS bootstrap failed: {0}")]
    AisBootstrapFailed(String),

    /// 存储层错误
    #[error("storage error: {0}")]
    Storage(String),

    /// 配置错误
    #[error("config error: {0}")]
    Config(String),

    /// 命名空间模板变量缺失
    #[error("namespace template variable `{0}` not available")]
    TemplateVariable(String),

    /// 运行时管理错误（spawn 失败、进程崩溃等）
    #[error("runtime error: {0}")]
    Runtime(String),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

pub type HyperResult<T> = Result<T, HyperError>;
