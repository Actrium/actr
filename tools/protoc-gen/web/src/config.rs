//! 配置类型定义

use std::path::PathBuf;

/// Web 代码生成配置
#[derive(Debug, Clone)]
pub struct WebCodegenConfig {
    /// Proto 文件路径列表
    pub proto_files: Vec<PathBuf>,

    /// Rust 输出目录（WASM 侧）
    pub rust_output_dir: PathBuf,

    /// TypeScript 输出目录（Web SDK 侧）
    pub ts_output_dir: PathBuf,

    /// 是否生成 React Hooks
    pub generate_react_hooks: bool,

    /// Proto 包含路径（用于解析 import）
    pub includes: Vec<PathBuf>,

    /// 是否格式化生成的代码
    pub format_code: bool,

    /// 自定义模板目录（可选）
    pub custom_templates_dir: Option<PathBuf>,
}

impl WebCodegenConfig {
    /// 创建新的配置构建器
    pub fn builder() -> WebCodegenConfigBuilder {
        WebCodegenConfigBuilder::default()
    }

    /// 验证配置
    pub fn validate(&self) -> crate::Result<()> {
        use crate::error::CodegenError;

        // 验证至少有一个 proto 文件
        if self.proto_files.is_empty() {
            return Err(CodegenError::config("至少需要一个 proto 文件"));
        }

        // 验证 proto 文件存在
        for proto in &self.proto_files {
            if !proto.exists() {
                return Err(CodegenError::FileNotFound(proto.clone()));
            }
        }

        // 验证 includes 目录存在
        for include in &self.includes {
            if !include.exists() {
                return Err(CodegenError::FileNotFound(include.clone()));
            }
        }

        Ok(())
    }
}

/// 配置构建器
#[derive(Default)]
pub struct WebCodegenConfigBuilder {
    proto_files: Vec<PathBuf>,
    rust_output_dir: Option<PathBuf>,
    ts_output_dir: Option<PathBuf>,
    generate_react_hooks: bool,
    includes: Vec<PathBuf>,
    format_code: bool,
    custom_templates_dir: Option<PathBuf>,
}

impl WebCodegenConfigBuilder {
    /// 添加 proto 文件
    pub fn proto_file<P: Into<PathBuf>>(mut self, path: P) -> Self {
        self.proto_files.push(path.into());
        self
    }

    /// 添加多个 proto 文件
    pub fn proto_files<I, P>(mut self, paths: I) -> Self
    where
        I: IntoIterator<Item = P>,
        P: Into<PathBuf>,
    {
        self.proto_files.extend(paths.into_iter().map(Into::into));
        self
    }

    /// 设置 Rust 输出目录
    pub fn rust_output<P: Into<PathBuf>>(mut self, dir: P) -> Self {
        self.rust_output_dir = Some(dir.into());
        self
    }

    /// 设置 TypeScript 输出目录
    pub fn ts_output<P: Into<PathBuf>>(mut self, dir: P) -> Self {
        self.ts_output_dir = Some(dir.into());
        self
    }

    /// 启用 React Hooks 生成
    pub fn with_react_hooks(mut self, enabled: bool) -> Self {
        self.generate_react_hooks = enabled;
        self
    }

    /// 添加 include 路径
    pub fn include<P: Into<PathBuf>>(mut self, path: P) -> Self {
        self.includes.push(path.into());
        self
    }

    /// 添加多个 include 路径
    pub fn includes<I, P>(mut self, paths: I) -> Self
    where
        I: IntoIterator<Item = P>,
        P: Into<PathBuf>,
    {
        self.includes.extend(paths.into_iter().map(Into::into));
        self
    }

    /// 启用代码格式化
    pub fn with_formatting(mut self, enabled: bool) -> Self {
        self.format_code = enabled;
        self
    }

    /// 设置自定义模板目录
    pub fn custom_templates<P: Into<PathBuf>>(mut self, dir: P) -> Self {
        self.custom_templates_dir = Some(dir.into());
        self
    }

    /// 构建配置
    pub fn build(self) -> crate::Result<WebCodegenConfig> {
        use crate::error::CodegenError;

        let rust_output_dir = self
            .rust_output_dir
            .ok_or_else(|| CodegenError::config("缺少 rust_output_dir 配置"))?;

        let ts_output_dir = self
            .ts_output_dir
            .ok_or_else(|| CodegenError::config("缺少 ts_output_dir 配置"))?;

        let config = WebCodegenConfig {
            proto_files: self.proto_files,
            rust_output_dir,
            ts_output_dir,
            generate_react_hooks: self.generate_react_hooks,
            includes: self.includes,
            format_code: self.format_code,
            custom_templates_dir: self.custom_templates_dir,
        };

        config.validate()?;

        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builder() {
        let config = WebCodegenConfig::builder()
            .proto_file("test.proto")
            .rust_output("src/generated")
            .ts_output("src/types")
            .with_react_hooks(true)
            .include("proto")
            .with_formatting(true);

        // 注意：这里不能 build() 因为文件不存在
        assert_eq!(config.generate_react_hooks, true);
        assert_eq!(config.format_code, true);
    }
}
