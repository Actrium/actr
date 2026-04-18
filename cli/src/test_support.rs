use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use rusqlite::Connection;
use serde_json::Value;
use tempfile::TempDir;

pub const DEFAULT_ACTRIX_REPO: &str = "https://github.com/Actrium/actrix.git";
pub const DEFAULT_ACTRIX_ARTIFACT_REPO: &str = "Actrium/actrix";
pub const DEFAULT_ACTRIX_ARTIFACT_WORKFLOW: &str = "243491296";
pub const DEFAULT_ACTRIX_ARTIFACT_BRANCH: &str = "main";
pub const LOCAL_E2E_REALM_ID: u32 = 1001;
const KS_GRPC_PORT: u16 = 50052;

pub fn actr_bin() -> PathBuf {
    if let Some(path) = std::env::var_os("CARGO_BIN_EXE_actr") {
        return PathBuf::from(path);
    }

    let candidate = workspace_root()
        .join("target/debug")
        .join(format!("actr{}", std::env::consts::EXE_SUFFIX));
    if candidate.is_file() {
        return candidate;
    }

    panic!(
        "failed to locate actr binary: CARGO_BIN_EXE_actr is unset and fallback missing at {}",
        candidate.display()
    );
}

pub fn rust_e2e_target_dir() -> PathBuf {
    workspace_root().join("target/e2e-cache/rust-target")
}

fn local_swift_package_dir() -> PathBuf {
    workspace_root().join("bindings/swift/.e2e/package")
}

pub fn run_actr(args: &[&str], cwd: &Path) -> Output {
    let mut cmd = Command::new(actr_bin());
    cmd.args(args).current_dir(cwd);
    cmd.env("CARGO_TARGET_DIR", rust_e2e_target_dir());

    // Make Swift template/e2e use workspace-local bindings first to avoid stale remote package behavior.
    let local_swift = {
        let e2e_package = local_swift_package_dir();
        if e2e_package.join("Package.swift").is_file() {
            e2e_package
        } else {
            workspace_root().join("bindings/swift")
        }
    };
    if local_swift.join("Package.swift").is_file() {
        cmd.env("ACTR_SWIFT_LOCAL_PATH", local_swift);
    }

    cmd.output().expect("failed to run actr binary")
}

