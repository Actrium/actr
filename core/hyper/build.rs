//! Build script for `actr-hyper`.
//!
//! When the `wasm-engine` feature is enabled, compiles the
//! `wasm_actor_fixture` guest crate (`tests/wasm_actor_fixture/`) to a
//! wasm32-wasip2 Component Model component and exposes its bytes to the
//! integration tests via the `ACTR_WASM_FIXTURE` env var plus the
//! `actr_wasm_fixture_available` cfg. Tests then `include_bytes!` the built
//! artifact instead of carrying a 15k-line hex blob in source.
//!
//! Requires the `wasm32-wasip2` rustup target and `wasm-component-ld`
//! (>= 0.5.22) on `PATH` or in `~/.cargo/bin`. CI installs both; a local
//! developer without them gets a `cargo:warning` and the fixture tests are
//! compiled out via the `actr_wasm_fixture_available` cfg gate — no
//! committed binary blob is needed.

use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    // Declare the custom cfg unconditionally so the test files gating on
    // `actr_wasm_fixture_available` don't trip `unexpected_cfgs` — even when
    // the wasm toolchain is absent and the cfg is never actually set.
    println!("cargo:rustc-check-cfg=cfg(actr_wasm_fixture_available)");

    // Only build the fixture when the wasm-engine feature is on; otherwise
    // the consuming tests are `#[cfg(feature = "wasm-engine")]`-gated out
    // anyway and there is nothing to do.
    if env::var_os("CARGO_FEATURE_WASM_ENGINE").is_none() {
        return;
    }

    let manifest_dir =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR set by cargo"));
    let guest_dir = manifest_dir.join("tests/wasm_actor_fixture");
    let wit = manifest_dir.join("../framework/wit/actr-workload.wit");

    // Rebuild when the guest source, its manifest, or the WIT contract moves.
    println!(
        "cargo:rerun-if-changed={}",
        guest_dir.join("src/lib.rs").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        guest_dir.join("Cargo.toml").display()
    );
    println!("cargo:rerun-if-changed={}", wit.display());

    let ld = match find_wasm_component_ld() {
        Some(path) => path,
        None => {
            emit_toolchain_warning(
                "wasm-component-ld was not found on PATH or in ~/.cargo/bin",
                "cargo install wasm-component-ld --version 0.5.22",
            );
            return;
        }
    };

    if !target_installed("wasm32-wasip2") {
        emit_toolchain_warning(
            "the wasm32-wasip2 rustup target is not installed",
            "rustup target add wasm32-wasip2",
        );
        return;
    }

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR set by cargo"));
    // Isolated target-dir so the nested `cargo build` never contends for the
    // host workspace's target-dir locks (the guest is its own workspace).
    let guest_target_dir = out_dir.join("wasm-guest-target");

    let rustflags = format!("-Clinker={}", ld.display());
    let status = Command::new("cargo")
        .args(["build", "--release", "--target", "wasm32-wasip2"])
        .current_dir(&guest_dir)
        .env("RUSTFLAGS", &rustflags)
        .env("CARGO_TARGET_DIR", &guest_target_dir)
        .status()
        .expect("failed to spawn `cargo build` for wasm_actor_fixture");
    if !status.success() {
        panic!("`cargo build` of wasm_actor_fixture (wasm32-wasip2) failed");
    }

    let wasm = guest_target_dir.join("wasm32-wasip2/release/wasm_actor_fixture.wasm");
    if !wasm.exists() {
        panic!("expected guest artifact not found: {}", wasm.display());
    }

    let dest = out_dir.join("wasm_actor_fixture.wasm");
    std::fs::copy(&wasm, &dest).expect("failed to copy built wasm fixture into OUT_DIR");

    println!("cargo:rustc-env=ACTR_WASM_FIXTURE={}", dest.display());
    println!("cargo:rustc-cfg=actr_wasm_fixture_available");
}

/// Emit a `cargo:warning` explaining the wasm toolchain gap and that the
/// fixture tests will be compiled out, so a missing local toolchain never
/// hard-fails the build.
fn emit_toolchain_warning(reason: &str, install_hint: &str) {
    println!("cargo:warning=wasm-engine feature is on but {reason};");
    println!("cargo:warning=  install with: `{install_hint}`");
    println!("cargo:warning=  wasm_actor_fixture integration tests will be compiled out.");
}

/// Locate `wasm-component-ld`, honouring an explicit `WASM_COMPONENT_LD`
/// override (mirrors the old `build.sh`), then `PATH`, then `~/.cargo/bin`.
fn find_wasm_component_ld() -> Option<PathBuf> {
    if let Some(path) = env::var_os("WASM_COMPONENT_LD") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Some(path);
        }
    }
    if let Some(path) = find_in_path("wasm-component-ld") {
        return Some(path);
    }
    let home = env::var_os("HOME")?;
    let candidate = PathBuf::from(home).join(".cargo/bin/wasm-component-ld");
    if candidate.is_file() {
        Some(candidate)
    } else {
        None
    }
}

fn find_in_path(cmd: &str) -> Option<PathBuf> {
    let path = env::var_os("PATH")?;
    env::split_paths(&path)
        .map(|dir| dir.join(cmd))
        .find(|p| p.is_file())
}

fn target_installed(target: &str) -> bool {
    let output = match Command::new("rustup")
        .args(["target", "list", "--installed"])
        .output()
    {
        Ok(output) => output,
        Err(_) => return false,
    };
    if !output.status.success() {
        return false;
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .any(|line| line.trim() == target)
}
