//! # 代码生成命令
//!
//! 从 proto 文件生成 Rust Actor 代码，包括：
//! 1. protobuf 消息类型
//! 2. Actor 基础设施代码
//! 3. 用户业务逻辑框架（带 TODO 注释）

use crate::commands::Command;
use crate::error::{ActrCliError, Result};
// 只导入必要的类型，避免拉入不需要的依赖如 sqlite
// use actr_framework::prelude::*;
use async_trait::async_trait;
use clap::Parser;
use std::path::{Path, PathBuf};
use std::process::Command as StdCommand;
use tracing::{debug, info, warn};

#[derive(Parser, Debug, Clone)]
pub struct GenCommand {
    /// 输入的 proto 文件或目录
    #[arg(short, long, default_value = "proto")]
    pub input: PathBuf,

    /// 输出目录
    #[arg(short, long, default_value = "src/generated")]
    pub output: PathBuf,

    /// 是否生成用户代码框架
    #[arg(long, default_value = "true")]
    pub generate_scaffold: bool,

    /// 是否覆盖已存在的用户代码文件
    #[arg(long)]
    pub overwrite_user_code: bool,

    /// 是否运行 rustfmt 格式化生成的代码
    #[arg(long, default_value = "true")]
    pub format: bool,

    /// 调试模式：保留中间生成文件
    #[arg(long)]
    pub debug: bool,
}

#[async_trait]
impl Command for GenCommand {
    async fn execute(&self) -> Result<()> {
        info!("🚀 开始代码生成...");

        // 1. 验证输入
        self.validate_inputs()?;

        // 2. 准备输出目录
        self.prepare_output_dirs()?;

        // 3. 发现 proto 文件
        let proto_files = self.discover_proto_files()?;
        info!("📁 发现 {} 个 proto 文件", proto_files.len());

        // 4. 生成基础设施代码
        self.generate_infrastructure_code(&proto_files).await?;

        // 5. 生成用户代码框架
        if self.generate_scaffold {
            self.generate_user_code_scaffold(&proto_files).await?;
        }

        // 6. 格式化代码
        if self.format {
            self.format_generated_code().await?;
        }

        // 7. 验证生成的代码
        self.validate_generated_code().await?;

        info!("✅ 代码生成完成！");
        self.print_next_steps();

        Ok(())
    }
}

impl GenCommand {
    /// 读取 Actr.toml 中的 manufacturer
    fn read_manufacturer(&self) -> Result<String> {
        use std::fs;

        // Look for Actr.toml in current directory
        let config_path = PathBuf::from("Actr.toml");
        if !config_path.exists() {
            warn!("Actr.toml not found, using default manufacturer 'acme'");
            return Ok("acme".to_string());
        }

        // Read and parse TOML directly
        let content = fs::read_to_string(&config_path)
            .map_err(|e| ActrCliError::config_error(format!("Failed to read Actr.toml: {e}")))?;

        let raw_config: actr::config::RawConfig = toml::from_str(&content)
            .map_err(|e| ActrCliError::config_error(format!("Failed to parse Actr.toml: {e}")))?;

        Ok(raw_config.package.actr_type.manufacturer)
    }

    /// 验证输入参数
    fn validate_inputs(&self) -> Result<()> {
        if !self.input.exists() {
            return Err(ActrCliError::config_error(format!(
                "输入路径不存在: {:?}",
                self.input
            )));
        }

        if self.input.is_file() && self.input.extension().unwrap_or_default() != "proto" {
            warn!("输入文件不是 .proto 文件: {:?}", self.input);
        }

        Ok(())
    }

    /// 准备输出目录
    fn prepare_output_dirs(&self) -> Result<()> {
        std::fs::create_dir_all(&self.output)
            .map_err(|e| ActrCliError::config_error(format!("创建输出目录失败: {e}")))?;

        if self.generate_scaffold {
            let user_code_dir = self.output.join("../");
            std::fs::create_dir_all(&user_code_dir)
                .map_err(|e| ActrCliError::config_error(format!("创建用户代码目录失败: {e}")))?;
        }

        Ok(())
    }

