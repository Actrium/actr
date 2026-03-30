use std::ffi::OsString;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let proto_dir = "proto";
    let proto_file = "proto/echo.proto";

    println!("cargo:rerun-if-changed={proto_file}");

    // Step 1: prost — generate protobuf message types (echo.rs)
    prost_build::Config::new()
        .compile_protos(&[proto_file], &[proto_dir])
        .expect("prost_build failed");

    // Step 2: protoc-gen-actrframework — generate actor framework code (echo_actor.rs)
    let plugin = ensure_plugin("protoc-gen-actrframework")
        .expect("protoc-gen-actrframework not found in PATH and automatic bootstrap failed");

    let status = Command::new("protoc")
        .arg(format!("--proto_path={proto_dir}"))
        .arg(format!(
            "--plugin=protoc-gen-actrframework={}",
            plugin.display()
        ))
        .arg("--actrframework_opt=manufacturer=actrium,LocalFiles=echo.proto")
        .arg(format!("--actrframework_out={}", out_dir.display()))
        .arg(proto_file)
        .status()
        .expect("failed to run protoc");

    assert!(status.success(), "protoc-gen-actrframework failed");
}

fn ensure_plugin(name: &str) -> Option<PathBuf> {
    if let Some(path) = find_plugin(name) {
        return Some(path);
    }

    if let Some(path) = bootstrap_local_workspace_plugin(name) {
        return Some(path);
    }

    if install_plugin_from_git(name).is_ok() {
        return find_plugin(name);
    }

    None
}

fn find_plugin(name: &str) -> Option<PathBuf> {
    let candidates = plugin_candidate_names(name);

    if let Some(path_var) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path_var) {
            for candidate in &candidates {
                let path = dir.join(candidate);
                if path.is_file() {
                    return Some(path);
                }
            }
        }
    }

    if let Some(home) = std::env::var_os("HOME") {
        let cargo_bin = PathBuf::from(home).join(".cargo/bin");
        for candidate in &candidates {
            let path = cargo_bin.join(candidate);
            if path.is_file() {
                return Some(path);
            }
        }
    }

    None
}

fn bootstrap_local_workspace_plugin(name: &str) -> Option<PathBuf> {
    let manifest_dir = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR")?);
    let candidate_roots = [
        manifest_dir.join("../../.."), // Inside actr repo: examples/rust/echo-actr -> actr
        manifest_dir.join("../actr"),  // Standalone: next to actr repo
        manifest_dir.parent()?.to_path_buf(), // Other workspace configurations
    ];

    for root in candidate_roots {
        if !root.join("tools/protoc-gen/rust/Cargo.toml").is_file() {
            continue;
        }

        let status = Command::new("cargo")
            .arg("build")
            .arg("--manifest-path")
            .arg(root.join("tools/protoc-gen/rust/Cargo.toml"))
            .arg("--bin")
            .arg(name)
            .status()
            .ok()?;

        if !status.success() {
            continue;
        }

        let plugin_path = root.join("target/debug").join(plugin_file_name(name));
        if plugin_path.is_file() {
            return Some(plugin_path);
        }
    }

    None
}

fn install_plugin_from_git(name: &str) -> Result<(), String> {
    let rev = resolve_actr_rev()?;
    let status = Command::new("cargo")
        .arg("install")
        .arg("--git")
        .arg("https://github.com/Actrium/actr.git")
        .arg("--rev")
        .arg(rev)
        .arg("--bin")
        .arg(name)
        .arg("actr-framework-protoc-codegen")
        .status()
        .map_err(|error| format!("failed to run cargo install: {error}"))?;

    if status.success() {
        Ok(())
    } else {
        Err(format!("cargo install exited with status {status}"))
    }
}

fn resolve_actr_rev() -> Result<String, String> {
    let manifest_dir = PathBuf::from(
        std::env::var_os("CARGO_MANIFEST_DIR")
            .ok_or_else(|| "CARGO_MANIFEST_DIR is not set".to_string())?,
    );
    let cargo_toml =
        fs::read_to_string(manifest_dir.join("Cargo.toml")).map_err(|error| error.to_string())?;

    for line in cargo_toml.lines() {
        if !line.contains("actr-framework") || !line.contains("rev = ") {
            continue;
        }
        if let Some(rev) = extract_quoted_value(line, "rev = ") {
            return Ok(rev);
        }
    }

    Err("failed to locate actr-framework rev in Cargo.toml".to_string())
}

fn extract_quoted_value(line: &str, key: &str) -> Option<String> {
    let start = line.find(key)? + key.len();
    let rest = &line[start..];
    let first_quote = rest.find('"')? + 1;
    let quoted = &rest[first_quote..];
    let end_quote = quoted.find('"')?;
    Some(quoted[..end_quote].to_string())
}

fn plugin_candidate_names(name: &str) -> Vec<OsString> {
    #[cfg(windows)]
    {
        let mut candidates = vec![OsString::from(name)];
        candidates.push(OsString::from(format!("{name}.exe")));
        return candidates;
    }
    #[cfg(not(windows))]
    {
        vec![OsString::from(name)]
    }
}

fn plugin_file_name(name: &str) -> OsString {
    #[cfg(windows)]
    {
        OsString::from(format!("{name}.exe"))
    }
    #[cfg(not(windows))]
    {
        OsString::from(name)
    }
}
