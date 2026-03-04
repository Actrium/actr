//! 代码生成器完整功能测试

use actr_web_protoc_codegen::{WebCodegen, WebCodegenConfig};
use anyhow::Result;
use std::path::PathBuf;
use tracing_subscriber;

fn main() -> Result<()> {
    // 初始化日志
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    println!("🚀 actr-web-protoc-codegen 完整功能测试\n");

    // 获取项目根目录
    let project_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));

    // 配置代码生成器
    let config = WebCodegenConfig::builder()
        .proto_file(project_root.join("proto/user_service.proto"))
        .rust_output(project_root.join("generated-rust"))
        .ts_output(project_root.join("generated-ts"))
        .with_react_hooks(true)
        .with_formatting(false) // 暂时禁用格式化，以便查看原始输出
        .include(project_root.join("proto"))
        .build()?;

    println!("📋 配置信息:");
    println!("  Proto 文件: {:?}", config.proto_files);
    println!("  Rust 输出: {:?}", config.rust_output_dir);
    println!("  TypeScript 输出: {:?}", config.ts_output_dir);
    println!("  生成 React Hooks: {}", config.generate_react_hooks);
    println!();

    // 执行代码生成
    println!("🔄 开始生成代码...\n");
    let codegen = WebCodegen::new(config);
    let files = codegen.generate()?;

    // 显示生成结果
    println!("\n✅ 代码生成完成！\n");
    println!("📊 生成统计:");
    println!("  Rust 文件: {} 个", files.rust_files.len());
    println!("  TypeScript 类型: {} 个", files.ts_types.len());
    println!("  ActorRef 类: {} 个", files.ts_actor_refs.len());
    println!("  React Hooks: {} 个", files.react_hooks.len());
    println!("  总计: {} 个文件", files.total_count());
    println!();

    // 列出所有生成的文件
    println!("📁 生成的文件列表:\n");

    println!("Rust 文件:");
    for file in &files.rust_files {
        println!("  ✓ {}", file.path.display());
        println!("    {} 行", file.content.lines().count());
    }
    println!();

    println!("TypeScript 类型:");
    for file in &files.ts_types {
        println!("  ✓ {}", file.path.display());
        println!("    {} 行", file.content.lines().count());
    }
    println!();

    println!("TypeScript ActorRef:");
    for file in &files.ts_actor_refs {
        println!("  ✓ {}", file.path.display());
        println!("    {} 行", file.content.lines().count());
    }
    println!();

    if !files.react_hooks.is_empty() {
        println!("React Hooks:");
        for file in &files.react_hooks {
            println!("  ✓ {}", file.path.display());
            println!("    {} 行", file.content.lines().count());
        }
        println!();
    }

    // 显示部分生成内容预览
    println!("📝 生成内容预览:\n");

    if let Some(rust_file) = files.rust_files.first() {
        println!("=== Rust 代码示例 ===");
        println!("{}", rust_file.path.display());
        let lines: Vec<&str> = rust_file.content.lines().take(30).collect();
        for line in lines {
            println!("{}", line);
        }
        println!("... (省略 {} 行)", rust_file.content.lines().count().saturating_sub(30));
        println!();
    }

    if let Some(ts_file) = files.ts_types.first() {
        println!("=== TypeScript 类型示例 ===");
        println!("{}", ts_file.path.display());
        let lines: Vec<&str> = ts_file.content.lines().take(30).collect();
        for line in lines {
            println!("{}", line);
        }
        println!("... (省略 {} 行)", ts_file.content.lines().count().saturating_sub(30));
        println!();
    }

    if let Some(actor_ref_file) = files.ts_actor_refs.first() {
        println!("=== TypeScript ActorRef 示例 ===");
        println!("{}", actor_ref_file.path.display());
        let lines: Vec<&str> = actor_ref_file.content.lines().take(40).collect();
        for line in lines {
            println!("{}", line);
        }
        println!("... (省略 {} 行)", actor_ref_file.content.lines().count().saturating_sub(40));
        println!();
    }

    println!("🎉 测试完成！所有文件已生成到:");
    println!("  Rust: {}", project_root.join("generated-rust").display());
    println!("  TypeScript: {}", project_root.join("generated-ts").display());

    Ok(())
}