    /// 发现 proto 文件
    fn discover_proto_files(&self) -> Result<Vec<PathBuf>> {
        let mut proto_files = Vec::new();

        if self.input.is_file() {
            proto_files.push(self.input.clone());
        } else {
            // 遍历目录查找 .proto 文件
            for entry in std::fs::read_dir(&self.input)
                .map_err(|e| ActrCliError::config_error(format!("读取输入目录失败: {e}")))?
            {
                let entry = entry.map_err(|e| ActrCliError::config_error(e.to_string()))?;
                let path = entry.path();

                if path.extension().unwrap_or_default() == "proto" {
                    proto_files.push(path);
                }
            }
        }

        if proto_files.is_empty() {
            return Err(ActrCliError::config_error("未找到 proto 文件"));
        }

        Ok(proto_files)
    }

    /// 确保 protoc-gen-actrframework 插件可用
    ///
    /// 版本管理策略：
    /// 1. 检查系统已安装版本
    /// 2. 如果版本匹配 → 直接使用
    /// 3. 如果版本不匹配或未安装 → 自动安装/升级
    ///
    /// 这种策略确保：
    /// - 版本一致性：插件版本始终与 CLI 匹配
    /// - 自动管理：无需手动安装或升级
    /// - 简单明确：只看版本，不区分开发/生产环境
    fn ensure_protoc_plugin(&self) -> Result<PathBuf> {
        // Expected version (same as actr-framework-protoc-codegen)
        const EXPECTED_VERSION: &str = env!("CARGO_PKG_VERSION");

        // 1. Check installed version
        let installed_version = self.check_installed_plugin_version()?;

        match installed_version {
            Some(version) if version == EXPECTED_VERSION => {
                // Version matches, use it directly
                info!("✅ Using installed protoc-gen-actrframework v{}", version);
                let output = StdCommand::new("which")
                    .arg("protoc-gen-actrframework")
                    .output()
                    .map_err(|e| {
                        ActrCliError::command_error(format!("Failed to locate plugin: {e}"))
                    })?;

                let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                Ok(PathBuf::from(path))
            }
            Some(version) => {
                // Version mismatch, upgrade needed
                info!(
                    "🔄 Version mismatch: installed v{}, need v{}",
                    version, EXPECTED_VERSION
                );
                info!("🔨 Upgrading plugin...");
                self.install_or_upgrade_plugin()
            }
            None => {
                // Not installed, install it
                info!("📦 protoc-gen-actrframework not found, installing...");
                self.install_or_upgrade_plugin()
            }
        }
    }

    /// Check installed plugin version
    fn check_installed_plugin_version(&self) -> Result<Option<String>> {
        let output = StdCommand::new("protoc-gen-actrframework")
            .arg("--version")
            .output();

        match output {
            Ok(output) if output.status.success() => {
                let version_info = String::from_utf8_lossy(&output.stdout);
                // Parse "protoc-gen-actrframework 0.1.0"
                let version = version_info
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .map(|v| v.to_string());

                debug!("Detected installed version: {:?}", version);
                Ok(version)
            }
            _ => {
                debug!("Plugin not found in PATH");
                Ok(None)
            }
        }
    }

