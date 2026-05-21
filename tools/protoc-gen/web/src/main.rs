//! protoc-gen-actr-web — dual-mode binary
//!
//! Mode 1 (default): protoc plugin protocol — reads `CodeGeneratorRequest`
//! from stdin (binary protobuf), writes `CodeGeneratorResponse` to stdout.
//!
//! Mode 2 (`--generate`): CLI code-generation mode — reads a JSON
//! `WebCodegenRequest` from stdin, writes a JSON `WebCodegenResponse`
//! to stdout; used by `actr gen -l web`.

use actr_web_protoc_codegen::{WebCodegen, WebCodegenConfig, WebCodegenRequest, descriptor};
use prost::Message;
use prost_types::compiler::CodeGeneratorRequest;
use std::io::{self, Read, Write};
use std::path::PathBuf;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();

    if args.iter().any(|a| a == "--generate") {
        return run_codegen_mode();
    }

    // Default: protoc plugin mode
    run_protoc_mode()
}

/// Mode 2: full code generation from a JSON request on stdin.
fn run_codegen_mode() -> anyhow::Result<()> {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input)?;

    let request: WebCodegenRequest = serde_json::from_str(&input)
        .map_err(|e| anyhow::anyhow!("Failed to parse WebCodegenRequest: {e}"))?;

    let response = actr_web_protoc_codegen::generate(&request);

    let json = serde_json::to_string(&response)?;
    io::stdout().write_all(json.as_bytes())?;
    Ok(())
}

/// Mode 1: standard protoc plugin binary protocol.
///
/// Because `protoc` hands us the already-parsed `FileDescriptorProto` for
/// every proto file, this mode skips the `parse_proto_files` path entirely
/// (no second `protoc` invocation, no temp files) and feeds the descriptors
/// straight into the codegen pipeline.
fn run_protoc_mode() -> anyhow::Result<()> {
    let mut input = Vec::new();
    io::stdin().read_to_end(&mut input)?;

    let request = CodeGeneratorRequest::decode(&input[..])?;
    let params = parse_parameters(request.parameter.as_deref().unwrap_or(""));

    // Build `ProtoService` entries directly from the descriptors for the
    // files the caller asked us to generate.
    let mut services = Vec::new();
    for proto_file in &request.proto_file {
        let name = match &proto_file.name {
            Some(n) => n,
            None => continue,
        };
        if !request.file_to_generate.contains(name) {
            continue;
        }
        match descriptor::file_to_proto_service(proto_file) {
            Some(svc) => services.push(svc),
            None => {
                // No service declared in this file. Nothing to emit from it;
                // protoc plugin contract requires we still return cleanly.
                eprintln!("[protoc-gen-actr-web] {name}: no service found, skipping");
            }
        }
    }

    // The config is consumed only for output paths and flags; proto files
    // are not re-parsed on this branch.
    let config = WebCodegenConfig {
        proto_files: Vec::new(),
        rust_output_dir: PathBuf::from("/tmp/unused"),
        ts_output_dir: params.output_dir.clone(),
        generate_react_hooks: params.react_hooks,
        includes: Vec::new(),
        format_code: false, // Formatting is the caller's responsibility.
        custom_templates_dir: None,
    };

    let codegen = WebCodegen::new(config);
    let generated_files = codegen.generate_typescript_from_services(&services)?;

    let mut response = prost_types::compiler::CodeGeneratorResponse::default();
    for file in generated_files {
        let mut gen_file = prost_types::compiler::code_generator_response::File::default();
        let relative_path = file
            .path
            .strip_prefix(&params.output_dir)
            .unwrap_or(&file.path);
        gen_file.name = Some(relative_path.to_string_lossy().to_string());
        gen_file.content = Some(file.content);
        response.file.push(gen_file);
    }

    let mut output = Vec::new();
    response.encode(&mut output)?;
    io::stdout().write_all(&output)?;

    Ok(())
}

struct PluginParams {
    output_dir: PathBuf,
    react_hooks: bool,
}

fn parse_parameters(params: &str) -> PluginParams {
    let mut output_dir = PathBuf::from(".");
    let mut react_hooks = false;

    for param in params.split(',') {
        if let Some(value) = param.strip_prefix("output=") {
            output_dir = value.into();
        } else if param == "react_hooks" {
            react_hooks = true;
        }
    }

    PluginParams {
        output_dir,
        react_hooks,
    }
}
