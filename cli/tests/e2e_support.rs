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
use tempfile::TempDir;

pub const DEFAULT_ACTRIX_REPO: &str = "https://github.com/actor-rtc/actrix.git";
pub const DEFAULT_ACTRIX_REV: &str = "28bf45d0566d7f73a5bd99802c6c36524acea57e";
pub const LOCAL_E2E_REALM_ID: u32 = 1001;
const KS_GRPC_PORT: u16 = 50052;

pub fn actr_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_actr"))
}

pub fn run_actr(args: &[&str], cwd: &Path) -> Output {
    let mut cmd = Command::new(actr_bin());
    cmd.args(args).current_dir(cwd);

    // Make Swift template/e2e use workspace-local bindings first to avoid stale remote package behavior.
    let local_swift = workspace_root().join("bindings/swift");
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

pub fn align_rust_project_with_workspace(project_dir: &Path) -> Result<()> {
    let cargo_toml_path = project_dir.join("Cargo.toml");
    let mut content = fs::read_to_string(&cargo_toml_path)
        .with_context(|| format!("failed to read {}", cargo_toml_path.display()))?;

    if content.contains("[patch.crates-io]") && content.contains("actr-runtime = { path =") {
        return Ok(());
    }

    let workspace = workspace_root();
    let mut patch = String::from("\n[patch.crates-io]\n");
    let crates = [
        ("actr", workspace.clone()),
        ("actr-protocol", workspace.join("core/protocol")),
        ("actr-framework", workspace.join("core/framework")),
        ("actr-runtime", workspace.join("core/runtime")),
        ("actr-config", workspace.join("core/config")),
        ("actr-version", workspace.join("core/version")),
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
    let output_path = workspace.join("bindings/swift/.e2e/ActrFFI.xcframework");
    if output_path.exists() {
        let _ = SWIFT_XCFRAMEWORK.set(output_path.clone());
        return Ok(output_path);
    }

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

    if !output_path.exists() {
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

        let static_lib = workspace.join("target/aarch64-apple-darwin/release/libactr.a");
        if !static_lib.exists() {
            bail!(
                "local swift ffi static library not found at {}",
                static_lib.display()
            );
        }

        let headers = workspace.join("bindings/swift/ActrBindings/include");
        if !headers.join("actrFFI.h").exists() {
            bail!(
                "swift ffi headers not found at {}",
                headers.join("actrFFI.h").display()
            );
        }

        run_checked(
            {
                let mut cmd = Command::new("xcodebuild");
                cmd.arg("-create-xcframework")
                    .arg("-library")
                    .arg(&static_lib)
                    .arg("-headers")
                    .arg(&headers)
                    .arg("-output")
                    .arg(&output_path)
                    .current_dir(&workspace);
                cmd
            },
            "xcodebuild create local swift ffi xcframework",
        )?;
    }

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
    name: String,
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
        Ok(Self {
            name: name.to_string(),
            child,
            logs,
        })
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

pub struct LocalActrix {
    _state_dir: TempDir,
    process: LoggedProcess,
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
            _state_dir: state_dir,
            process,
            http_base_url: format!("http://127.0.0.1:{http_port}"),
            signaling_ws_url: format!("ws://127.0.0.1:{http_port}/signaling/ws"),
        })
    }

    pub fn logs(&self) -> String {
        self.process.logs()
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
        disable_acl(&service_dir)?;
        align_rust_project_with_workspace(&service_dir)?;
        let install_out = run_actr(&["install"], &service_dir);
        ensure_success(&install_out, "actr install rust service")?;
        let gen_out = run_actr(&["gen", "-l", "rust"], &service_dir);
        ensure_success(&gen_out, "actr gen rust service")?;
        cargo_build(&service_dir);

        let mut cmd = Command::new("cargo");
        cmd.args(["run"]).current_dir(&service_dir);
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
    let actr_toml_path = project_dir.join("Actr.toml");
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

fn disable_acl(project_dir: &Path) -> Result<()> {
    let actr_toml_path = project_dir.join("Actr.toml");
    let content = fs::read_to_string(&actr_toml_path)
        .with_context(|| format!("failed to read {}", actr_toml_path.display()))?;

    let acl_start = content.find("[acl]").with_context(|| {
        format!(
            "failed to find [acl] section in {}",
            actr_toml_path.display()
        )
    })?;
    let rewritten = content[..acl_start].trim_end().to_string() + "\n";

    fs::write(&actr_toml_path, rewritten)
        .with_context(|| format!("failed to write {}", actr_toml_path.display()))?;
    Ok(())
}

fn ensure_realm_exists(sqlite_dir: &Path, realm_id: u32) -> Result<()> {
    let db_path = sqlite_dir.join("actrix.db");
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match Connection::open(&db_path) {
            Ok(conn) => {
                conn.busy_timeout(Duration::from_secs(3))
                    .context("failed to set sqlite busy timeout")?;
                conn.execute(
                    "INSERT OR IGNORE INTO realm (realm_id, name, status, expires_at, created_at, updated_at)
                     VALUES (?1, 'e2e-realm', 'Normal', NULL, strftime('%s','now'), strftime('%s','now'))",
                    [realm_id],
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

    let repo =
        std::env::var("ACTR_E2E_ACTRIX_REPO").unwrap_or_else(|_| DEFAULT_ACTRIX_REPO.to_string());
    let rev =
        std::env::var("ACTR_E2E_ACTRIX_REV").unwrap_or_else(|_| DEFAULT_ACTRIX_REV.to_string());
    let cache_root = workspace_root().join("target/e2e-cache");
    fs::create_dir_all(&cache_root).context("failed to create e2e cache root")?;
    let _lock = DirLock::acquire(
        &cache_root.join("actrix-build.lock"),
        Duration::from_secs(600),
    )?;

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
    let log_path = normalize_path_for_toml(&log_dir);

    let config = format!(
        r#"enable = 25
name = "actrix-e2e"
env = "dev"
sqlite_path = "{sqlite_path}"
location_tag = "local,e2e,default"
actrix_shared_key = "actrix-e2e-shared-key-0123456789abcdef"

[observability]
filter_level = "info"

[observability.log]
output = "console"
rotate = false
path = "{log_path}"

[bind.http]
domain_name = "127.0.0.1"
advertised_ip = "127.0.0.1"
ip = "127.0.0.1"
port = {http_port}

[bind.ice]
domain_name = "127.0.0.1"
ip = "127.0.0.1"
port = {ice_port}

[turn]
advertised_ip = "127.0.0.1"
advertised_port = {ice_port}
relay_port_range = "49152-49200"
realm = "local.actrix"

[services.ks]
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
