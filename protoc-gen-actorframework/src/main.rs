use anyhow::{Context, Result};
use bytes::Bytes;
use heck::ToSnakeCase;
use prost::Message;
use prost_types::{
    compiler::{code_generator_response::File, CodeGeneratorRequest, CodeGeneratorResponse},
    DescriptorProto, FileDescriptorProto, ServiceDescriptorProto,
};
use std::collections::{HashMap, HashSet};
use std::io::{self, Read, Write};

mod generator;

use generator::{ActorAdapterGenerator, ActorTraitGenerator, FileRole};

fn main() -> Result<()> {
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

fn generate_code(request: CodeGeneratorRequest) -> Result<CodeGeneratorResponse> {
    let mut response = CodeGeneratorResponse::default();
    // Set supported features if available
    response.supported_features = Some(1u64); // FEATURE_PROTO3_OPTIONAL = 1

    // 构建类型映射用于解析
    let mut message_types = HashMap::new();
    for file in &request.proto_file {
        collect_message_types(file, &mut message_types, &file.package().to_string());
    }

    // 为每个要生成的文件处理 services
    for file_name in &request.file_to_generate {
        if let Some(file) = request.proto_file.iter().find(|f| f.name() == file_name) {
            for service in &file.service {
                let generated_file = generate_service_code(file, service, &message_types)?;
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
) -> Result<File> {
    let service_name = service.name();
    let package_name = file.package();

    // Determine file role based on proto file characteristics
    let file_role = FileRole::from_proto_file(file);

    // 生成 Actor Trait/Client based on file role
    let trait_generator = ActorTraitGenerator::new(package_name, service_name, file_role.clone());
    let trait_code = trait_generator.generate(&service.method)?;

    // 生成 ActorAdapter/ClientManager based on file role
    let adapter_generator =
        ActorAdapterGenerator::new(package_name, service_name, file_role.clone());
    let adapter_code = adapter_generator.generate(&service.method)?;

    // 组合代码并去重imports
    let combined_code = if trait_code.trim().is_empty() {
        adapter_code
    } else if adapter_code.trim().is_empty() {
        trait_code
    } else {
        // 去重imports并组合代码
        remove_duplicate_imports(&format!("{}\n\n{}", trait_code, adapter_code))
    };

    // Generate appropriate file name based on role
    let file_suffix = match file_role {
        FileRole::LocalService => "_service",
        FileRole::RemoteClient => "_client",
        FileRole::Mixed => "_actor",
        FileRole::MessageTypes => "_types",
    };

    Ok(File {
        name: Some(format!(
            "{}{}.rs",
            service_name.to_snake_case(),
            file_suffix
        )),
        content: Some(combined_code),
        insertion_point: None,
        generated_code_info: None,
    })
}

/// 去除重复的use语句
fn remove_duplicate_imports(code: &str) -> String {
    let lines: Vec<&str> = code.lines().collect();
    let mut seen_imports = HashSet::new();
    let mut result_lines = Vec::new();

    for line in lines {
        let trimmed = line.trim();
        if trimmed.starts_with("use ") && trimmed.ends_with(";") {
            if seen_imports.insert(trimmed.to_string()) {
                result_lines.push(line.to_string());
            }
        } else {
            result_lines.push(line.to_string());
        }
    }

    result_lines.join("\n")
}