pub fn assert_success(out: &Output, context: &str) {
    assert!(
        out.status.success(),
        "{context} failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

pub fn ensure_success(out: &Output, context: &str) -> Result<()> {
    if out.status.success() {
        return Ok(());
    }
    bail!(
        "{context} failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

pub fn cargo_build(dir: &Path) {
    let out = Command::new("cargo")
        .args(["build"])
        .current_dir(dir)
        .env("CARGO_TARGET_DIR", rust_e2e_target_dir())
        .output()
        .expect("cargo build failed");
    assert_success(&out, &format!("cargo build in {}", dir.display()));
}

pub fn random_manufacturer() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .subsec_nanos();
    format!("test{nanos:08x}")
}

pub fn align_project_with_local_actrix(project_dir: &Path) -> Result<()> {
    rewrite_project_realm_id(project_dir, LOCAL_E2E_REALM_ID)
}

pub fn pin_echo_service_dependency_version(project_dir: &Path, manufacturer: &str) -> Result<()> {
    let actr_toml_path = project_dir.join("manifest.toml");
    let content = fs::read_to_string(&actr_toml_path)
        .with_context(|| format!("failed to read {}", actr_toml_path.display()))?;
    let target = "echo-service = {}";
    let replacement =
        format!("echo-service = {{ actr_type = \"{manufacturer}:EchoService:1.0.0\" }}");

    if !content.contains(target) {
        bail!(
            "failed to pin echo dependency version in {}: '{target}' not found",
            actr_toml_path.display()
        );
    }

    let rewritten = content.replacen(target, &replacement, 1);
    fs::write(&actr_toml_path, rewritten)
        .with_context(|| format!("failed to write {}", actr_toml_path.display()))?;
    Ok(())
}

pub fn align_rust_project_with_workspace(project_dir: &Path) -> Result<()> {
    let cargo_toml_path = project_dir.join("Cargo.toml");
    let mut content = fs::read_to_string(&cargo_toml_path)
        .with_context(|| format!("failed to read {}", cargo_toml_path.display()))?;

    if content.contains("[patch.crates-io]") && content.contains("actr = { path =") {
        return Ok(());
    }

    let workspace = workspace_root();
    let mut patch = String::from("\n[patch.crates-io]\n");
    let crates = [
        ("actr", workspace.clone()),
        ("actr-protocol", workspace.join("core/protocol")),
        ("actr-framework", workspace.join("core/framework")),
        ("actr-hyper", workspace.join("core/hyper")),
        ("actr-runtime", workspace.join("core/runtime")),
        ("actr-config", workspace.join("core/config")),
        ("actr-service-compat", workspace.join("core/service-compat")),
        (
            "actr-runtime-mailbox",
            workspace.join("core/runtime-mailbox"),
        ),
    ];

    for (name, path) in crates {
        patch.push_str(&format!(
            "{name} = {{ path = \"{}\" }}\n",
            normalize_path_for_toml(&path)
        ));
    }

    content.push_str(&patch);
    fs::write(&cargo_toml_path, content)
        .with_context(|| format!("failed to write {}", cargo_toml_path.display()))?;
    Ok(())
}

pub fn ensure_local_swift_xcframework() -> Result<PathBuf> {
    static SWIFT_XCFRAMEWORK: OnceLock<PathBuf> = OnceLock::new();
    if let Some(path) = SWIFT_XCFRAMEWORK.get() {
        return Ok(path.clone());
    }

    let workspace = workspace_root();
    let package_dir = local_swift_package_dir();
    let output_path = package_dir.join("ActrFFI.xcframework");

    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }

    let cache_root = workspace.join("target/e2e-cache");
    fs::create_dir_all(&cache_root).context("failed to create e2e cache root")?;
    let _lock = DirLock::acquire(
        &cache_root.join("swift-xcframework-build.lock"),
        Duration::from_secs(900),
    )?;

    if package_dir.exists() {
        fs::remove_dir_all(&package_dir)
            .with_context(|| format!("failed to remove {}", package_dir.display()))?;
    }
    if output_path.exists() {
        fs::remove_dir_all(&output_path)
            .with_context(|| format!("failed to remove {}", output_path.display()))?;
    }

    fs::create_dir_all(&package_dir)
        .with_context(|| format!("failed to create {}", package_dir.display()))?;
    fs::copy(
        workspace.join("bindings/swift/Package.swift"),
        package_dir.join("Package.swift"),
    )
    .context("failed to copy swift Package.swift")?;
    copy_dir_all(
        &workspace.join("bindings/swift/Sources"),
        &package_dir.join("Sources"),
    )?;

    let bindings_dir = package_dir.join("ActrBindings");
    let headers_dir = bindings_dir.join("include");
    fs::create_dir_all(&headers_dir)
        .with_context(|| format!("failed to create {}", headers_dir.display()))?;

    let c_shim = bindings_dir.join("actrFFI.c");
    if !c_shim.exists() {
        fs::write(&c_shim, "").with_context(|| format!("failed to create {}", c_shim.display()))?;
    }

    for path in [
        bindings_dir.join("Actr.swift"),
        bindings_dir.join("actrFFI.h"),
        bindings_dir.join("ActrFFI.h"),
        bindings_dir.join("actrFFI.modulemap"),
        bindings_dir.join("ActrFFI.modulemap"),
        headers_dir.join("actrFFI.h"),
    ] {
        if path.exists() {
            fs::remove_file(&path)
                .with_context(|| format!("failed to remove {}", path.display()))?;
        }
    }

    run_checked(
        {
            let mut cmd = Command::new("cargo");
            cmd.args([
                "build",
                "-p",
                "libactr",
                "--release",
                "--target",
                "aarch64-apple-darwin",
                "--features",
                "macos-oslog",
            ])
            .current_dir(&workspace);
            cmd
        },
        "cargo build local swift ffi",
    )?;

    let dylib_path = workspace.join("target/aarch64-apple-darwin/release/libactr.dylib");
    let static_lib = workspace.join("target/aarch64-apple-darwin/release/libactr.a");
    if !dylib_path.exists() {
        bail!(
            "local swift ffi dylib not found at {}",
            dylib_path.display()
        );
    }
    if !static_lib.exists() {
        bail!(
            "local swift ffi static library not found at {}",
            static_lib.display()
        );
    }

    run_checked(
        {
            let mut cmd = Command::new("uniffi-bindgen");
            cmd.args([
                "generate",
                "--library",
                dylib_path.to_string_lossy().as_ref(),
                "--language",
                "swift",
                "--out-dir",
                bindings_dir.to_string_lossy().as_ref(),
            ])
            .current_dir(workspace.join("bindings/ffi"));
            cmd
        },
        "generate local swift bindings",
    )?;

    let actr_swift = bindings_dir.join("Actr.swift");
    let modulemap_path = if bindings_dir.join("actrFFI.modulemap").exists() {
        bindings_dir.join("actrFFI.modulemap")
    } else {
        bindings_dir.join("ActrFFI.modulemap")
    };

    run_checked(
        {
            let mut cmd = Command::new("perl");
            cmd.args([
                "-0777pi",
                "-e",
                "s|(#if canImport\\(ActrFFI\\)\\nimport ActrFFI\\n#endif)|#if canImport(actrFFI)\\n    import actrFFI\\n#endif\\n$1|",
                actr_swift.to_string_lossy().as_ref(),
            ]);
            cmd
        },
        "patch swift bindings import",
    )?;

    run_checked(
        {
            let mut cmd = Command::new("perl");
            cmd.args([
                "-0777pi",
                "-e",
                "s|(private func uniffiTraitInterfaceCallAsync<T>\\()|private struct UniffiUnsafeSendable<T>: \\@unchecked Sendable {\\n    let value: T\\n\\n    init(_ value: T) {\\n        self.value = value\\n    }\\n}\\n\\n$1|",
                actr_swift.to_string_lossy().as_ref(),
            ]);
            cmd
        },
        "inject swift sendable helper",
    )?;

    run_checked(
        {
            let mut cmd = Command::new("perl");
            cmd.args([
                "-0777pi",
                "-e",
                "s|private func uniffiTraitInterfaceCallAsync<T>\\(\\n\\s*makeCall: \\@escaping \\(\\) async throws -> T,\\n\\s*handleSuccess: \\@escaping \\(T\\) -> \\(\\),\\n\\s*handleError: \\@escaping \\(Int8, RustBuffer\\) -> \\(\\),\\n\\s*droppedCallback: UnsafeMutablePointer<UniffiForeignFutureDroppedCallbackStruct>\\n\\) \\{\\n\\s*let task = Task \\{\\n\\s*do \\{\\n\\s*handleSuccess\\(try await makeCall\\(\\)\\)\\n\\s*\\} catch \\{\\n\\s*handleError\\(CALL_UNEXPECTED_ERROR, FfiConverterString\\.lower\\(String\\(describing: error\\)\\)\\)\\n\\s*\\}\\n\\s*\\}|private func uniffiTraitInterfaceCallAsync<T>(\\n    makeCall: \\@escaping () async throws -> T,\\n    handleSuccess: \\@escaping (T) -> (),\\n    handleError: \\@escaping (Int8, RustBuffer) -> (),\\n    droppedCallback: UnsafeMutablePointer<UniffiForeignFutureDroppedCallbackStruct>\\n) {\\n    let makeCallSendable = UniffiUnsafeSendable(makeCall)\\n    let handleSuccessSendable = UniffiUnsafeSendable(handleSuccess)\\n    let handleErrorSendable = UniffiUnsafeSendable(handleError)\\n\\n    let task = Task {\\n        do {\\n            handleSuccessSendable.value(try await makeCallSendable.value())\\n        } catch {\\n            handleErrorSendable.value(\\n                CALL_UNEXPECTED_ERROR,\\n                FfiConverterString.lower(String(describing: error))\\n            )\\n        }\\n    }|sg",
                actr_swift.to_string_lossy().as_ref(),
            ]);
            cmd
        },
        "patch swift async callback helper",
    )?;

    run_checked(
        {
            let mut cmd = Command::new("perl");
            cmd.args([
                "-0777pi",
                "-e",
                "s|private func uniffiTraitInterfaceCallAsyncWithError<T, E>\\(\\n\\s*makeCall: \\@escaping \\(\\) async throws -> T,\\n\\s*handleSuccess: \\@escaping \\(T\\) -> \\(\\),\\n\\s*handleError: \\@escaping \\(Int8, RustBuffer\\) -> \\(\\),\\n\\s*lowerError: \\@escaping \\(E\\) -> RustBuffer,\\n\\s*droppedCallback: UnsafeMutablePointer<UniffiForeignFutureDroppedCallbackStruct>\\n\\) \\{\\n\\s*let task = Task \\{\\n\\s*do \\{\\n\\s*handleSuccess\\(try await makeCall\\(\\)\\)\\n\\s*\\} catch let error as E \\{\\n\\s*handleError\\(CALL_ERROR, lowerError\\(error\\)\\)\\n\\s*\\} catch \\{\\n\\s*handleError\\(CALL_UNEXPECTED_ERROR, FfiConverterString\\.lower\\(String\\(describing: error\\)\\)\\)\\n\\s*\\}\\n\\s*\\}|private func uniffiTraitInterfaceCallAsyncWithError<T, E>(\\n    makeCall: \\@escaping () async throws -> T,\\n    handleSuccess: \\@escaping (T) -> (),\\n    handleError: \\@escaping (Int8, RustBuffer) -> (),\\n    lowerError: \\@escaping (E) -> RustBuffer,\\n    droppedCallback: UnsafeMutablePointer<UniffiForeignFutureDroppedCallbackStruct>\\n) {\\n    let makeCallSendable = UniffiUnsafeSendable(makeCall)\\n    let handleSuccessSendable = UniffiUnsafeSendable(handleSuccess)\\n    let handleErrorSendable = UniffiUnsafeSendable(handleError)\\n    let lowerErrorSendable = UniffiUnsafeSendable(lowerError)\\n\\n    let task = Task {\\n        do {\\n            handleSuccessSendable.value(try await makeCallSendable.value())\\n        } catch let error as E {\\n            handleErrorSendable.value(CALL_ERROR, lowerErrorSendable.value(error))\\n        } catch {\\n            handleErrorSendable.value(\\n                CALL_UNEXPECTED_ERROR,\\n                FfiConverterString.lower(String(describing: error))\\n            )\\n        }\\n    }|sg",
                actr_swift.to_string_lossy().as_ref(),
            ]);
            cmd
        },
        "patch swift async error callback helper",
    )?;

    let header_path = if bindings_dir.join("actrFFI.h").exists() {
        bindings_dir.join("actrFFI.h")
    } else {
        bindings_dir.join("ActrFFI.h")
    };
    if !header_path.exists() {
        bail!(
            "generated swift ffi header not found at {}",
            header_path.display()
        );
    }
    fs::rename(&header_path, headers_dir.join("actrFFI.h")).with_context(|| {
        format!(
            "failed to move {} -> {}",
            header_path.display(),
            headers_dir.join("actrFFI.h").display()
        )
    })?;

    run_checked(
        {
            let mut cmd = Command::new("perl");
            cmd.args([
                "-0pi",
                "-e",
                "s|header \".*\"|header \"include/actrFFI.h\"|g",
                modulemap_path.to_string_lossy().as_ref(),
            ]);
            cmd
        },
        "patch swift modulemap header path",
    )?;

    run_checked(
        {
            let mut cmd = Command::new("xcodebuild");
            cmd.arg("-create-xcframework")
                .arg("-library")
                .arg(&static_lib)
                .arg("-headers")
                .arg(&headers_dir)
                .arg("-output")
                .arg(&output_path)
                .current_dir(&workspace);
            cmd
        },
        "xcodebuild create local swift ffi xcframework",
    )?;

    if !output_path.exists() {
        bail!(
            "local swift ffi xcframework not found at {}",
            output_path.display()
        );
    }

    let _ = SWIFT_XCFRAMEWORK.set(output_path.clone());
    Ok(output_path)
}

pub struct LoggedProcess {
    child: Child,
    logs: Arc<Mutex<Vec<String>>>,
}

impl LoggedProcess {
    pub fn spawn(mut cmd: Command, name: &str) -> Result<Self> {
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = cmd
            .spawn()
            .with_context(|| format!("failed to spawn process '{name}'"))?;
        let logs = Arc::new(Mutex::new(Vec::new()));
        drain_stream(child.stdout.take(), Arc::clone(&logs), name, "stdout");
        drain_stream(child.stderr.take(), Arc::clone(&logs), name, "stderr");
        Ok(Self { child, logs })
    }

    pub fn wait_for_log(&mut self, needle: &str, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            if self
                .logs
                .lock()
                .unwrap()
                .iter()
                .any(|line| line.contains(needle))
            {
                return true;
            }

            if matches!(self.child.try_wait(), Ok(Some(_))) {
                return false;
            }
            if Instant::now() > deadline {
                return false;
            }
            thread::sleep(Duration::from_millis(200));
        }
    }

    pub fn try_wait(&mut self) -> std::io::Result<Option<std::process::ExitStatus>> {
        self.child.try_wait()
    }

    pub fn logs(&self) -> String {
        self.logs.lock().unwrap().join("\n")
    }

    pub fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for LoggedProcess {
    fn drop(&mut self) {
        if let Ok(None) = self.child.try_wait() {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
    }
}

/// Mock signaling server for e2e tests (replaces LocalActrix).
///
/// Runs `MockSignalingServer` on a background tokio runtime and exposes
/// a synchronous API compatible with the test harness.
pub struct MockSignaling {
    pub signaling_ws_url: String,
    _runtime: tokio::runtime::Runtime,
    _server: std::sync::Arc<tokio::sync::Mutex<actr_mock_actrix::MockActrixServer>>,
}

impl MockSignaling {
    pub fn start() -> Result<Self> {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .context("failed to build tokio runtime for mock signaling")?;

        let server = rt
            .block_on(actr_mock_actrix::MockActrixServer::start())
            .context("failed to start mock signaling server")?;

        let ws_url = server.url();
        let server = std::sync::Arc::new(tokio::sync::Mutex::new(server));

        Ok(Self {
            signaling_ws_url: ws_url,
            _runtime: rt,
            _server: server,
        })
    }
}

pub struct LocalActrix {
    pub state_dir: TempDir,
    process: LoggedProcess,
    actrix_bin: PathBuf,
    config_path: PathBuf,
    http_port: u16,
    pub http_base_url: String,
    pub signaling_ws_url: String,
}

impl LocalActrix {
    pub fn start() -> Result<Self> {
        ensure_ks_port_available()?;
        let actrix_bin = ensure_actrix_binary()?;

        let state_dir = TempDir::new().context("failed to create actrix state dir")?;
        let http_port = free_port().context("failed to allocate HTTP port")?;
        let ice_port = free_port().context("failed to allocate ICE port")?;
        let config_path = state_dir.path().join("actrix-e2e.toml");
        write_actrix_config(&config_path, state_dir.path(), http_port, ice_port)?;

        let mut cmd = Command::new(&actrix_bin);
        cmd.arg("--config")
            .arg(&config_path)
            .current_dir(state_dir.path());
        let mut process = LoggedProcess::spawn(cmd, "actrix")?;

        let health_path = "/signaling/health";
        if !wait_for_http_ok(
            &mut process,
            http_port,
            health_path,
            Duration::from_secs(120),
        ) {
            let logs = process.logs();
            bail!(
                "actrix did not become healthy within timeout (http://127.0.0.1:{http_port}{health_path})\n{logs}"
            );
        }

        ensure_realm_exists(&state_dir.path().join("sqlite"), LOCAL_E2E_REALM_ID)?;

        Ok(Self {
            state_dir,
            process,
            actrix_bin,
            config_path,
            http_port,
            http_base_url: format!("http://127.0.0.1:{http_port}"),
            signaling_ws_url: format!("ws://127.0.0.1:{http_port}/signaling/ws"),
        })
    }

    /// Kill the actrix process without cleaning up state (SQLite remains).
    pub fn kill(&mut self) {
        self.process.kill();
    }

    /// Restart actrix using the same state directory and ports.
    /// The SQLite database is preserved so service registry can be restored.
    pub fn restart(&mut self) -> Result<()> {
        self.kill();
        // Brief pause to let the OS release the port
        thread::sleep(Duration::from_millis(500));

        let mut cmd = Command::new(&self.actrix_bin);
        cmd.arg("--config")
            .arg(&self.config_path)
            .current_dir(self.state_dir.path());
        let mut process = LoggedProcess::spawn(cmd, "actrix")?;

        let health_path = "/signaling/health";
        if !wait_for_http_ok(
            &mut process,
            self.http_port,
            health_path,
            Duration::from_secs(120),
        ) {
            let logs = process.logs();
            bail!("actrix did not become healthy after restart\n{logs}");
        }

        self.process = process;
        Ok(())
    }

    pub fn logs(&self) -> String {
        self.process.logs()
    }

    pub fn wait_for_log(&mut self, needle: &str, timeout: Duration) -> bool {
        self.process.wait_for_log(needle, timeout)
    }
}

pub struct LocalRustEchoService {
    _workspace: TempDir,
    process: LoggedProcess,
}

impl LocalRustEchoService {
    pub fn start(signaling_ws_url: &str) -> Result<Self> {
        let workspace = TempDir::new().context("failed to create rust echo e2e workspace")?;
        let project_name = "registry-echo-service";

        let init_out = run_actr(
            &[
                "init",
                "-l",
                "rust",
                "--template",
                "echo",
                "--role",
                "service",
                "--manufacturer",
                "acme",
                "--signaling",
                signaling_ws_url,
                project_name,
            ],
            workspace.path(),
        );
        ensure_success(&init_out, "actr init rust service")?;

        let service_dir = workspace.path().join(project_name);
        align_project_with_local_actrix(&service_dir)?;
        align_rust_project_with_workspace(&service_dir)?;
        let install_out = run_actr(&["install"], &service_dir);
        ensure_success(&install_out, "actr deps install rust service")?;
        let gen_out = run_actr(&["gen", "-l", "rust"], &service_dir);
        ensure_success(&gen_out, "actr gen rust service")?;
        cargo_build(&service_dir);

        let mut cmd = Command::new("cargo");
        cmd.args(["run"])
            .current_dir(&service_dir)
            .env("CARGO_TARGET_DIR", rust_e2e_target_dir());
        let mut process = LoggedProcess::spawn(cmd, "rust-echo-service")?;
        if !process.wait_for_log("EchoService registered", Duration::from_secs(180)) {
            let logs = process.logs();
            bail!("rust echo service did not register in time\n{logs}");
        }

        Ok(Self {
            _workspace: workspace,
            process,
        })
    }

    pub fn logs(&self) -> String {
        self.process.logs()
    }
}

fn rewrite_project_realm_id(project_dir: &Path, realm_id: u32) -> Result<()> {
    let actr_toml_path = project_dir.join("manifest.toml");
    let content = fs::read_to_string(&actr_toml_path)
        .with_context(|| format!("failed to read {}", actr_toml_path.display()))?;

    let mut replaced = false;
    let mut rewritten = String::with_capacity(content.len() + 32);
    for line in content.lines() {
        if line.trim_start().starts_with("realm_id =") {
            let prefix = line.split("realm_id").next().unwrap_or_default();
            rewritten.push_str(prefix);
            rewritten.push_str("realm_id = ");
            rewritten.push_str(&realm_id.to_string());
            rewritten.push('\n');
            replaced = true;
        } else {
            rewritten.push_str(line);
            rewritten.push('\n');
        }
    }

    if !replaced {
        bail!(
            "failed to rewrite realm_id in {}: realm_id line not found",
            actr_toml_path.display()
        );
    }

    if !content.ends_with('\n') {
        rewritten.pop();
    }

    fs::write(&actr_toml_path, rewritten)
        .with_context(|| format!("failed to write {}", actr_toml_path.display()))?;
    Ok(())
}

fn ensure_realm_exists(sqlite_dir: &Path, realm_id: u32) -> Result<()> {
    let db_path = sqlite_dir.join("actrix.db");
    let deadline = Instant::now() + Duration::from_secs(10);

    // Generate a stable realm secret for e2e tests
    let realm_secret = format!(
        "{:032x}{:032x}",
        realm_id as u64 ^ 0xDEAD_BEEF_CAFE_BABEu64,
        realm_id as u64 ^ 0x0123_4567_89AB_CDEFu64,
    );

    loop {
        match Connection::open(&db_path) {
            Ok(conn) => {
                conn.busy_timeout(Duration::from_secs(3))
                    .context("failed to set sqlite busy timeout")?;
                conn.execute_batch(
                    "CREATE TABLE IF NOT EXISTS realm (
                        id INTEGER PRIMARY KEY AUTOINCREMENT,
                        name TEXT NOT NULL,
                        status TEXT NOT NULL DEFAULT 'Active',
                        enabled INTEGER NOT NULL DEFAULT 1,
                        expires_at INTEGER,
                        created_at INTEGER NOT NULL,
                        updated_at INTEGER,
                        secret_current TEXT NOT NULL DEFAULT '',
                        secret_previous_hash TEXT,
                        secret_previous_valid_until INTEGER
                    );
                     INSERT OR IGNORE INTO sqlite_sequence(name, seq) VALUES('realm', 33554431);",
                )?;
                conn.execute(
                    "INSERT OR IGNORE INTO realm (id, name, status, enabled, created_at, secret_current)
                     VALUES (?1, 'e2e-realm', 'Active', 1, strftime('%s','now'), ?2)",
                    rusqlite::params![realm_id, realm_secret],
                )
                .context("failed to ensure local e2e realm exists")?;
                return Ok(());
            }
            Err(err) if Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(200));
                let _ = err;
            }
            Err(err) => {
                return Err(err)
                    .with_context(|| format!("failed to open actrix db {}", db_path.display()));
            }
        }
    }
}

