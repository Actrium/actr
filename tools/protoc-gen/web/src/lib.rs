//! # actr-web-protoc-codegen
//!
//! Protoc 代码生成器，用于从 Protobuf 定义生成 actr-web 代码
//!
//! ## 功能
//!
//! - 从 `.proto` 文件生成 Rust WASM Actor 代码
//! - 生成 TypeScript 类型定义
//! - 生成 TypeScript ActorRef 包装类
//! - 可选：生成 React Hooks
//!
//! ## 使用方式
//!
//! ### 方式 1：在 build.rs 中使用
//!
//! ```rust,no_run
//! use actr_web_protoc_codegen::{WebCodegen, WebCodegenConfig};
//!
//! let config = WebCodegenConfig {
//!     proto_files: vec!["proto/echo.proto".into()],
//!     rust_output_dir: "src/generated".into(),
//!     ts_output_dir: "../packages/web-sdk/src/generated".into(),
//!     generate_react_hooks: true,
//!     includes: vec!["proto".into()],
//!     custom_templates_dir: None,
//!     format_code: true,
//! };
//!
//! WebCodegen::new(config)
//!     .generate()
//!     .expect("Failed to generate code");
//! ```
//!
//! ### 方式 2：通过 actr-cli 使用
//!
//! ```bash
//! actr gen --platform web \
//!   --input proto/ \
//!   --output crates/actors/src/generated/ \
//!   --ts-output packages/web-sdk/src/generated/ \
//!   --react-hooks
//! ```

use std::path::PathBuf;

pub mod codegen;
mod config;
mod error;
mod generator;
pub mod request;
mod templates;
mod typescript;

pub use codegen::generate;
pub use config::*;
pub use error::*;
pub use generator::*;
pub use request::{
    ActrTypeInfo, DependencyInfo, FileInfo, MethodInfo, ObservabilityInfo, ServiceInfo,
    WebCodegenRequest, WebCodegenResponse,
};

/// Web 平台代码生成器
pub struct WebCodegen {
    config: WebCodegenConfig,
}

impl WebCodegen {
    /// 创建新的代码生成器实例
    pub fn new(config: WebCodegenConfig) -> Self {
        Self { config }
    }

    /// 生成所有代码（Rust + TypeScript）
    pub fn generate(&self) -> Result<GeneratedFiles> {
        tracing::info!("🚀 开始生成 actr-web 代码");

        let mut files = GeneratedFiles::default();

        // 1. 解析 proto 文件
        let services = self.parse_proto_files()?;
        tracing::info!("📁 解析了 {} 个服务", services.len());

        // 2. 生成 Rust WASM Actor 代码
        tracing::info!("🦀 生成 Rust WASM 代码...");
        files.rust_files = self.generate_rust_actors(&services)?;

        // 3. 生成 TypeScript 类型定义
        tracing::info!("📘 生成 TypeScript 类型...");
        files.ts_types = self.generate_typescript_types(&services)?;

        // 4. 生成 TypeScript ActorRef 包装
        tracing::info!("🎯 生成 ActorRef 包装...");
        files.ts_actor_refs = self.generate_actor_refs(&services)?;

        // 5. 可选：生成 React Hooks
        if self.config.generate_react_hooks {
            tracing::info!("⚛️  生成 React Hooks...");
            files.react_hooks = self.generate_react_hooks(&services)?;
        }

        // 6. 写入文件
        files.write_to_disk()?;

        // 7. 格式化代码
        if self.config.format_code {
            files.format_code()?;
        }

        tracing::info!("✅ 代码生成完成！共生成 {} 个文件", files.total_count());

        Ok(files)
    }

    /// 仅生成 Rust 代码（供 build.rs 使用）
    pub fn generate_rust_only(&self) -> Result<Vec<GeneratedFile>> {
        let services = self.parse_proto_files()?;
        self.generate_rust_actors(&services)
    }

    /// 仅生成 TypeScript 代码
    pub fn generate_typescript_only(&self) -> Result<Vec<GeneratedFile>> {
        let services = self.parse_proto_files()?;
        let mut files = Vec::new();
        files.extend(self.generate_typescript_types(&services)?);
        files.extend(self.generate_actor_refs(&services)?);
        Ok(files)
    }

    /// 解析 proto 文件
    fn parse_proto_files(&self) -> Result<Vec<ProtoService>> {
        generator::parse_proto_files(&self.config)
    }

    /// 生成 Rust Actor 代码
    fn generate_rust_actors(&self, services: &[ProtoService]) -> Result<Vec<GeneratedFile>> {
        generator::generate_rust_actors(&self.config, services)
    }

    /// 生成 TypeScript 类型
    fn generate_typescript_types(&self, services: &[ProtoService]) -> Result<Vec<GeneratedFile>> {
        typescript::generate_types(&self.config, services)
    }

    /// 生成 ActorRef 包装
    fn generate_actor_refs(&self, services: &[ProtoService]) -> Result<Vec<GeneratedFile>> {
        typescript::generate_actor_refs(&self.config, services)
    }

    /// 生成 React Hooks
    fn generate_react_hooks(&self, services: &[ProtoService]) -> Result<Vec<GeneratedFile>> {
        typescript::generate_react_hooks(&self.config, services)
    }
}

