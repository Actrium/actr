//! [...]

use actr_web_protoc_codegen::{WebCodegen, WebCodegenConfig};
use anyhow::Result;
use std::path::PathBuf;
use tracing_subscriber;

fn main() -> Result<()> {
    // initializelog
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    println!("🚀 actr-web-protoc-codegen [...]\n");

    // [...]
    let project_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));

    // config[...]
    let config = WebCodegenConfig::builder()
        .proto_file(project_root.join("proto/user_service.proto"))
        .rust_output(project_root.join("generated-rust"))
        .ts_output(project_root.join("generated-ts"))
        .with_react_hooks(true)
        .with_formatting(false) // [...]，[...]
        .include(project_root.join("proto"))
        .build()?;

    println!("📋 config[...]:");
    println!("  Proto [...]: {:?}", config.proto_files);
    println!("  Rust [...]: {:?}", config.rust_output_dir);
    println!("  TypeScript [...]: {:?}", config.ts_output_dir);
    println!("  [...] React Hooks: {}", config.generate_react_hooks);
    println!();

    // [...]
    println!("🔄 [...]...\n");
    let codegen = WebCodegen::new(config);
    let files = codegen.generate()?;

    // [...]
    println!("\n✅ [...]！\n");
    println!("📊 [...]:");
    println!("  Rust [...]: {} [...]", files.rust_files.len());
    println!("  TypeScript [...]: {} [...]", files.ts_types.len());
    println!("  ActorRef [...]: {} [...]", files.ts_actor_refs.len());
    println!("  React Hooks: {} [...]", files.react_hooks.len());
    println!("  [...]: {} [...]", files.total_count());
    println!();

    // [...]
    println!("📁 [...]:\n");

    println!("Rust [...]:");
    for file in &files.rust_files {
        println!("  ✓ {}", file.path.display());
        println!("    {} [...]", file.content.lines().count());
    }
    println!();

    println!("TypeScript [...]:");
    for file in &files.ts_types {
        println!("  ✓ {}", file.path.display());
        println!("    {} [...]", file.content.lines().count());
    }
    println!();

    println!("TypeScript ActorRef:");
    for file in &files.ts_actor_refs {
        println!("  ✓ {}", file.path.display());
        println!("    {} [...]", file.content.lines().count());
    }
    println!();

    if !files.react_hooks.is_empty() {
        println!("React Hooks:");
        for file in &files.react_hooks {
            println!("  ✓ {}", file.path.display());
            println!("    {} [...]", file.content.lines().count());
        }
        println!();
    }

    // [...]
    println!("📝 [...]:\n");

    if let Some(rust_file) = files.rust_files.first() {
        println!("=== Rust [...] ===");
        println!("{}", rust_file.path.display());
        let lines: Vec<&str> = rust_file.content.lines().take(30).collect();
        for line in lines {
            println!("{}", line);
        }
        println!("... ([...] {} [...])", rust_file.content.lines().count().saturating_sub(30));
        println!();
    }

    if let Some(ts_file) = files.ts_types.first() {
        println!("=== TypeScript [...] ===");
        println!("{}", ts_file.path.display());
        let lines: Vec<&str> = ts_file.content.lines().take(30).collect();
        for line in lines {
            println!("{}", line);
        }
        println!("... ([...] {} [...])", ts_file.content.lines().count().saturating_sub(30));
        println!();
    }

    if let Some(actor_ref_file) = files.ts_actor_refs.first() {
        println!("=== TypeScript ActorRef [...] ===");
        println!("{}", actor_ref_file.path.display());
        let lines: Vec<&str> = actor_ref_file.content.lines().take(40).collect();
        for line in lines {
            println!("{}", line);
        }
        println!("... ([...] {} [...])", actor_ref_file.content.lines().count().saturating_sub(40));
        println!();
    }

    println!("🎉 [...]！[...]already[...]:");
    println!("  Rust: {}", project_root.join("generated-rust").display());
    println!("  TypeScript: {}", project_root.join("generated-ts").display());

    Ok(())
}