fn drain_stream(
    stream: Option<impl Read + Send + 'static>,
    logs: Arc<Mutex<Vec<String>>>,
    name: &str,
    stream_name: &str,
) {
    if let Some(stream) = stream {
        let tag = format!("[{name}:{stream_name}]");
        thread::spawn(move || {
            for line in BufReader::new(stream).lines().map_while(Result::ok) {
                logs.lock().unwrap().push(format!("{tag} {line}"));
            }
        });
    }
}

fn ensure_ks_port_available() -> Result<()> {
    let probe = TcpListener::bind(("127.0.0.1", KS_GRPC_PORT))
        .with_context(|| format!("port {KS_GRPC_PORT} is already in use"))?;
    drop(probe);
    Ok(())
}

fn wait_for_http_ok(process: &mut LoggedProcess, port: u16, path: &str, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if http_get_ok(port, path) {
            return true;
        }
        if matches!(process.try_wait(), Ok(Some(_))) {
            return false;
        }
        if Instant::now() > deadline {
            return false;
        }
        thread::sleep(Duration::from_millis(250));
    }
}

fn http_get_ok(port: u16, path: &str) -> bool {
    let addr = format!("127.0.0.1:{port}");
    let timeout = Duration::from_secs(1);
    let Ok(mut stream) =
        TcpStream::connect_timeout(&addr.parse().expect("valid socket addr"), timeout)
    else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(timeout));
    let _ = stream.set_write_timeout(Some(timeout));

    let request = format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n");
    if stream.write_all(request.as_bytes()).is_err() {
        return false;
    }

    let mut response = String::new();
    if stream.read_to_string(&mut response).is_err() {
        return false;
    }

    let Some(status_line) = response.lines().next() else {
        return false;
    };
    status_line.contains(" 200 ")
}

