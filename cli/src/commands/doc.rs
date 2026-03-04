//! Doc command implementation - generate project documentation

use crate::commands::Command;
use crate::error::Result;
use actr::config::{Config, ConfigParser};
use async_trait::async_trait;
use clap::Args;
use std::path::Path;
use tracing::{debug, info};

#[derive(Args)]
pub struct DocCommand {
    /// Output directory for documentation (defaults to "./docs")
    #[arg(short = 'o', long = "output")]
    pub output_dir: Option<String>,
}

#[async_trait]
impl Command for DocCommand {
    async fn execute(&self) -> Result<()> {
        let output_dir = self.output_dir.as_deref().unwrap_or("docs");

        info!("📚 Generating project documentation to: {}", output_dir);

        // Create output directory
        std::fs::create_dir_all(output_dir)?;

        // Load project configuration
        let config = if Path::new("Actr.toml").exists() {
            Some(ConfigParser::from_file("Actr.toml")?)
        } else {
            None
        };

        // Generate documentation files
        self.generate_index_html(output_dir, &config).await?;
        self.generate_api_html(output_dir, &config).await?;
        self.generate_config_html(output_dir, &config).await?;

        info!("✅ Documentation generated successfully");
        info!("📄 Generated files:");
        info!("  - {}/index.html (project overview)", output_dir);
        info!("  - {}/api.html (API interface documentation)", output_dir);
        info!(
            "  - {}/config.html (configuration documentation)",
            output_dir
        );

        Ok(())
    }
}

