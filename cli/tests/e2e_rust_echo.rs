//! End-to-end tests for Rust echo template.
//!
//! These tests run against a local Actrix instance and local services only.
//! Run with: `cargo test --test e2e_rust_echo -- --ignored --test-threads=1`

mod e2e_support;

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use e2e_support::{
    LocalActrix, LoggedProcess, align_project_with_local_actrix, align_rust_project_with_workspace,
    assert_success, cargo_build, pin_echo_service_dependency_version, random_manufacturer,
    run_actr, rust_e2e_target_dir,
};
use tempfile::TempDir;

#[test]
#[ignore] // Slow e2e (~200s), run explicitly with --ignored
fn rust_echo_e2e_service_and_app() {
    let actrix = LocalActrix::start().expect("failed to start local actrix");
    let tmp = TempDir::new().unwrap();
    let mfr = random_manufacturer();

    let init_out = run_actr(
        &[
            "init",
            "-l",
            "rust",
            "--template",
            "echo",
            "--role",
            "both",
            "--signaling",
            &actrix.signaling_ws_url,
            "--manufacturer",
            &mfr,
            "e2e",
        ],
        tmp.path(),
    );
    assert_success(&init_out, "actr init --role both");

    let svc_dir = tmp.path().join("e2e/echo-service");
    let app_dir = tmp.path().join("e2e/echo-app");
    assert!(svc_dir.exists(), "echo-service dir");
    assert!(app_dir.exists(), "echo-app dir");
    align_project_with_local_actrix(&svc_dir).expect("failed to set local realm for svc");
    align_project_with_local_actrix(&app_dir).expect("failed to set local realm for app");
    pin_echo_service_dependency_version(&app_dir, &mfr)
        .expect("failed to pin app echo dependency version");
    align_rust_project_with_workspace(&svc_dir).expect("failed to patch svc Cargo.toml");
    align_rust_project_with_workspace(&app_dir).expect("failed to patch app Cargo.toml");

    assert_success(&run_actr(&["install"], &svc_dir), "actr install (svc)");
    assert_success(
        &run_actr(&["gen", "-l", "rust"], &svc_dir),
        "actr gen (svc)",
    );
    cargo_build(&svc_dir);

    let mut svc_cmd = Command::new("cargo");
    svc_cmd
        .args(["run"])
        .current_dir(&svc_dir)
        .env("CARGO_TARGET_DIR", rust_e2e_target_dir());
    let mut svc = LoggedProcess::spawn(svc_cmd, "rust-e2e-service").expect("start rust service");
    assert!(
        svc.wait_for_log("EchoService registered", Duration::from_secs(180)),
        "service not ready within timeout:\n{}",
        svc.logs()
    );

    assert_success(&run_actr(&["install"], &app_dir), "actr install (app)");
    assert_success(
        &run_actr(&["gen", "-l", "rust"], &app_dir),
        "actr gen (app)",
    );
    cargo_build(&app_dir);

    let mut app = Command::new("cargo")
        .args(["run"])
        .current_dir(&app_dir)
        .env("CARGO_TARGET_DIR", rust_e2e_target_dir())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        match app.try_wait().unwrap() {
            Some(_) => break,
            None if Instant::now() > deadline => {
                app.kill().ok();
                panic!("app did not exit within 60s");
            }
            None => std::thread::sleep(Duration::from_millis(500)),
        }
    }

    let app_out = app.wait_with_output().unwrap();
    let stdout = String::from_utf8_lossy(&app_out.stdout);
    let stderr = String::from_utf8_lossy(&app_out.stderr);
    assert!(
        app_out.status.success(),
        "app failed:\nstdout: {stdout}\nstderr: {stderr}\nservice logs:\n{}\nactrix logs:\n{}",
        svc.logs(),
        actrix.logs()
    );
    assert!(
        stdout.contains("Echo reply:"),
        "missing echo reply in:\n{stdout}"
    );
    assert!(
        svc.logs().contains("Received echo request: hello"),
        "service missing request log:\n{}",
        svc.logs()
    );
}