fn free_port() -> Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0").context("failed to bind ephemeral port")?;
    let port = listener
        .local_addr()
        .context("failed to read local address")?
        .port();
    drop(listener);
    Ok(port)
}

fn ensure_actrix_binary() -> Result<PathBuf> {
    static ACTRIX_BIN: OnceLock<PathBuf> = OnceLock::new();
    if let Some(path) = ACTRIX_BIN.get() {
        return Ok(path.clone());
    }

    if let Ok(path) = std::env::var("ACTR_E2E_ACTRIX_BIN") {
        let binary_path = PathBuf::from(path);
        if binary_path.is_file() {
            let _ = ACTRIX_BIN.set(binary_path.clone());
            return Ok(binary_path);
        }
        bail!(
            "ACTR_E2E_ACTRIX_BIN points to a missing file: {}",
            binary_path.display()
        );
    }

    let repo =
        std::env::var("ACTR_E2E_ACTRIX_REPO").unwrap_or_else(|_| DEFAULT_ACTRIX_REPO.to_string());
    let cache_root = workspace_root().join("target/e2e-cache");
    fs::create_dir_all(&cache_root).context("failed to create e2e cache root")?;
    let _lock = DirLock::acquire(
        &cache_root.join("actrix-build.lock"),
        Duration::from_secs(600),
    )?;

    let latest_run = if std::env::var("ACTR_E2E_ACTRIX_REV").is_err() && artifact_download_enabled()
    {
        Some(latest_successful_actrix_run()?)
    } else {
        None
    };

    if artifact_download_enabled() {
        if let Some(binary_path) =
            try_ensure_actrix_artifact_binary(&cache_root, latest_run.as_ref())?
        {
            let _ = ACTRIX_BIN.set(binary_path.clone());
            return Ok(binary_path);
        }
    }

    let rev = resolve_actrix_source_rev(&repo, latest_run.as_ref())?;
    let checkout_dir = cache_root.join("actrix-checkout");
    ensure_actrix_checkout(&checkout_dir, &repo, &rev)?;

    let target_dir = cache_root.join("actrix-target");
    let mut build_cmd = Command::new("cargo");
    build_cmd
        .arg("build")
        .arg("--release")
        .arg("--bin")
        .arg("actrix")
        .current_dir(&checkout_dir)
        .env("CARGO_TARGET_DIR", &target_dir);
    run_checked(build_cmd, "cargo build --release --bin actrix")?;

    let binary_path = target_dir.join("release/actrix");
    if !binary_path.exists() {
        bail!("actrix binary not found at {}", binary_path.display());
    }

    let _ = ACTRIX_BIN.set(binary_path.clone());
    Ok(binary_path)
}