impl DocCommand {
    /// Generate project overview documentation
    async fn generate_index_html(&self, output_dir: &str, config: &Option<Config>) -> Result<()> {
        debug!("Generating index.html...");

        let project_name = config
            .as_ref()
            .map(|c| c.package.name.as_str())
            .unwrap_or("Actor-RTC Project");
        // Note: package.version doesn't exist in new API, use default or read from Cargo.toml
        let project_version = "0.1.0";
        let project_description = config
            .as_ref()
            .and_then(|c| c.package.description.as_ref())
            .map(|s| s.as_str())
            .unwrap_or("An Actor-RTC project");

        let html_content = format!(
            r#"<!DOCTYPE html>
<html lang="zh">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>{project_name} - 项目概览</title>
    <style>
        body {{ font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif; margin: 0; padding: 20px; line-height: 1.6; }}
        .header {{ background: linear-gradient(135deg, #667eea 0%, #764ba2 100%); color: white; padding: 20px; border-radius: 8px; margin-bottom: 20px; }}
        .content {{ max-width: 800px; margin: 0 auto; }}
        .section {{ background: white; padding: 20px; margin: 20px 0; border-radius: 8px; box-shadow: 0 2px 10px rgba(0,0,0,0.1); }}
        .nav {{ display: flex; gap: 10px; margin: 20px 0; }}
        .nav a {{ padding: 10px 20px; background: #f0f0f0; text-decoration: none; color: #333; border-radius: 4px; }}
        .nav a:hover {{ background: #667eea; color: white; }}
        h1, h2 {{ margin-top: 0; }}
        .badge {{ background: #667eea; color: white; padding: 4px 8px; border-radius: 4px; font-size: 0.8em; }}
    </style>
</head>
<body>
    <div class="content">
        <div class="header">
            <h1>{project_name}</h1>
            <p>{project_description}</p>
            <span class="badge">v{project_version}</span>
        </div>
        
        <div class="nav">
            <a href="index.html">项目概览</a>
            <a href="api.html">API 文档</a>
            <a href="config.html">配置说明</a>
        </div>
        
        <div class="section">
            <h2>📋 项目信息</h2>
            <p><strong>名称:</strong> {project_name}</p>
            <p><strong>版本:</strong> {project_version}</p>
            <p><strong>描述:</strong> {project_description}</p>
        </div>
        
        <div class="section">
            <h2>🚀 快速开始</h2>
            <p>这是一个基于 Actor-RTC 框架的项目。以下是一些常用的开发命令：</p>
            <pre><code># 生成代码
actr gen --input proto --output src/generated

# 运行项目
actr run

# 安装依赖
actr install

# 检查配置
actr check</code></pre>
        </div>
        
        <div class="section">
            <h2>📁 项目结构</h2>
            <pre><code>{project_name}/ 
├── Actr.toml          # 项目配置文件
├── src/               # 源代码目录
│   ├── main.rs        # 程序入口点
│   └── generated/     # 自动生成的代码
├── proto/             # Protocol Buffers 定义
└── docs/              # 项目文档</code></pre>
        </div>
        
        <div class="section">
            <h2>🔗 相关链接</h2>
            <ul>
                <li><a href="api.html">API 接口文档</a> - 查看服务接口定义</li>
                <li><a href="config.html">配置说明</a> - 了解项目配置选项</li>
            </ul>
        </div>
    </div>
</body>
</html>"#
        );

        let index_path = Path::new(output_dir).join("index.html");
        std::fs::write(index_path, html_content)?;

        Ok(())
    }

    /// Generate API documentation
    async fn generate_api_html(&self, output_dir: &str, config: &Option<Config>) -> Result<()> {
        debug!("Generating api.html...");

        let project_name = config
            .as_ref()
            .map(|c| c.package.name.as_str())
            .unwrap_or("Actor-RTC Project");

        // Collect proto files information
        let mut proto_info = Vec::new();
        let proto_dir = Path::new("proto");

        if proto_dir.exists() {
            if let Ok(entries) = std::fs::read_dir(proto_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().and_then(|s| s.to_str()) == Some("proto") {
                        let filename = path.file_name().unwrap().to_string_lossy();
                        let content = std::fs::read_to_string(&path).unwrap_or_default();
                        proto_info.push((filename.to_string(), content));
                    }
                }
            }
        }

        let mut proto_sections = String::new();
        if proto_info.is_empty() {
            proto_sections.push_str(
                r#"<div class="section">
                <p>暂无 Protocol Buffers 定义文件。</p>
            </div>"#,
            );
        } else {
            for (filename, content) in proto_info {
                proto_sections.push_str(&format!(
                    r#"<div class="section">
                    <h3>📄 {}</h3>
                    <pre><code>{}</code></pre>
                </div>"#,
                    filename,
                    Self::html_escape(&content)
                ));
            }
        }

        let html_content = format!(
            r#"<!DOCTYPE html>
<html lang="zh">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>{project_name} - API 文档</title>
    <style>
        body {{ font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif; margin: 0; padding: 20px; line-height: 1.6; }}
        .header {{ background: linear-gradient(135deg, #667eea 0%, #764ba2 100%); color: white; padding: 20px; border-radius: 8px; margin-bottom: 20px; }}
        .content {{ max-width: 1000px; margin: 0 auto; }}
        .section {{ background: white; padding: 20px; margin: 20px 0; border-radius: 8px; box-shadow: 0 2px 10px rgba(0,0,0,0.1); }}
        .nav {{ display: flex; gap: 10px; margin: 20px 0; }}
        .nav a {{ padding: 10px 20px; background: #f0f0f0; text-decoration: none; color: #333; border-radius: 4px; }}
        .nav a:hover {{ background: #667eea; color: white; }}
        .nav a.active {{ background: #667eea; color: white; }}
        h1, h2, h3 {{ margin-top: 0; }}
        pre {{ background: #f5f5f5; padding: 15px; border-radius: 4px; overflow-x: auto; }}
        code {{ font-family: 'Monaco', 'Consolas', monospace; }}
    </style>
</head>
<body>
    <div class="content">
        <div class="header">
            <h1>{project_name} - API 接口文档</h1>
            <p>服务接口定义和协议规范</p>
        </div>
        
        <div class="nav">
            <a href="index.html">项目概览</a>
            <a href="api.html" class="active">API 文档</a>
            <a href="config.html">配置说明</a>
        </div>
        
        <div class="section">
            <h2>📋 Protocol Buffers 定义</h2>
            <p>以下是项目中定义的 Protocol Buffers 文件：</p>
        </div>
        
        {proto_sections}
    </div>
</body>
</html>"#
        );

        let api_path = Path::new(output_dir).join("api.html");
        std::fs::write(api_path, html_content)?;

        Ok(())
    }

    /// Generate configuration documentation
    async fn generate_config_html(&self, output_dir: &str, config: &Option<Config>) -> Result<()> {
        debug!("Generating config.html...");

        let project_name = config
            .as_ref()
            .map(|c| c.package.name.as_str())
            .unwrap_or("Actor-RTC Project");

        // Generate configuration example
        // Note: Config doesn't implement Serialize, read raw Actr.toml instead
        let config_example = if Path::new("Actr.toml").exists() {
            std::fs::read_to_string("Actr.toml").unwrap_or_default()
        } else {
            r#"[project]
name = "my-actor-service"
version = "0.1.0"
description = "An example Actor-RTC service"

[build]
output_dir = "generated"

[dependencies]
# Add your proto dependencies here

[system.signaling]
url = "ws://localhost:8081"

[scripts]
run = "cargo run"
build = "cargo build"
test = "cargo test""#
                .to_string()
        };

        let html_content = format!(
            r#"<!DOCTYPE html>
<html lang="zh">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>{} - 配置说明</title>
    <style>
        body {{ font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif; margin: 0; padding: 20px; line-height: 1.6; }}
        .header {{ background: linear-gradient(135deg, #667eea 0%, #764ba2 100%); color: white; padding: 20px; border-radius: 8px; margin-bottom: 20px; }}
        .content {{ max-width: 1000px; margin: 0 auto; }}
        .section {{ background: white; padding: 20px; margin: 20px 0; border-radius: 8px; box-shadow: 0 2px 10px rgba(0,0,0,0.1); }}
        .nav {{ display: flex; gap: 10px; margin: 20px 0; }}
        .nav a {{ padding: 10px 20px; background: #f0f0f0; text-decoration: none; color: #333; border-radius: 4px; }}
        .nav a:hover {{ background: #667eea; color: white; }}
        .nav a.active {{ background: #667eea; color: white; }}
        h1, h2, h3 {{ margin-top: 0; }}
        pre {{ background: #f5f5f5; padding: 15px; border-radius: 4px; overflow-x: auto; }}
        code {{ font-family: 'Monaco', 'Consolas', monospace; background: #f0f0f0; padding: 2px 4px; border-radius: 2px; }}
        .config-table {{ width: 100%; border-collapse: collapse; margin: 15px 0; }}
        .config-table th, .config-table td {{ border: 1px solid #ddd; padding: 12px; text-align: left; }}
        .config-table th {{ background: #f5f5f5; font-weight: bold; }}
    </style>
</head>
<body>
    <div class="content">
        <div class="header">
            <h1>{} - 配置说明</h1>
            <p>项目配置选项和使用说明</p>
        </div>
        
        <div class="nav">
            <a href="index.html">项目概览</a>
            <a href="api.html">API 文档</a>
            <a href="config.html" class="active">配置说明</a>
        </div>
        
        <div class="section">
            <h2>📋 配置文件结构</h2>
            <p><code>Actr.toml</code> 是项目的核心配置文件，包含以下主要部分：</p>
            
            <table class="config-table">
                <tr>
                    <th>配置段</th>
                    <th>作用</th>
                    <th>必需</th>
                </tr>
                <tr>
                    <td><code>[project]</code></td>
                    <td>项目基本信息（名称、版本、描述等）</td>
                    <td>是</td>
                </tr>
                <tr>
                    <td><code>[build]</code></td>
                    <td>构建配置（输出目录等）</td>
                    <td>是</td>
                </tr>
                <tr>
                    <td><code>[dependencies]</code></td>
                    <td>Protocol Buffers 依赖定义</td>
                    <td>否</td>
                </tr>
                <tr>
                    <td><code>[system.signaling]</code></td>
                    <td>信令服务器配置</td>
                    <td>否</td>
                </tr>
                <tr>
                    <td><code>[system.routing]</code></td>
                    <td>高级路由规则配置</td>
                    <td>否</td>
                </tr>
                <tr>
                    <td><code>[scripts]</code></td>
                    <td>自定义脚本命令</td>
                    <td>否</td>
                </tr>
            </table>
        </div>
        
        <div class="section">
            <h2>⚙️ 配置示例</h2>
            <pre><code>{}</code></pre>
        </div>
        
        <div class="section">
            <h2>🔧 配置管理命令</h2>
            <p>使用 <code>actr config</code> 命令可以方便地管理项目配置：</p>
            <pre><code># 设置配置值
actr config set project.description "我的Actor服务"
actr config set system.signaling.url "wss://signal.example.com"

# 查看配置值
actr config get project.name
actr config list

# 查看完整配置
actr config show

# 删除配置项
actr config unset system.signaling.url</code></pre>
        </div>
        
        <div class="section">
            <h2>📝 依赖配置</h2>
            <p>在 <code>[dependencies]</code> 段中配置 Protocol Buffers 依赖：</p>
            <pre><code># 本地文件路径
user_service = "proto/user.proto"

# HTTP URL
api_service = "https://example.com/api/service.proto"

# Actor 注册表 
[dependencies.payment]
uri = "actr://payment-service/payment.proto"
fingerprint = "sha256:a1b2c3d4..."</code></pre>
        </div>
    </div>
</body>
</html>"#,
            project_name,
            project_name,
            Self::html_escape(&config_example)
        );

        let config_path = Path::new(output_dir).join("config.html");
        std::fs::write(config_path, html_content)?;

        Ok(())
    }

    /// Simple HTML escape function
    fn html_escape(text: &str) -> String {
        text.replace("&", "&amp;")
            .replace("<", "&lt;")
            .replace(">", "&gt;")
            .replace("\"", "&quot;")
            .replace("'", "&#x27;")
    }
}