/// 生成的所有文件
#[derive(Default, Debug)]
pub struct GeneratedFiles {
    pub rust_files: Vec<GeneratedFile>,
    pub ts_types: Vec<GeneratedFile>,
    pub ts_actor_refs: Vec<GeneratedFile>,
    pub react_hooks: Vec<GeneratedFile>,
}

impl GeneratedFiles {
    /// 获取所有文件
    pub fn all_files(&self) -> impl Iterator<Item = &GeneratedFile> {
        self.rust_files
            .iter()
            .chain(self.ts_types.iter())
            .chain(self.ts_actor_refs.iter())
            .chain(self.react_hooks.iter())
    }

    /// 获取文件总数
    pub fn total_count(&self) -> usize {
        self.rust_files.len()
            + self.ts_types.len()
            + self.ts_actor_refs.len()
            + self.react_hooks.len()
    }

    /// 写入所有文件到磁盘
    pub fn write_to_disk(&self) -> Result<()> {
        for file in self.all_files() {
            file.write_to_disk()?;
        }
        Ok(())
    }

    /// 格式化所有生成的代码
    pub fn format_code(&self) -> Result<()> {
        tracing::info!("🎨 格式化生成的代码...");

        // 格式化 Rust 代码
        for file in &self.rust_files {
            if file.path.extension().and_then(|s| s.to_str()) == Some("rs") {
                format_rust_file(&file.path)?;
            }
        }

        // 格式化 TypeScript 代码
        let ts_files: Vec<_> = self
            .ts_types
            .iter()
            .chain(self.ts_actor_refs.iter())
            .chain(self.react_hooks.iter())
            .collect();

        for file in ts_files {
            if file.path.extension().and_then(|s| s.to_str()) == Some("ts") {
                format_typescript_file(&file.path)?;
            }
        }

        tracing::info!("✅ 代码格式化完成");
        Ok(())
    }
}

/// 单个生成的文件
#[derive(Debug, Clone)]
pub struct GeneratedFile {
    pub path: PathBuf,
    pub content: String,
}

impl GeneratedFile {
    /// 创建新的生成文件
    pub fn new(path: PathBuf, content: String) -> Self {
        Self { path, content }
    }

    /// 写入文件到磁盘
    pub fn write_to_disk(&self) -> Result<()> {
        use std::fs;

        // 创建父目录
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }

        // 写入文件
        fs::write(&self.path, &self.content)?;
        tracing::debug!("✅ 写入文件: {}", self.path.display());

        Ok(())
    }
}

/// Proto 服务定义
#[derive(Debug, Clone)]
pub struct ProtoService {
    pub name: String,
    pub package: String,
    pub methods: Vec<ProtoMethod>,
    pub messages: Vec<ProtoMessage>,
}

/// Proto 方法定义
#[derive(Debug, Clone)]
pub struct ProtoMethod {
    pub name: String,
    pub input_type: String,
    pub output_type: String,
    pub is_streaming: bool,
}

/// Proto 消息定义
#[derive(Debug, Clone)]
pub struct ProtoMessage {
    pub name: String,
    pub fields: Vec<ProtoField>,
}

/// Proto 字段定义
#[derive(Debug, Clone)]
pub struct ProtoField {
    pub name: String,
    pub field_type: String,
    pub number: u32,
    pub is_repeated: bool,
    pub is_optional: bool,
}

/// 格式化 Rust 文件
fn format_rust_file(path: &std::path::Path) -> Result<()> {
    use std::process::Command;

    let output = Command::new("rustfmt")
        .arg("--edition")
        .arg("2021")
        .arg(path)
        .output();

    match output {
        Ok(output) if output.status.success() => {
            tracing::debug!("✅ 格式化 Rust 文件: {}", path.display());
            Ok(())
        }
        Ok(output) => {
            tracing::warn!(
                "⚠️  rustfmt 失败: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            Ok(()) // 格式化失败不应该阻塞生成流程
        }
        Err(e) => {
            tracing::warn!("⚠️  rustfmt 未找到或执行失败: {}", e);
            Ok(()) // 格式化失败不应该阻塞生成流程
        }
    }
}

/// 格式化 TypeScript 文件
fn format_typescript_file(path: &std::path::Path) -> Result<()> {
    use std::process::Command;

    // 尝试使用 prettier
    let output = Command::new("npx")
        .args(["prettier", "--write", path.to_str().unwrap()])
        .output();

    match output {
        Ok(output) if output.status.success() => {
            tracing::debug!("✅ 格式化 TypeScript 文件: {}", path.display());
            Ok(())
        }
        Ok(output) => {
            tracing::warn!(
                "⚠️  prettier 失败: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            Ok(())
        }
        Err(_) => {
            // prettier 不可用，尝试 dprint
            let output = Command::new("dprint")
                .args(["fmt", path.to_str().unwrap()])
                .output();

            match output {
                Ok(output) if output.status.success() => {
                    tracing::debug!("✅ 格式化 TypeScript 文件（dprint）: {}", path.display());
                    Ok(())
                }
                _ => {
                    tracing::warn!("⚠️  TypeScript 格式化工具未找到（prettier/dprint）");
                    Ok(())
                }
            }
        }
    }
}