#[derive(Clone, Debug)]
struct ActrixRunInfo {
    run_id: String,
    head_sha: String,
}

fn artifact_download_enabled() -> bool {
    std::env::var("ACTR_E2E_ACTRIX_ARTIFACT")
        .map(|value| value != "0" && !value.eq_ignore_ascii_case("false"))
        .unwrap_or(true)
}

fn try_ensure_actrix_artifact_binary(
    cache_root: &Path,
    latest_run: Option<&ActrixRunInfo>,
) -> Result<Option<PathBuf>> {
    let Some(artifact_name) = current_actrix_artifact_name() else {
        return Ok(None);
    };
    let Some(latest_run) = latest_run else {
        return Ok(None);
    };

    let artifact_repo = std::env::var("ACTR_E2E_ACTRIX_ARTIFACT_REPO")
        .unwrap_or_else(|_| DEFAULT_ACTRIX_ARTIFACT_REPO.to_string());

    let artifact_dir = cache_root
        .join("actrix-artifacts")
        .join(&latest_run.run_id)
        .join(artifact_name);
    let binary_path = artifact_dir.join("actrix");
    if binary_path.is_file() {
        ensure_executable(&binary_path)?;
        return Ok(Some(binary_path));
    }

    fs::create_dir_all(&artifact_dir)
        .with_context(|| format!("failed to create {}", artifact_dir.display()))?;

    let download_output = Command::new("gh")
        .args([
            "run",
            "download",
            &latest_run.run_id,
            "-R",
            &artifact_repo,
            "-n",
            artifact_name,
        ])
        .current_dir(&artifact_dir)
        .output()
        .context("failed to invoke gh run download for actrix artifact")?;

    if !download_output.status.success() {
        eprintln!(
            "actrix artifact download failed, falling back to source build:\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&download_output.stdout),
            String::from_utf8_lossy(&download_output.stderr)
        );
        return Ok(None);
    }

    if !binary_path.is_file() {
        eprintln!(
            "actrix artifact downloaded but binary missing at {}, falling back to source build",
            binary_path.display()
        );
        return Ok(None);
    }

    ensure_executable(&binary_path)?;
    Ok(Some(binary_path))
}

