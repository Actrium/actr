use anyhow::{Context, Result, anyhow};
use bytes::Bytes;
use heck::ToSnakeCase;
use prost::Message;
use prost_types::{
    DescriptorProto, FileDescriptorProto, ServiceDescriptorProto,
    compiler::{CodeGeneratorRequest, CodeGeneratorResponse, code_generator_response::File},
};
use std::collections::HashMap;
use std::io::{self, Read, Write};

use actr_framework_protoc_codegen::{GeneratorRole, ModernGenerator};
use actr_protocol::{PackageName, ServiceName};

/// Proto 源类型枚举 - 简化设计，支持编译时路由
#[derive(Debug, Clone, PartialEq)]
pub enum ProtoSource {
    /// 本地服务（来自 proto/ 目录）
    Local,
    /// 远程服务（来自 Actr.toml [dependencies]）
    Remote,
}

impl ProtoSource {
    /// 从 proto 文件推断源类型
    /// 注意：在编译时路由架构中，这个推断主要用于向后兼容
    /// 真正的路由决策应该基于项目配置和路由表
    pub fn from_proto_file(file: &FileDescriptorProto) -> Self {
        let has_services = !file.service.is_empty();

        if has_services {
            // 默认假设为本地服务
            // 在实际编译时路由中，这会由项目扫描逻辑确定
            Self::Local
        } else {
            // 纯消息类型文件，通常是远程依赖
            Self::Remote
        }
    }
}

fn main() -> Result<()> {
    // 支持 --version 和 --help 参数
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 {
        match args[1].as_str() {
            "--version" | "-V" => {
                println!("protoc-gen-actrframework {}", env!("CARGO_PKG_VERSION"));
                println!(
                    "actr-framework-protoc-codegen library version: {}",
                    env!("CARGO_PKG_VERSION")
                );
                return Ok(());
            }
            "--help" | "-h" => {
                println!("protoc-gen-actrframework - Protobuf plugin for Actor-RTC framework");
                println!();
                println!("USAGE:");
                println!(
                    "    As protoc plugin: protoc --plugin=protoc-gen-actrframework=PATH --actrframework_out=OUT_DIR input.proto"
                );
                println!("    Version info:     protoc-gen-actrframework --version");
                println!();
                println!("VERSION:");
                println!("    {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            _ => {}
        }
    }

    // 从标准输入读取 CodeGeneratorRequest
    let mut stdin = io::stdin();
    let mut buf = Vec::new();
    stdin
        .read_to_end(&mut buf)
        .context("Failed to read from stdin")?;

    let request = CodeGeneratorRequest::decode(Bytes::from(buf))
        .context("Failed to decode CodeGeneratorRequest")?;

    // 生成代码
    let response = generate_code(request)?;

    // 将 CodeGeneratorResponse 写入标准输出
    let mut out_buf = Vec::new();
    response
        .encode(&mut out_buf)
        .context("Failed to encode CodeGeneratorResponse")?;

    io::stdout()
        .write_all(&out_buf)
        .context("Failed to write to stdout")?;

    Ok(())
}

/// Parse parameters from protoc --actrframework_opt
/// Format: key1=value1,key2=value2
fn parse_parameters(param_str: &str) -> HashMap<String, String> {
    let mut params = HashMap::new();
    for pair in param_str.split(',') {
        if let Some((key, value)) = pair.split_once('=') {
            params.insert(key.trim().to_string(), value.trim().to_string());
        }
    }
    params
}

fn generate_code(request: CodeGeneratorRequest) -> Result<CodeGeneratorResponse> {
    // Set supported features if available (FEATURE_PROTO3_OPTIONAL = 1)
    let mut response = CodeGeneratorResponse {
        supported_features: Some(1u64),
        ..Default::default()
    };

    // Parse parameters from --actrframework_opt
    let params = parse_parameters(request.parameter.as_deref().unwrap_or(""));

    // 构建类型映射用于解析
    let mut message_types = HashMap::new();
    for file in &request.proto_file {
        collect_message_types(file, &mut message_types, file.package());
    }

    // 为每个要生成的文件处理 services
    for file_name in &request.file_to_generate {
        if let Some(file) = request.proto_file.iter().find(|f| f.name() == file_name) {
            for service in &file.service {
                let generated_file = generate_service_code(file, service, &message_types, &params)?;
                response.file.push(generated_file);
            }
        }
    }

    Ok(response)
}

fn collect_message_types(
    file: &FileDescriptorProto,
    types: &mut HashMap<String, DescriptorProto>,
    package_prefix: &str,
) {
    for message in &file.message_type {
        let full_name = if package_prefix.is_empty() {
            message.name().to_string()
        } else {
            format!("{}.{}", package_prefix, message.name())
        };
        types.insert(full_name.clone(), message.clone());

        // 递归处理嵌套消息
        for nested in &message.nested_type {
            let nested_name = format!("{}.{}", full_name, nested.name());
            types.insert(nested_name, nested.clone());
        }
    }
}

fn generate_service_code(
    file: &FileDescriptorProto,
    service: &ServiceDescriptorProto,
    _message_types: &HashMap<String, DescriptorProto>,
    params: &HashMap<String, String>,
) -> Result<File> {
    let service_name = service.name();
    let package_name = file.package();

    // Validate proto package name early to surface clear errors
    PackageName::new(package_name.to_string())
        .map_err(|e| anyhow!("Invalid proto package name '{}': {}", package_name, e))?;
    // Validate proto service name early
    ServiceName::new(service_name.to_string())
        .map_err(|e| anyhow!("Invalid proto service name '{}': {}", service_name, e))?;

    // Determine proto source based on proto file characteristics
    let proto_source = ProtoSource::from_proto_file(file);

    // 🚀 使用现代化代码生成器
    let role = match proto_source {
        ProtoSource::Local => GeneratorRole::ServerSide,
        ProtoSource::Remote => GeneratorRole::ClientSide,
    };

    // Get manufacturer from parameters, default to "acme" for backward compatibility
    let manufacturer = params
        .get("manufacturer")
        .cloned()
        .unwrap_or_else(|| "acme".to_string());

    let mut generator = ModernGenerator::new(package_name, service_name, role);
    generator.set_manufacturer(manufacturer);
    let final_code = generator.generate(&service.method)?;

    // 根据角色生成不同的文件后缀
    let file_suffix = match role {
        GeneratorRole::ServerSide => "_actor",
        GeneratorRole::ClientSide => "_client",
    };

    Ok(File {
        name: Some(format!(
            "{}{}.rs",
            service_name.to_snake_case(),
            file_suffix
        )),
        content: Some(final_code),
        insertion_point: None,
        generated_code_info: None,
    })
}
