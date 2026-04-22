//! Descriptor-based proto parsing.
//!
//! Runs `protoc` against the configured proto files with
//! `--descriptor_set_out`, decodes the emitted `FileDescriptorSet` via
//! `prost_types`, and converts the structured result into crate-local model
//! types. This replaces the previous regex-based `.proto` text parser: the
//! descriptor path is robust against comments, nested messages, streaming
//! markers, and proto2/proto3 syntax differences.

use crate::{ProtoField, ProtoMessage, ProtoMethod, ProtoService, error::CodegenError};
use prost::Message;
use prost_types::{
    DescriptorProto, FieldDescriptorProto, FileDescriptorProto, FileDescriptorSet,
    field_descriptor_proto,
};
use std::path::{Path, PathBuf};

/// Invoke `protoc` to compile the given proto files into a
/// `FileDescriptorSet`, then return the decoded structure.
///
/// `includes` is the list of `-I` search paths passed to `protoc`. If empty,
/// each proto file's parent directory is used as a fallback include path so
/// that relative imports still resolve.
pub fn compile_to_descriptor_set(
    proto_files: &[PathBuf],
    includes: &[PathBuf],
) -> crate::error::Result<FileDescriptorSet> {
    use std::process::Command;

    if proto_files.is_empty() {
        return Ok(FileDescriptorSet::default());
    }

    // `protoc` writes the binary descriptor set to a temporary file; we then
    // read and decode it. Using a process-unique name avoids clashes when the
    // codegen library is used concurrently in tests.
    let out_path = std::env::temp_dir().join(format!(
        "actr-web-protoc-codegen-{}-{}.desc",
        std::process::id(),
        // Use the files' hash to disambiguate parallel compile calls.
        fnv_hash(proto_files),
    ));

    let mut cmd = Command::new("protoc");
    cmd.arg("--include_imports")
        .arg("--include_source_info")
        .arg(format!("--descriptor_set_out={}", out_path.display()));

    let mut seen_includes: Vec<PathBuf> = Vec::new();
    for inc in includes {
        if !seen_includes.iter().any(|p| p == inc) {
            seen_includes.push(inc.clone());
        }
    }
    // Fallback: ensure each proto file is reachable via at least its parent,
    // so relative imports in the proto resolve even when the caller did not
    // pass an explicit include root.
    for proto in proto_files {
        if let Some(parent) = proto.parent().filter(|p| !p.as_os_str().is_empty()) {
            let parent = parent.to_path_buf();
            if !seen_includes.iter().any(|p| p == &parent) {
                seen_includes.push(parent);
            }
        }
    }

    for inc in &seen_includes {
        cmd.arg("-I").arg(inc);
    }

    for proto in proto_files {
        cmd.arg(proto);
    }

    tracing::debug!("Running: {:?}", cmd);

    let output = cmd
        .output()
        .map_err(|e| CodegenError::proto_parse(format!("failed to spawn protoc: {e}")))?;

    if !output.status.success() {
        // Best-effort cleanup; the file may not exist.
        let _ = std::fs::remove_file(&out_path);
        return Err(CodegenError::proto_parse(format!(
            "protoc failed (status {}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        )));
    }

    let bytes = std::fs::read(&out_path).map_err(CodegenError::IoError)?;
    let _ = std::fs::remove_file(&out_path);

    let set = FileDescriptorSet::decode(bytes.as_slice())
        .map_err(|e| CodegenError::proto_parse(format!("FileDescriptorSet decode failed: {e}")))?;

    Ok(set)
}

/// Convert a `FileDescriptorProto` into the crate's `ProtoService` model.
///
/// Returns `None` when the file declares no service — callers treat this as
/// "nothing to generate from this file".
pub fn file_to_proto_service(file: &FileDescriptorProto) -> Option<ProtoService> {
    // The crate currently assumes one service per proto file, matching the
    // sibling `tools/protoc-gen/rust/` plugin. If a file declares more than
    // one service only the first is used; remaining services are logged.
    let service = file.service.first()?;
    if file.service.len() > 1 {
        tracing::warn!(
            "{}: declares {} services, only the first ({}) is emitted",
            file.name(),
            file.service.len(),
            service.name()
        );
    }

    let package = if file.package().is_empty() {
        "default".to_string()
    } else {
        file.package().to_string()
    };

    let methods = service
        .method
        .iter()
        .map(method_to_proto_method)
        .collect::<Vec<_>>();

    let messages = file
        .message_type
        .iter()
        .map(message_to_proto_message)
        .collect::<Vec<_>>();

    Some(ProtoService {
        name: service.name().to_string(),
        package,
        methods,
        messages,
    })
}

/// Locate a top-level message by name and return its field list in the form
/// `(field_name, proto_wire_type_token)` expected by the WASM scaffold
/// generator.
pub fn message_fields_for_scaffold(
    file: &FileDescriptorProto,
    message_name: &str,
) -> Option<Vec<(String, String)>> {
    let desc = file
        .message_type
        .iter()
        .find(|m| m.name() == message_name)?;
    Some(
        desc.field
            .iter()
            .map(|f| (f.name().to_string(), scalar_type_token(f)))
            .collect(),
    )
}

/// Convert a `MethodDescriptorProto` into the crate's `ProtoMethod`.
fn method_to_proto_method(method: &prost_types::MethodDescriptorProto) -> ProtoMethod {
    ProtoMethod {
        name: method.name().to_string(),
        input_type: short_type_name(method.input_type()),
        output_type: short_type_name(method.output_type()),
        is_streaming: method.client_streaming() || method.server_streaming(),
    }
}

/// Convert a top-level `DescriptorProto` into a `ProtoMessage`, flattening
/// fields. Nested message types are ignored — they are not supported by the
/// rest of the generator and preserving them here would silently change
/// behaviour from the previous regex implementation, which also ignored
/// nested message bodies.
fn message_to_proto_message(message: &DescriptorProto) -> ProtoMessage {
    let fields = message.field.iter().map(field_to_proto_field).collect();
    ProtoMessage {
        name: message.name().to_string(),
        fields,
    }
}

/// Convert a `FieldDescriptorProto` into the crate's `ProtoField`.
///
/// `is_optional` is only set when the source declared the field with an
/// explicit `optional` keyword (proto2 or proto3 `optional`). Plain proto3
/// scalars carry `Label::Optional` on the descriptor but are not surfaced as
/// optional, matching the behaviour of the previous regex text parser.
fn field_to_proto_field(field: &FieldDescriptorProto) -> ProtoField {
    use field_descriptor_proto::Label;

    let is_repeated = field.label() == Label::Repeated;
    let is_optional = !is_repeated && field.proto3_optional();

    ProtoField {
        name: field.name().to_string(),
        field_type: scalar_type_token(field),
        number: field.number() as u32,
        is_repeated,
        is_optional,
    }
}

/// Render the type name for a field as the token the rest of the codegen
/// expects (e.g. "string", "int32", or a message type's short name).
fn scalar_type_token(field: &FieldDescriptorProto) -> String {
    use field_descriptor_proto::Type;

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
        Type::Message | Type::Enum | Type::Group => short_type_name(field.type_name()),
    }
}

/// Strip the package prefix and leading dot from a fully-qualified proto type
/// name, yielding the short symbol that downstream TypeScript / Rust emitters
/// consume.
pub fn short_type_name(raw: &str) -> String {
    raw.trim_start_matches('.')
        .rsplit('.')
        .next()
        .unwrap_or(raw)
        .to_string()
}

/// Find the `FileDescriptorProto` matching a caller-supplied proto path.
///
/// `protoc` records files by the name it was invoked with (typically the
/// path relative to an `-I` include root). We therefore try a few
/// normalisations: exact match, basename match, and `ends_with` match.
pub fn find_file<'a>(
    set: &'a FileDescriptorSet,
    proto_path: &Path,
) -> Option<&'a FileDescriptorProto> {
    let file_name = proto_path.file_name().and_then(|s| s.to_str());
    set.file.iter().find(|f| {
        let n = f.name();
        n == proto_path.to_string_lossy()
            || file_name.is_some_and(|b| n == b)
            || file_name.is_some_and(|b| n.ends_with(b))
    })
}

/// Minimal FNV-1a 32-bit hash of the proto paths. Used solely to build a
/// per-invocation unique temp filename; not security-sensitive.
fn fnv_hash(paths: &[PathBuf]) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for p in paths {
        for b in p.as_os_str().to_string_lossy().as_bytes() {
            h ^= *b as u32;
            h = h.wrapping_mul(0x0100_0193);
        }
        h ^= 0x2f; // path separator in hash
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}
