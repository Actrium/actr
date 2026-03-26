use std::path::PathBuf;
use std::sync::OnceLock;

static FIXTURE_BYTES: OnceLock<Vec<u8>> = OnceLock::new();

/// Build the wasm actor fixture (if needed) and return its bytes.
///
/// Calls `build.sh` which runs `cargo build --target wasm32-unknown-unknown`
/// and `wasm-opt --asyncify`, then reads the resulting `.wasm` file.
/// The result is cached for the lifetime of the test process.
pub fn fixture_bytes() -> &'static [u8] {
    FIXTURE_BYTES.get_or_init(|| {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let script = manifest_dir.join("tests/wasm_actor_fixture/build.sh");
        let wasm_path = manifest_dir.join("tests/wasm_actor_fixture/built/wasm_actor_fixture.wasm");

        let output = std::process::Command::new("bash")
            .arg(&script)
            .output()
            .expect("failed to run wasm_actor_fixture/build.sh");
        if !output.status.success() {
            panic!(
                "wasm_actor_fixture/build.sh failed:\nstdout: {}\nstderr: {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr),
            );
        }

        std::fs::read(&wasm_path).expect("wasm_actor_fixture.wasm not found after build")
    })
}