    /// Install or upgrade plugin from workspace
    fn install_or_upgrade_plugin(&self) -> Result<PathBuf> {
        // Find actr workspace
        let current_dir = std::env::current_dir()?;
        let workspace_root = current_dir.ancestors().find(|p| {
            let is_workspace =
                p.join("Cargo.toml").exists() && p.join("crates/framework-protoc-codegen").exists();
            if is_workspace {
                debug!("Found workspace root: {:?}", p);
            }
            is_workspace
        });

        let workspace_root = workspace_root.ok_or_else(|| {
            ActrCliError::config_error(
                "Cannot find actr workspace.\n\
                 Please run this command from within an actr project or workspace.",
            )
        })?;

        info!("🔍 Found actr workspace at: {}", workspace_root.display());

        // Step 1: Build the plugin
        info!("🔨 Building protoc-gen-actrframework...");
        let mut build_cmd = StdCommand::new("cargo");
        build_cmd
            .arg("build")
            .arg("-p")
            .arg("actr-framework-protoc-codegen")
            .arg("--bin")
            .arg("protoc-gen-actrframework")
            .current_dir(workspace_root);

        debug!("Running: {:?}", build_cmd);
        let output = build_cmd
            .output()
            .map_err(|e| ActrCliError::command_error(format!("Failed to build plugin: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(ActrCliError::command_error(format!(
                "Failed to build plugin:\n{stderr}"
            )));
        }

        // Step 2: Install to ~/.cargo/bin/
        info!("📦 Installing to ~/.cargo/bin/...");
        let mut install_cmd = StdCommand::new("cargo");
        install_cmd
            .arg("install")
            .arg("--path")
            .arg(workspace_root.join("crates/framework-protoc-codegen"))
            .arg("--bin")
            .arg("protoc-gen-actrframework")
            .arg("--force"); // Overwrite existing version

