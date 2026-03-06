//! protoc-gen-actr-web — dual-mode binary
//!
//! Mode 1 (default): protoc plugin protocol — reads `CodeGeneratorRequest` from
//! stdin (binary protobuf), writes `CodeGeneratorResponse` to stdout.
//!
//! Mode 2 (`--generate`): CLI code-generation mode — reads a JSON
//! `WebCodegenRequest` from stdin, writes a JSON `WebCodegenResponse`
//! to stdout; used by `actr gen -l web`.

use actr_web_protoc_codegen::{WebCodegen, WebCodegenConfig, WebCodegenRequest};
use prost::Message;
use prost_types::compiler::CodeGeneratorRequest;
use std::fs;
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

    let response = actr_web_protoc_codegen::codegen::generate(&request);

    let json = serde_json::to_string(&response)?;
    io::stdout().write_all(json.as_bytes())?;
    Ok(())
}

/// Mode 1: standard protoc plugin binary protocol.
fn run_protoc_mode() -> anyhow::Result<()> {
    // Read CodeGeneratorRequest from stdin (protoc sends this)
    let mut input = Vec::new();
    io::stdin().read_to_end(&mut input)?;

    let request = CodeGeneratorRequest::decode(&input[..])?;

    // Parse parameters
    let params = parse_parameters(request.parameter.as_deref().unwrap_or(""));

    // Create a temporary directory for proto files
    let temp_dir = std::env::temp_dir().join(format!("protoc-gen-actr-web-{}", std::process::id()));
    fs::create_dir_all(&temp_dir)?;

    // Write proto files to temp directory
    let mut proto_file_paths = Vec::new();
    for proto_file in &request.proto_file {
        if let Some(name) = &proto_file.name {
            // Only process files that are in the files_to_generate list
            if !request.file_to_generate.contains(name) {
                continue;
            }

            // Reconstruct proto file content from descriptor
            let proto_content = reconstruct_proto_content(proto_file);

            let file_path = temp_dir.join(name);
            if let Some(parent) = file_path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&file_path, proto_content)?;
            proto_file_paths.push(file_path);
        }
    }

    // Generate TypeScript code using actr-web-protoc-codegen
    let config = WebCodegenConfig {
        proto_files: proto_file_paths,
        rust_output_dir: PathBuf::from("/tmp/unused"),
        ts_output_dir: params.output_dir.clone(),
        generate_react_hooks: params.react_hooks,
        includes: vec![temp_dir.clone()],
        format_code: false, // Disable formatting in plugin
        custom_templates_dir: None,
    };

    let codegen = WebCodegen::new(config);
    let generated_files = codegen.generate_typescript_only()?;

    // Create CodeGeneratorResponse
    let mut response = prost_types::compiler::CodeGeneratorResponse::default();

    for file in generated_files {
        let mut gen_file = prost_types::compiler::code_generator_response::File::default();
        // Use relative path from output dir
        let relative_path = file
            .path
            .strip_prefix(&params.output_dir)
            .unwrap_or(&file.path);
        gen_file.name = Some(relative_path.to_string_lossy().to_string());
        gen_file.content = Some(file.content);
        response.file.push(gen_file);
    }

    // Clean up temp directory
    let _ = fs::remove_dir_all(&temp_dir);

    // Write response to stdout
    let mut output = Vec::new();
    response.encode(&mut output)?;
    io::stdout().write_all(&output)?;

    Ok(())
}

/// Reconstruct proto file content from FileDescriptorProto
fn reconstruct_proto_content(proto: &prost_types::FileDescriptorProto) -> String {
    let mut content = String::new();

    content.push_str("syntax = \"proto3\";\n\n");

    if let Some(package) = &proto.package {
        content.push_str(&format!("package {};\n\n", package));
    }

    // Add messages
    for message in &proto.message_type {
        content.push_str(&format!("message {} {{\n", message.name.as_ref().unwrap()));
        for field in &message.field {
            let field_name = field.name.as_ref().unwrap();
            let field_number = field.number.unwrap();
            let field_type = get_field_type_name(field);
            content.push_str(&format!(
                "  {} {} = {};\n",
                field_type, field_name, field_number
            ));
        }
        content.push_str("}\n\n");
    }

    // Add services
    for service in &proto.service {
        content.push_str(&format!("service {} {{\n", service.name.as_ref().unwrap()));
        for method in &service.method {
            let method_name = method.name.as_ref().unwrap();
            // 去掉包名前缀，只保留消息类型名
            let input_type = method
                .input_type
                .as_ref()
                .unwrap()
                .trim_start_matches('.')
                .rsplit('.')
                .next()
                .unwrap_or("Unknown");
            let output_type = method
                .output_type
                .as_ref()
                .unwrap()
                .trim_start_matches('.')
                .rsplit('.')
                .next()
                .unwrap_or("Unknown");
            content.push_str(&format!(
                "  rpc {}({}) returns ({});\n",
                method_name, input_type, output_type
            ));
        }
        content.push_str("}\n\n");
    }

    content
}

fn get_field_type_name(field: &prost_types::FieldDescriptorProto) -> String {
    use prost_types::field_descriptor_proto::Type;

    match field.r#type() {
        Type::Double => "double".to_string(),
        Type::Float => "float".to_string(),
        Type::Int64 => "int64".to_string(),
        Type::Uint64 => "uint64".to_string(),
        Type::Int32 => "int32".to_string(),
        Type::Fixed64 => "fixed64".to_string(),
        Type::Fixed32 => "fixed32".to_string(),
        Type::Bool => "bool".to_string(),
        Type::String => "string".to_string(),
        Type::Bytes => "bytes".to_string(),
        Type::Uint32 => "uint32".to_string(),
        Type::Sfixed32 => "sfixed32".to_string(),
        Type::Sfixed64 => "sfixed64".to_string(),
        Type::Sint32 => "sint32".to_string(),
        Type::Sint64 => "sint64".to_string(),
        Type::Message | Type::Enum => field
            .type_name
            .as_ref()
            .map(|s| s.trim_start_matches('.').to_string())
            .unwrap_or_else(|| "unknown".to_string()),
        _ => "unknown".to_string(),
    }
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
