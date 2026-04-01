use std::ffi::OsString;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let proto_dir = "../proto";
    let proto_file = "../proto/echo.proto";

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

    // Post-process: convert inner doc comments (//!) to outer (///) and
    // inner attributes (#![...]) to outer (#[...]) in generated code.
    // This is needed because the generated file is included via include!()
    // inside a mod block, where inner doc comments and inner attributes
    // are not valid in Rust 2024 edition.
    let actor_file = out_dir.join("echo_actor.rs");
    if actor_file.exists() {
        let content = fs::read_to_string(&actor_file).expect("read echo_actor.rs");
        let fixed = content
            .lines()
            .map(|line| {
                if line.starts_with("//!") {
                    // //! doc comment → // regular comment
                    format!("//{}", &line[3..])
                } else if line.starts_with("#![") {
                    // #![attr] → #[attr]
                    format!("#[{}", &line[3..])
                } else {
                    line.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(&actor_file, fixed).expect("write echo_actor.rs");
    }
}

fn ensure_plugin(name: &str) -> Option<PathBuf> {
    // First: check if plugin is already on PATH / cargo bin
    if let Some(path) = find_plugin(name) {
        return Some(path);
    }

    // Second: try to build from local actr workspace
    if let Some(path) = bootstrap_local_workspace_plugin(name) {
        return Some(path);
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

    // From server-guest/ → ../../../../.. = actr workspace root
    let actr_root = manifest_dir.join("../../../../..");
    let candidate_roots = [
        actr_root.clone(),
        manifest_dir.join("../actr"),
        manifest_dir.parent()?.to_path_buf(),
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