        debug!("Running: {:?}", install_cmd);
        let output = install_cmd
            .output()
            .map_err(|e| ActrCliError::command_error(format!("Failed to install plugin: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(ActrCliError::command_error(format!(
                "Failed to install plugin:\n{stderr}"
            )));
        }

        info!("✅ Plugin installed successfully");

        // Return the installed path
        let which_output = StdCommand::new("which")
            .arg("protoc-gen-actrframework")
            .output()
            .map_err(|e| {
                ActrCliError::command_error(format!("Failed to locate installed plugin: {e}"))
            })?;

        let path = String::from_utf8_lossy(&which_output.stdout)
            .trim()
            .to_string();
        Ok(PathBuf::from(path))
    }

    /// 生成基础设施代码
    async fn generate_infrastructure_code(&self, proto_files: &[PathBuf]) -> Result<()> {
        info!("🔧 生成基础设施代码...");

        // 确保 protoc 插件可用
        let plugin_path = self.ensure_protoc_plugin()?;

        // 读取 Actr.toml 获取 manufacturer
        let manufacturer = self.read_manufacturer()?;
        debug!("Using manufacturer from Actr.toml: {}", manufacturer);

        for proto_file in proto_files {
            debug!("处理 proto 文件: {:?}", proto_file);

            // 第一步：使用 prost 生成基础 protobuf 消息类型
            let mut cmd = StdCommand::new("protoc");
            cmd.arg(format!("--proto_path={}", self.input.display()))
                .arg(format!("--prost_out={}", self.output.display()))
                .arg(proto_file);

            debug!("执行 protoc (prost): {:?}", cmd);
            let output = cmd.output().map_err(|e| {
                ActrCliError::command_error(format!("执行 protoc (prost) 失败: {e}"))
            })?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(ActrCliError::command_error(format!(
                    "protoc (prost) 执行失败: {stderr}"
                )));
            }

            // 第二步：使用 actrframework 插件生成 Actor 框架代码
            let mut cmd = StdCommand::new("protoc");
            cmd.arg(format!("--proto_path={}", self.input.display()))
                .arg(format!(
                    "--plugin=protoc-gen-actrframework={}",
                    plugin_path.display()
                ))
                .arg(format!("--actrframework_opt=manufacturer={manufacturer}"))
                .arg(format!("--actrframework_out={}", self.output.display()))
                .arg(proto_file);

            debug!("执行 protoc (actrframework): {:?}", cmd);
            let output = cmd.output().map_err(|e| {
                ActrCliError::command_error(format!("执行 protoc (actrframework) 失败: {e}"))
            })?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(ActrCliError::command_error(format!(
                    "protoc (actrframework) 执行失败: {stderr}"
                )));
            }

            let stdout = String::from_utf8_lossy(&output.stdout);
            if !stdout.is_empty() {
                debug!("protoc 输出: {}", stdout);
            }
        }

        // 生成 mod.rs
        self.generate_mod_rs(proto_files).await?;

        // 为生成的文件添加只读属性（防止误修改）
        self.set_generated_files_readonly()?;

        info!("✅ 基础设施代码生成完成");
        Ok(())
    }

    /// 生成 mod.rs 文件
    async fn generate_mod_rs(&self, _proto_files: &[PathBuf]) -> Result<()> {
        let mod_path = self.output.join("mod.rs");

        // 扫描实际生成的文件，而不是根据 proto 文件名猜测
        let mut proto_modules = Vec::new();
        let mut service_modules = Vec::new();

        use std::fs;
        for entry in fs::read_dir(&self.output)
            .map_err(|e| ActrCliError::config_error(format!("读取输出目录失败: {e}")))?
        {
            let entry = entry.map_err(|e| ActrCliError::config_error(e.to_string()))?;
            let path = entry.path();

            if path.is_file() && path.extension().unwrap_or_default() == "rs" {
                if let Some(file_name) = path.file_stem().and_then(|s| s.to_str()) {
                    // 跳过 mod.rs 本身
                    if file_name == "mod" {
                        continue;
                    }

                    // 区分 service_actor 文件和 proto 文件
                    if file_name.ends_with("_service_actor") {
                        service_modules.push(format!("pub mod {file_name};"));
                    } else {
                        proto_modules.push(format!("pub mod {file_name};"));
                    }
                }
            }
        }

        // 排序以保证生成的 mod.rs 内容稳定
        proto_modules.sort();
        service_modules.sort();

        let mod_content = format!(
            r#"//! 自动生成的代码模块
//!
//! 此模块由 `actr gen` 命令自动生成，包括：
//! - protobuf 消息类型定义
//! - Actor 框架代码（路由器、trait）
//!
//! ⚠️  请勿手动修改此目录中的文件

// Protobuf 消息类型（由 prost 生成）
{}

// Actor 框架代码（由 protoc-gen-actrframework 生成）
{}

// 常用类型会在各自的模块中定义，请按需导入
"#,
            proto_modules.join("\n"),
            service_modules.join("\n"),
        );

        std::fs::write(&mod_path, mod_content)
            .map_err(|e| ActrCliError::config_error(format!("写入 mod.rs 失败: {e}")))?;

        debug!("生成 mod.rs: {:?}", mod_path);
        Ok(())
    }

    /// 将生成目录中的文件设置为只读
    fn set_generated_files_readonly(&self) -> Result<()> {
        use std::fs;

        for entry in fs::read_dir(&self.output)
            .map_err(|e| ActrCliError::config_error(format!("读取输出目录失败: {e}")))?
        {
            let entry = entry.map_err(|e| ActrCliError::config_error(e.to_string()))?;
            let path = entry.path();

            if path.is_file() && path.extension().unwrap_or_default() == "rs" {
                // 获取当前权限
                let metadata = fs::metadata(&path)
                    .map_err(|e| ActrCliError::config_error(format!("获取文件元数据失败: {e}")))?;
                let mut permissions = metadata.permissions();

                // 设置只读（移除写权限）
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let mode = permissions.mode();
                    permissions.set_mode(mode & !0o222); // 移除所有写权限
                }

                #[cfg(not(unix))]
                {
                    permissions.set_readonly(true);
                }

                fs::set_permissions(&path, permissions)
                    .map_err(|e| ActrCliError::config_error(format!("设置文件权限失败: {e}")))?;

                debug!("设置只读属性: {:?}", path);
            }
        }

        Ok(())
    }

    /// 生成用户代码框架
    async fn generate_user_code_scaffold(&self, proto_files: &[PathBuf]) -> Result<()> {
        info!("📝 生成用户代码框架...");

        for proto_file in proto_files {
            let service_name = proto_file
                .file_stem()
                .and_then(|s| s.to_str())
                .ok_or_else(|| ActrCliError::config_error("无效的 proto 文件名"))?;

            self.generate_service_scaffold(service_name).await?;
        }

        info!("✅ 用户代码框架生成完成");
        Ok(())
    }

    /// 为特定服务生成用户代码框架
    async fn generate_service_scaffold(&self, service_name: &str) -> Result<()> {
        let user_file_path = self
            .output
            .parent()
            .unwrap_or_else(|| Path::new("src"))
            .join(format!("{}_service.rs", service_name.to_lowercase()));

        // 如果文件已存在且不强制覆盖，跳过
        if user_file_path.exists() && !self.overwrite_user_code {
            info!("⏭️  跳过已存在的用户代码文件: {:?}", user_file_path);
            return Ok(());
        }

        let scaffold_content = self.generate_scaffold_content(service_name);

        std::fs::write(&user_file_path, scaffold_content)
            .map_err(|e| ActrCliError::config_error(format!("写入用户代码框架失败: {e}")))?;

        info!("📄 生成用户代码框架: {:?}", user_file_path);
        Ok(())
    }

    /// 生成用户代码框架内容
    fn generate_scaffold_content(&self, service_name: &str) -> String {
        let service_name_pascal = service_name
            .split('_')
            .map(|s| {
                let mut chars = s.chars();
                match chars.next() {
                    None => String::new(),
                    Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                }
            })
            .collect::<String>();

        let template = format!(
            r#"//! # {service_name_pascal} 用户业务逻辑实现
//!
//! 这个文件是由 `actr gen` 命令自动生成的用户代码框架。
//! 请在这里实现您的具体业务逻辑。

use crate::generated::{{{service_name_pascal}Handler, {service_name_pascal}Actor}};
// 只导入必要的类型，避免拉入不需要的依赖如 sqlite
// use actr_framework::prelude::*;
use std::sync::Arc;

/// {service_name_pascal} 服务的具体实现
/// 
/// TODO: 添加您需要的状态字段，例如：
/// - 数据库连接池
/// - 配置信息
/// - 缓存客户端
/// - 日志记录器等
pub struct My{service_name_pascal}Service {{
    // TODO: 添加您的服务状态字段
    // 例如：
    // pub db_pool: Arc<DatabasePool>,
    // pub config: Arc<ServiceConfig>,
    // pub metrics: Arc<Metrics>,
}}

impl My{service_name_pascal}Service {{
    /// 创建新的服务实例
    /// 
    /// TODO: 根据您的需要修改构造函数参数
    pub fn new(/* TODO: 添加必要的依赖 */) -> Self {{
        Self {{
            // TODO: 初始化您的字段
        }}
    }}
    
    /// 使用默认配置创建服务实例（用于测试）
    pub fn default_for_testing() -> Self {{
        Self {{
            // TODO: 提供测试用的默认值
        }}
    }}
}}

// TODO: 实现 {service_name_pascal}Handler trait 的所有方法
// 注意：impl_user_code_scaffold! 宏已经为您生成了基础框架，
// 您需要将其替换为真实的业务逻辑实现。
//
// 示例：
// #[async_trait]
// impl {service_name_pascal}Handler for My{service_name_pascal}Service {{
//     async fn method_name(&self, req: RequestType) -> ActorResult<ResponseType> {{
//         // 1. 验证输入
//         // 2. 执行业务逻辑
//         // 3. 返回结果
//         todo!("实现您的业务逻辑")
//     }}
// }}

#[cfg(test)]
mod tests {{
    use super::*;
    
    #[tokio::test]
    async fn test_service_creation() {{
        let _service = My{service_name_pascal}Service::default_for_testing();
        // TODO: 添加您的测试
    }}
    
    // TODO: 添加更多测试用例
}}

/*
📚 使用指南

## 🚀 快速开始

1. **实现业务逻辑**：
   在 `My{service_name_pascal}Service` 中实现 `{service_name_pascal}Handler` trait 的所有方法

2. **添加依赖**：
   在 `Cargo.toml` 中添加您需要的依赖，例如数据库客户端、HTTP 客户端等

3. **配置服务**：
   修改 `new()` 构造函数，注入必要的依赖

4. **启动服务**：
   ```rust
   #[tokio::main]
   async fn main() -> ActorResult<()> {{
       let service = My{service_name_pascal}Service::new(/* 依赖 */);
       
       ActorSystem::new()
           .attach(service)
           .start()
           .await
   }}
   ```

## 🔧 开发提示

- 使用 `tracing` crate 进行日志记录
- 实现错误处理和重试逻辑
- 添加单元测试和集成测试
- 考虑使用配置文件管理环境变量
- 实现健康检查和指标收集

## 📖 更多资源

- Actor-RTC 文档: [链接]
- API 参考: [链接]
- 示例项目: [链接]
*/
"# // 示例代码中的 Service
        );

        template
    }

    /// 格式化生成的代码
    async fn format_generated_code(&self) -> Result<()> {
        info!("🎨 格式化生成的代码...");

        let mut cmd = StdCommand::new("rustfmt");
        cmd.arg("--edition")
            .arg("2024")
            .arg("--config")
            .arg("max_width=100");

        // 格式化生成目录中的所有 .rs 文件
        for entry in std::fs::read_dir(&self.output)
            .map_err(|e| ActrCliError::config_error(format!("读取输出目录失败: {e}")))?
        {
            let entry = entry.map_err(|e| ActrCliError::config_error(e.to_string()))?;
            let path = entry.path();

            if path.extension().unwrap_or_default() == "rs" {
                cmd.arg(&path);
            }
        }

        let output = cmd
            .output()
            .map_err(|e| ActrCliError::command_error(format!("执行 rustfmt 失败: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            warn!("rustfmt 执行警告: {}", stderr);
        } else {
            info!("✅ 代码格式化完成");
        }

        Ok(())
    }

    /// 验证生成的代码
    async fn validate_generated_code(&self) -> Result<()> {
        info!("🔍 验证生成的代码...");

        // 查找项目根目录（包含 Cargo.toml 的目录）
        let project_root = self.find_project_root()?;

        let mut cmd = StdCommand::new("cargo");
        cmd.arg("check").arg("--quiet").current_dir(&project_root);

        let output = cmd
            .output()
            .map_err(|e| ActrCliError::command_error(format!("执行 cargo check 失败: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            warn!("生成的代码存在编译警告或错误:\n{}", stderr);
            info!("💡 这通常是正常的，因为用户代码框架包含 TODO 标记");
        } else {
            info!("✅ 代码验证通过");
        }

        Ok(())
    }

    /// 查找项目根目录（包含 Cargo.toml 的目录）
    fn find_project_root(&self) -> Result<PathBuf> {
        let mut current = std::env::current_dir().map_err(ActrCliError::Io)?;

        loop {
            if current.join("Cargo.toml").exists() {
                return Ok(current);
            }

            match current.parent() {
                Some(parent) => current = parent.to_path_buf(),
                None => break,
            }
        }

        // 如果找不到 Cargo.toml，回退到当前目录
        std::env::current_dir().map_err(ActrCliError::Io)
    }

    /// 打印后续步骤提示
    fn print_next_steps(&self) {
        println!("\n🎉 代码生成完成！");
        println!("\n📋 后续步骤：");
        println!("1. 📖 查看生成的代码: {:?}", self.output);
        if self.generate_scaffold {
            println!("2. ✏️  实现业务逻辑: 在 src/ 目录下的 *_service.rs 文件中");
            println!("3. 🔧 添加依赖: 在 Cargo.toml 中添加需要的依赖包");
        }
        println!("4. 🏗️  编译项目: cargo build");
        println!("5. 🧪 运行测试: cargo test");
        println!("6. 🚀 启动服务: cargo run");
        println!("\n💡 提示: 查看生成的用户代码文件中的详细使用指南");
    }
}