/// 复现 Ghost Candidates Bug。
///
/// 正确的触发路径：
///   1. 启动 actrix
///   2. 启动 EchoService → 注册 → AIS 分配 ActorId A (serial=xxx1) → SQLite 落盘
///   3. Kill actrix（服务进程可以继续跑或者也 kill，不影响 SQLite 缓存）
///   4. 重启 actrix → restore_from_storage() 从 SQLite 加载 ActorId A → 内存已有 A
///   5. 启动全新 EchoService 进程 → 向 actrix 发 RegisterRequest
///      → AIS 分配新 ActorId B (serial=xxx2，新进程全新注册)
///      → register_service_full() 直接 push B 进 services Vec ← ⚠️ 没有去重！
///      → 内存: services["EchoService"] = [A（幽灵）, B（真实）]
///   6. Client 触发 RouteCandidatesRequest → actrix 返回 2 个候选 → 幽灵 Bug！
///
/// Run:
///   ACTR_E2E_ACTRIX_BIN=.../actrix cargo test --test e2e_rust_echo ghost_candidates -- --nocapture
#[test]
#[ignore] // Slow e2e, requires actrix binary. Run explicitly with --ignored
fn ghost_candidates_after_actrix_restart() {
    // 自动探测 actrix binary
    if std::env::var("ACTR_E2E_ACTRIX_BIN").is_err() {
        let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("cli should be under workspace root");
        for bin in &[
            workspace.join("../actrix/target/debug/actrix"),
            workspace.join("../../actrix/target/debug/actrix"),
        ] {
            if let Ok(canonical) = bin.canonicalize() {
                unsafe {
                    std::env::set_var("ACTR_E2E_ACTRIX_BIN", &canonical);
                }
                eprintln!(
                    "[ghost] ACTR_E2E_ACTRIX_BIN auto-set to {}",
                    canonical.display()
                );
                break;
            }
        }
    }

    let tmp = TempDir::new().unwrap();
    let mfr = random_manufacturer();
    eprintln!("[ghost] manufacturer = {mfr}");

    // ── Step 1: 启动 actrix (phase-1) ───────────────────────────────────────
    eprintln!("[ghost] Step 1: Starting actrix (phase-1) ...");
    let mut actrix = LocalActrix::start().expect("failed to start local actrix");
    let signaling_url = actrix.signaling_ws_url.clone();
    eprintln!("[ghost] Step 1: actrix started at {signaling_url}");

    // ── Step 2: 初始化并构建 EchoService ────────────────────────────────────
    eprintln!("[ghost] Step 2: actr init + build EchoService ...");
    let init_out = run_actr(
        &[
            "init",
            "-l",
            "rust",
            "--template",
            "echo",
            "--role",
            "service",
            "--signaling",
            &signaling_url,
            "--manufacturer",
            &mfr,
            "e2e-ghost-svc",
        ],
        tmp.path(),
    );
    assert_success(&init_out, "actr init --role service");
    let svc_dir = tmp.path().join("e2e-ghost-svc");
    align_project_with_local_actrix(&svc_dir).expect("align realm");
    align_rust_project_with_workspace(&svc_dir).expect("patch Cargo.toml");
    assert_success(&run_actr(&["install"], &svc_dir), "actr install");
    assert_success(&run_actr(&["gen", "-l", "rust"], &svc_dir), "actr gen");
    cargo_build(&svc_dir);
    eprintln!("[ghost] Step 2: build done");

    // ── Step 3: EchoService 第 1 次启动 → 注册 → SQLite 落盘 ────────────────
    eprintln!("[ghost] Step 3: EchoService 1st start → register → ActorId A ...");
    let mut svc1 = LoggedProcess::spawn(
        {
            let mut c = Command::new("cargo");
            c.args(["run"])
                .current_dir(&svc_dir)
                .env("CARGO_TARGET_DIR", rust_e2e_target_dir());
            c
        },
        "ghost-svc-1",
    )
    .expect("start svc1");
    assert!(
        svc1.wait_for_log("EchoService registered", Duration::from_secs(60)),
        "svc1 did not register:\n{}",
        svc1.logs()
    );
    eprintln!("[ghost] Step 3: svc1 registered (ActorId A) ✅");
    // 等 SQLite 异步落盘（actrix 内部用 tokio::spawn 写库）
    std::thread::sleep(Duration::from_secs(3));

    // ── Step 4: Kill actrix，SQLite 缓存保留 ActorId A ──────────────────────
    eprintln!("[ghost] Step 4: Kill actrix (SQLite keeps ActorId A) ...");
    actrix.kill();
    // 同时 kill svc1（它会尝试重连，但 actrix 已经不在了）
    drop(svc1);
    std::thread::sleep(Duration::from_millis(500));

    // ── Step 5: 重启 actrix → restore_from_storage() → 内存加载 ActorId A ──
    eprintln!("[ghost] Step 5: Restart actrix → restore_from_storage → memory has ActorId A ...");
    actrix.restart().expect("failed to restart actrix");
    // 等 actrix 完成 restore_from_storage
    let restored = actrix.wait_for_log("从缓存恢复", Duration::from_secs(30));
    eprintln!("[ghost] Step 5: restore log found = {restored}");
    // 即使没打印也继续——restore 可能很快
    std::thread::sleep(Duration::from_secs(1));
    eprintln!("[ghost] Step 5: actrix restarted ✅ (SQLite ActorId A now in memory)");

    // ── Step 6: 全新 EchoService 进程注册 → AIS 分配 ActorId B ──────────────
    // 此时 actrix 内存里已有 ActorId A（来自 SQLite restore）
    // 新进程向 AIS 发 RegisterRequest → AIS 分配新 serial → ActorId B
    // register_service_full() 把 B push 进 services["EchoService"]
    // 内存: [A（幽灵）, B（真实）] → 2 条！
    eprintln!("[ghost] Step 6: EchoService 2nd start (fresh process) → ActorId B ...");
    let mut svc2 = LoggedProcess::spawn(
        {
            let mut c = Command::new("cargo");
            c.args(["run"])
                .current_dir(&svc_dir)
                .env("CARGO_TARGET_DIR", rust_e2e_target_dir());
            c
        },
        "ghost-svc-2",
    )
    .expect("start svc2");
    assert!(
        svc2.wait_for_log("EchoService registered", Duration::from_secs(60)),
        "svc2 did not register:\n{}",
        svc2.logs()
    );
    eprintln!("[ghost] Step 6: svc2 registered (ActorId B) ✅");
    std::thread::sleep(Duration::from_secs(1));

    // ── Step 7: 运行 EchoClient → 触发 RouteCandidatesRequest ───────────────
    eprintln!("[ghost] Step 7: Init + build + run EchoClient ...");
    // 使用 Phase-2 actrix 的地址
    let signaling_url2 = actrix.signaling_ws_url.clone();
    let init_client_out = run_actr(
        &[
            "init",
            "-l",
            "rust",
            "--template",
            "echo",
            "--role",
            "app",
            "--signaling",
            &signaling_url2,
            "--manufacturer",
            &mfr,
            "e2e-ghost-client",
        ],
        tmp.path(),
    );
    assert_success(&init_client_out, "actr init --role app");
    let client_dir = {
        let d = tmp.path().join("e2e-ghost-client");
        if d.join("Cargo.toml").exists() {
            d
        } else {
            d.join("echo-app")
        }
    };
    align_project_with_local_actrix(&client_dir).expect("align client realm");
    align_rust_project_with_workspace(&client_dir).expect("patch client Cargo.toml");
    assert_success(
        &run_actr(&["install"], &client_dir),
        "actr install (client)",
    );
    assert_success(
        &run_actr(&["gen", "-l", "rust"], &client_dir),
        "actr gen (client)",
    );
    cargo_build(&client_dir);

    let client_out = Command::new("cargo")
        .args(["run"])
        .current_dir(&client_dir)
        .env("CARGO_TARGET_DIR", rust_e2e_target_dir())
        .output()
        .expect("run client");
    let client_stdout = String::from_utf8_lossy(&client_out.stdout);
    let client_stderr = String::from_utf8_lossy(&client_out.stderr);
    eprintln!("[ghost] client stdout: {client_stdout}");
    eprintln!(
        "[ghost] client stderr (tail): {}",
        client_stderr
            .lines()
            .rev()
            .take(10)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("\n")
    );

    // ── Step 8: 验证候选数量 ──────────────────────────────────────────────────
    let actrix_logs = actrix.logs();
    let candidate_lines: Vec<&str> = actrix_logs
        .lines()
        .filter(|l| l.contains("找到") && l.contains("类型的候选实例"))
        .collect();
    eprintln!(
        "[ghost] RouteCandidates log lines:\n  {}",
        candidate_lines.join("\n  ")
    );

    let candidate_count: Option<usize> = candidate_lines.iter().rev().find_map(|line| {
        let after = line.split("找到").nth(1)?;
        after
            .trim_start()
            .split_whitespace()
            .next()?
            .parse::<usize>()
            .ok()
    });
    eprintln!("[ghost] candidate_count = {candidate_count:?}");
    eprintln!(
        "[ghost] actrix phase-2 logs (last 60 lines):\n{}",
        actrix_logs
            .lines()
            .rev()
            .take(60)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("\n")
    );

    assert!(
        candidate_count.is_some(),
        "[SETUP FAILED] RouteCandidatesRequest not triggered by client.\n\
         stdout:\n{client_stdout}\nstderr:\n{client_stderr}\nactrix:\n{actrix_logs}"
    );

    let n = candidate_count.unwrap();
    assert_eq!(
        n,
        2,
        "[BUG NOT REPRODUCED] Expected 2 candidates (ActorId A ghost + ActorId B real), got {}.\n\
         Check: did actrix restore ActorId A from SQLite? Did svc2 get a new serial from AIS?\n\
         svc2 logs:\n{}\nactrix logs:\n{actrix_logs}",
        n,
        svc2.logs()
    );

    println!(
        "[ghost] ✅ Bug reproduced! RouteCandidatesRequest returned {} candidates.\n\
         - ActorId A: loaded from SQLite at actrix startup (restore_from_storage)\n\
         - ActorId B: pushed by svc2 fresh registration (new serial from AIS)\n\
         - register_service_full() has no dedup → both A and B coexist in memory\n\
         FIX: register_service_full() must evict same-service_name entries before pushing.\n\
         manufacturer = {mfr}",
        n
    );
}