fn current_actrix_artifact_name() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => Some("actrix-linux-x86_64"),
        ("macos", "aarch64") => Some("actrix-macos-arm64"),
        _ => None,
    }
}

fn latest_successful_actrix_run() -> Result<ActrixRunInfo> {
    let artifact_repo = std::env::var("ACTR_E2E_ACTRIX_ARTIFACT_REPO")
        .unwrap_or_else(|_| DEFAULT_ACTRIX_ARTIFACT_REPO.to_string());
    let workflow = std::env::var("ACTR_E2E_ACTRIX_ARTIFACT_WORKFLOW")
        .unwrap_or_else(|_| DEFAULT_ACTRIX_ARTIFACT_WORKFLOW.to_string());
    let branch = std::env::var("ACTR_E2E_ACTRIX_ARTIFACT_BRANCH")
        .unwrap_or_else(|_| DEFAULT_ACTRIX_ARTIFACT_BRANCH.to_string());
    let route = format!(
        "repos/{artifact_repo}/actions/workflows/{workflow}/runs?branch={branch}&status=success&per_page=1"
    );
    let output = Command::new("gh")
        .args(["api", &route])
        .output()
        .context("failed to invoke gh api for latest actrix workflow run")?;

    if !output.status.success() {
        bail!(
            "failed to resolve latest actrix workflow run from GitHub:\nstdout: {}\nstderr: {}\nset ACTR_E2E_ACTRIX_REV or ACTR_E2E_ACTRIX_BIN to override",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let payload: Value = serde_json::from_slice(&output.stdout)
        .context("failed to parse latest actrix workflow run payload")?;
    let run = payload
        .get("workflow_runs")
        .and_then(Value::as_array)
        .and_then(|runs| runs.first())
        .context("latest actrix workflow run payload did not include a successful run")?;
    let run_id = run
        .get("id")
        .and_then(Value::as_u64)
        .context("latest actrix workflow run payload did not include a successful run id")?;
    let head_sha = run
        .get("head_sha")
        .and_then(Value::as_str)
        .context("latest actrix workflow run payload did not include head_sha")?;

    Ok(ActrixRunInfo {
        run_id: run_id.to_string(),
        head_sha: head_sha.to_string(),
    })
}

fn resolve_actrix_source_rev(repo: &str, latest_run: Option<&ActrixRunInfo>) -> Result<String> {
    if let Ok(rev) = std::env::var("ACTR_E2E_ACTRIX_REV") {
        return Ok(rev);
    }

    if let Some(latest_run) = latest_run {
        return Ok(latest_run.head_sha.clone());
    }

    let route = format!(
        "refs/heads/{}",
        std::env::var("ACTR_E2E_ACTRIX_ARTIFACT_BRANCH")
            .unwrap_or_else(|_| DEFAULT_ACTRIX_ARTIFACT_BRANCH.to_string())
    );
    let output = Command::new("git")
        .args(["ls-remote", repo, &route])
        .output()
        .context("failed to invoke git ls-remote for actrix revision")?;

    if !output.status.success() {
        bail!(
            "failed to resolve latest actrix revision:\nstdout: {}\nstderr: {}\nset ACTR_E2E_ACTRIX_REV or ACTR_E2E_ACTRIX_BIN to override",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let rev = stdout
        .split_whitespace()
        .next()
        .context("git ls-remote did not return a revision for actrix")?;

    Ok(rev.to_string())
}

fn ensure_executable(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mut perms = fs::metadata(path)
            .with_context(|| format!("failed to read metadata for {}", path.display()))?
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(path, perms)
            .with_context(|| format!("failed to mark {} executable", path.display()))?;
    }

    Ok(())
}

fn ensure_actrix_checkout(checkout_dir: &Path, repo: &str, rev: &str) -> Result<()> {
    if !checkout_dir.join(".git").exists() {
        clone_actrix_repo(checkout_dir, repo)?;
    } else {
        let current_remote = Command::new("git")
            .args(["config", "--get", "remote.origin.url"])
            .current_dir(checkout_dir)
            .output()
            .ok()
            .and_then(|out| {
                if out.status.success() {
                    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
                } else {
                    None
                }
            });
        if current_remote.as_deref() != Some(repo) {
            fs::remove_dir_all(checkout_dir).with_context(|| {
                format!(
                    "failed to remove stale actrix checkout {}",
                    checkout_dir.display()
                )
            })?;
            clone_actrix_repo(checkout_dir, repo)?;
        }
    }

    run_checked(
        {
            let mut cmd = Command::new("git");
            cmd.arg("fetch")
                .arg("--depth")
                .arg("1")
                .arg("origin")
                .arg(rev)
                .current_dir(checkout_dir);
            cmd
        },
        "git fetch actrix revision",
    )?;
    run_checked(
        {
            let mut cmd = Command::new("git");
            cmd.arg("checkout")
                .arg("--detach")
                .arg(rev)
                .current_dir(checkout_dir);
            cmd
        },
        "git checkout actrix revision",
    )?;
    Ok(())
}

fn clone_actrix_repo(checkout_dir: &Path, repo: &str) -> Result<()> {
    if let Some(parent) = checkout_dir.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!("failed to create parent dir for {}", checkout_dir.display())
        })?;
    }
    run_checked(
        {
            let mut cmd = Command::new("git");
            cmd.arg("clone")
                .arg("--filter=blob:none")
                .arg(repo)
                .arg(checkout_dir)
                .current_dir(
                    checkout_dir
                        .parent()
                        .expect("actrix checkout dir should have parent"),
                );
            cmd
        },
        "git clone actrix",
    )?;
    Ok(())
}

