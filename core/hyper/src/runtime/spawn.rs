//! Mode 2 子进程 spawn 实现
//!
//! 负责以子进程方式启动 ActrSystem+Workload，
//! 并将 AIS credential 通过环境变量安全传递给子进程。

use std::collections::HashMap;
use std::path::PathBuf;

use base64::Engine as _;
use tokio::process::Command;
use tracing::info;

use crate::error::{HyperError, HyperResult};

use super::handle::ChildProcessHandle;

/// Mode 2 子进程 spawn 配置
pub struct SpawnConfig {
    /// 可执行文件路径（已通过签名验证）
    pub binary_path: PathBuf,
    /// 传递给子进程的命令行参数
    pub args: Vec<String>,
    /// credential 字节（base64 编码后写入 ACTR_CREDENTIAL 环境变量）
    pub credential: Vec<u8>,
    /// 额外环境变量（在 ACTR_CREDENTIAL 之外）
    pub extra_env: HashMap<String, String>,
    /// restart policy
    pub restart_policy: RestartPolicy,
    /// ActrType 字符串（调试/日志用）
    pub actr_type: String,
}

/// 重启策略
#[derive(Debug, Clone)]
pub enum RestartPolicy {
    /// 不自动重启
    Never,
    /// 退出码非 0 时重启，最多 N 次
    OnFailure { max_retries: u32 },
    /// 总是重启（除非主动 shutdown），最多 N 次
    Always { max_retries: u32 },
}

/// 启动 Mode 2 子进程
///
/// 步骤：
/// 1. 将 credential base64 编码，写入 ACTR_CREDENTIAL 环境变量
/// 2. 设置额外环境变量
/// 3. spawn 子进程
/// 4. 记录 spawn 成功日志（含 binary path 和 pid）
/// 5. 返回 ChildProcessHandle
pub async fn spawn(config: SpawnConfig) -> HyperResult<ChildProcessHandle> {
    // 将 credential 编码为 base64，避免二进制数据直接写入环境变量
    let credential_b64 = base64::engine::general_purpose::STANDARD.encode(&config.credential);

    let mut cmd = Command::new(&config.binary_path);

    // 传递命令行参数
    cmd.args(&config.args);

    // 传递 AIS credential
    cmd.env("ACTR_CREDENTIAL", &credential_b64);

    // 传递额外环境变量
    for (key, value) in &config.extra_env {
        cmd.env(key, value);
    }

    // spawn 子进程，保留 stdin/stdout/stderr 为继承（父进程不做 I/O 代理）
    let child = cmd.spawn().map_err(|e| {
        HyperError::Runtime(format!(
            "spawn 子进程失败（binary: {}）: {e}",
            config.binary_path.display()
        ))
    })?;

    // 获取 PID（spawn 成功后一定有 id）
    let pid = child.id().ok_or_else(|| {
        HyperError::Runtime("spawn 后无法获取子进程 PID".to_string())
    })?;

    info!(
        pid,
        actr_type = %config.actr_type,
        binary = %config.binary_path.display(),
        "Mode 2 子进程已启动"
    );

    Ok(ChildProcessHandle::from_child(pid, config.actr_type, child))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// 测试 spawn() 能成功启动真实子进程（true 命令立即退出 0）
    #[tokio::test]
    async fn spawn_true_succeeds() {
        let config = SpawnConfig {
            binary_path: PathBuf::from("/usr/bin/true"),
            args: vec![],
            credential: vec![1, 2, 3],
            extra_env: HashMap::new(),
            restart_policy: RestartPolicy::Never,
            actr_type: "test:unit".to_string(),
        };

        let mut handle = spawn(config).await.expect("spawn 应成功");
        assert!(handle.pid > 0, "PID 应大于 0");

        let state = handle.wait().await.expect("wait 应成功");
        assert_eq!(
            state,
            crate::runtime::handle::ChildProcessState::Exited(0),
            "true 命令应以退出码 0 结束"
        );
    }

    /// 测试 ACTR_CREDENTIAL 环境变量被正确传递给子进程
    ///
    /// 用 `printenv ACTR_CREDENTIAL` 捕获输出，验证 base64 编码的 credential 正确传递。
    #[tokio::test]
    async fn credential_env_is_passed_to_child() {
        let credential = b"test-credential-bytes".to_vec();
        let expected_b64 = base64::engine::general_purpose::STANDARD.encode(&credential);

        // 将 ACTR_CREDENTIAL 输出到临时文件
        let tmp = tempfile::NamedTempFile::new().expect("创建临时文件");
        let tmp_path = tmp.path().to_str().unwrap().to_string();

        // sh -c 'printf "%s" "$ACTR_CREDENTIAL" > /tmp/xxx'
        let script = format!("printf '%s' \"$ACTR_CREDENTIAL\" > {tmp_path}");

        let config = SpawnConfig {
            binary_path: PathBuf::from("/bin/sh"),
            args: vec!["-c".to_string(), script],
            credential,
            extra_env: HashMap::new(),
            restart_policy: RestartPolicy::Never,
            actr_type: "test:credential".to_string(),
        };

        let mut handle = spawn(config).await.expect("spawn 应成功");
        let state = handle.wait().await.expect("wait 应成功");
        assert_eq!(
            state,
            crate::runtime::handle::ChildProcessState::Exited(0),
            "sh 脚本应成功退出"
        );

        // 读取输出，验证 credential 被正确传递
        let actual = std::fs::read_to_string(&tmp_path).expect("读取临时文件");
        assert_eq!(
            actual.trim(),
            expected_b64.trim(),
            "ACTR_CREDENTIAL 环境变量应为 base64 编码的 credential"
        );
    }

    /// 测试额外环境变量被正确传递
    #[tokio::test]
    async fn extra_env_is_passed_to_child() {
        let tmp = tempfile::NamedTempFile::new().expect("创建临时文件");
        let tmp_path = tmp.path().to_str().unwrap().to_string();

        let script = format!("printf '%s' \"$MY_EXTRA_VAR\" > {tmp_path}");

        let mut extra_env = HashMap::new();
        extra_env.insert("MY_EXTRA_VAR".to_string(), "hello-extra".to_string());

        let config = SpawnConfig {
            binary_path: PathBuf::from("/bin/sh"),
            args: vec!["-c".to_string(), script],
            credential: vec![],
            extra_env,
            restart_policy: RestartPolicy::Never,
            actr_type: "test:extra-env".to_string(),
        };

        let mut handle = spawn(config).await.expect("spawn 应成功");
        handle.wait().await.expect("wait 应成功");

        let actual = std::fs::read_to_string(&tmp_path).expect("读取临时文件");
        assert_eq!(actual, "hello-extra", "额外环境变量应被正确传递");
    }

    /// 测试 kill() 能终止正在运行的子进程
    #[tokio::test]
    async fn kill_terminates_running_process() {
        // sleep 100 秒，不会自然退出
        let mut cmd = tokio::process::Command::new("/bin/sleep");
        cmd.arg("100");
        let child = cmd.spawn().expect("spawn sleep 应成功");
        let pid = child.id().expect("应有 pid");

        let mut handle = ChildProcessHandle::from_child(pid, "test:kill", child);
        assert!(handle.is_running(), "spawn 后应为 Running 状态");

        // kill 应在 10 秒内完成（SIGTERM 最多等 5 秒，超时 SIGKILL）
        tokio::time::timeout(Duration::from_secs(10), handle.kill())
            .await
            .expect("kill 不应超时")
            .expect("kill 应成功");

        // 状态应更新为非 Running
        assert!(!handle.is_running(), "kill 后进程应不再运行");
    }

    /// 测试 try_check_alive() 对已退出进程返回 false
    #[tokio::test]
    async fn try_check_alive_returns_false_after_exit() {
        use crate::runtime::handle::ChildProcessState;

        let mut cmd = tokio::process::Command::new("/usr/bin/true");
        let child = cmd.spawn().expect("spawn 应成功");
        let pid = child.id().expect("应有 pid");

        let mut handle = ChildProcessHandle::from_child(pid, "test:alive", child);

        // 等待进程自然退出后 try_wait 能感知到
        tokio::time::sleep(Duration::from_millis(300)).await;

        let alive = handle.try_check_alive();
        assert!(!alive, "进程退出后 try_check_alive 应返回 false");
        assert!(
            matches!(handle.state, ChildProcessState::Exited(0)),
            "退出状态应为 Exited(0)，实际: {:?}",
            handle.state
        );
    }
}