fn run_checked(mut cmd: Command, context_name: &str) -> Result<Output> {
    let output = cmd
        .output()
        .with_context(|| format!("{context_name}: failed to execute"))?;
    if output.status.success() {
        return Ok(output);
    }

    bail!(
        "{} failed:\nstdout: {}\nstderr: {}",
        context_name,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn write_actrix_config(
    config_path: &Path,
    state_dir: &Path,
    http_port: u16,
    ice_port: u16,
) -> Result<()> {
    let sqlite_dir = state_dir.join("sqlite");
    let log_dir = state_dir.join("logs");
    fs::create_dir_all(&sqlite_dir).context("failed to create sqlite dir")?;
    fs::create_dir_all(&log_dir).context("failed to create log dir")?;

    let sqlite_path = normalize_path_for_toml(&sqlite_dir);

    let config = format!(
        r#"enable = 25
name = "actrix-e2e"
env = "dev"
sqlite_path = "{sqlite_path}"
location_tag = "local,e2e,default"
actrix_shared_key = "actrix-e2e-shared-key-0123456789abcdef"

[control]
head = "admin_ui"

[control.admin_ui]
password = "e2e-test-password"

[bind.http]
domain_name = "127.0.0.1"
advertised_ip = "127.0.0.1"
ip = "127.0.0.1"
port = {http_port}

[bind.ice]
domain_name = "127.0.0.1"
ip = "127.0.0.1"
port = {ice_port}
advertised_ip = "127.0.0.1"
advertised_port = {ice_port}

[turn]
advertised_ip = "127.0.0.1"
advertised_port = {ice_port}
relay_port_range = "49152-49200"
realm = "local.actrix"

[services.ks]

[services.signer]

[services.ais]

[services.signaling]

[services.signaling.server]
ws_path = "/signaling"
"#
    );

    fs::write(config_path, config)
        .with_context(|| format!("failed to write actrix config to {}", config_path.display()))?;
    Ok(())
}

fn normalize_path_for_toml(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn copy_dir_all(src: &Path, dst: &Path) -> Result<()> {
    fs::create_dir_all(dst).with_context(|| format!("failed to create {}", dst.display()))?;

    for entry in
        fs::read_dir(src).with_context(|| format!("failed to read directory {}", src.display()))?
    {
        let entry = entry.with_context(|| format!("failed to read entry in {}", src.display()))?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if entry
            .file_type()
            .with_context(|| format!("failed to read file type for {}", src_path.display()))?
            .is_dir()
        {
            copy_dir_all(&src_path, &dst_path)?;
        } else {
            fs::copy(&src_path, &dst_path).with_context(|| {
                format!(
                    "failed to copy {} -> {}",
                    src_path.display(),
                    dst_path.display()
                )
            })?;
        }
    }

    Ok(())
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("actr-cli should live under workspace root")
        .to_path_buf()
}

struct DirLock {
    path: PathBuf,
}

impl DirLock {
    fn acquire(path: &Path, timeout: Duration) -> Result<Self> {
        let deadline = Instant::now() + timeout;
        loop {
            match fs::create_dir(path) {
                Ok(()) => {
                    return Ok(Self {
                        path: path.to_path_buf(),
                    });
                }
                Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                    if Instant::now() > deadline {
                        bail!("timed out waiting for lock directory {}", path.display());
                    }
                    thread::sleep(Duration::from_millis(250));
                }
                Err(err) => return Err(err).context("failed to acquire lock directory"),
            }
        }
    }
}

impl Drop for DirLock {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}
