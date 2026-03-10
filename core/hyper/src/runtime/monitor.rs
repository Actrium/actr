//! Mode 2 子进程健康监控与重启逻辑
//!
//! 在后台任务中持续监控子进程状态，按 RestartPolicy 决定是否重新 spawn。
//! 收到 shutdown 信号时优雅退出并终止子进程。

use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use crate::error::HyperResult;

use super::handle::{ChildProcessHandle, ChildProcessState};
use super::spawn::{spawn, RestartPolicy, SpawnConfig};

/// 启动后台监控任务，按 restart policy 决策是否重启子进程
///
/// # 行为
///
/// - 等待进程退出（`handle.wait().await`）
/// - 按 RestartPolicy 决定是否重新 spawn
/// - 收到 shutdown 信号时，先 kill 当前子进程，再返回
/// - 重启之间不使用 sleep，直接再次 spawn 并进入下一轮监控循环
///
/// # 参数
///
/// - `handle`：初始子进程句柄
/// - `config`：spawn 配置（重启时复用）
/// - `shutdown`：外部取消令牌，触发时优雅退出
pub async fn monitor_process(
    mut handle: ChildProcessHandle,
    config: SpawnConfig,
    shutdown: CancellationToken,
) -> HyperResult<()> {
    let mut retries_used: u32 = 0;

    loop {
        // 同时等待：子进程退出 OR shutdown 信号
        let exit_state = tokio::select! {
            // 子进程退出
            result = handle.wait() => {
                match result {
                    Ok(state) => state,
                    Err(e) => {
                        error!(
                            pid = handle.pid,
                            actr_type = %handle.actr_type,
                            error = %e,
                            "等待子进程时出错，按崩溃处理"
                        );
                        ChildProcessState::Crashed
                    }
                }
            },
            // shutdown 信号
            _ = shutdown.cancelled() => {
                info!(
                    pid = handle.pid,
                    actr_type = %handle.actr_type,
                    "收到 shutdown 信号，正在终止子进程"
                );
                // 优雅终止子进程
                if let Err(e) = handle.kill().await {
                    error!(
                        pid = handle.pid,
                        error = %e,
                        "shutdown 时终止子进程失败"
                    );
                }
                info!(
                    actr_type = %handle.actr_type,
                    "监控任务已优雅退出"
                );
                return Ok(());
            }
        };

        // 记录进程退出原因
        match &exit_state {
            ChildProcessState::Exited(0) => {
                info!(
                    actr_type = %handle.actr_type,
                    pid = handle.pid,
                    "子进程正常退出（exit code 0）"
                );
            }
            ChildProcessState::Exited(code) => {
                error!(
                    actr_type = %handle.actr_type,
                    pid = handle.pid,
                    exit_code = code,
                    "子进程以非零退出码退出"
                );
            }
            ChildProcessState::Crashed => {
                error!(
                    actr_type = %handle.actr_type,
                    pid = handle.pid,
                    "子进程异常终止（信号或未知原因）"
                );
            }
            ChildProcessState::Running => {
                // wait() 返回时不应为 Running，防御性处理
                warn!(
                    actr_type = %handle.actr_type,
                    "wait() 返回了 Running 状态，跳过本轮"
                );
                continue;
            }
        }

        // 如果 shutdown 已触发，不再重启
        if shutdown.is_cancelled() {
            info!(
                actr_type = %handle.actr_type,
                "shutdown 已触发，不再重启子进程"
            );
            return Ok(());
        }

        // 按 RestartPolicy 决定是否重启
        let should_restart = match &config.restart_policy {
            RestartPolicy::Never => {
                info!(
                    actr_type = %handle.actr_type,
                    "RestartPolicy::Never，不重启"
                );
                false
            }
            RestartPolicy::OnFailure { max_retries } => {
                let failed = !matches!(exit_state, ChildProcessState::Exited(0));
                if !failed {
                    info!(
                        actr_type = %handle.actr_type,
                        "子进程正常退出，OnFailure 策略不重启"
                    );
                    false
                } else if retries_used >= *max_retries {
                    error!(
                        actr_type = %handle.actr_type,
                        retries_used,
                        max_retries,
                        "已达最大重试次数，停止重启"
                    );
                    false
                } else {
                    true
                }
            }
            RestartPolicy::Always { max_retries } => {
                if retries_used >= *max_retries {
                    error!(
                        actr_type = %handle.actr_type,
                        retries_used,
                        max_retries,
                        "已达最大重试次数，停止重启"
                    );
                    false
                } else {
                    true
                }
            }
        };

        if !should_restart {
            info!(
                actr_type = %handle.actr_type,
                "监控任务结束，不再重启"
            );
            return Ok(());
        }

        retries_used += 1;
        warn!(
            actr_type = %handle.actr_type,
            retries_used,
            "正在重启子进程（第 {retries_used} 次）"
        );

        // 重新 spawn，不使用 sleep——直接再次进入下一轮循环
        handle = match spawn(SpawnConfig {
            binary_path: config.binary_path.clone(),
            args: config.args.clone(),
            credential: config.credential.clone(),
            extra_env: config.extra_env.clone(),
            restart_policy: config.restart_policy.clone(),
            actr_type: config.actr_type.clone(),
        })
        .await
        {
            Ok(h) => {
                warn!(
                    pid = h.pid,
                    actr_type = %config.actr_type,
                    retries_used,
                    "子进程已重启"
                );
                h
            }
            Err(e) => {
                error!(
                    actr_type = %config.actr_type,
                    error = %e,
                    "重启子进程失败，监控任务退出"
                );
                return Err(e);
            }
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::spawn::{spawn, RestartPolicy, SpawnConfig};
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::time::Duration;
    use tokio_util::sync::CancellationToken;

    fn basic_config(binary: &str, args: Vec<&str>, policy: RestartPolicy) -> SpawnConfig {
        SpawnConfig {
            binary_path: PathBuf::from(binary),
            args: args.into_iter().map(|s| s.to_string()).collect(),
            credential: vec![],
            extra_env: HashMap::new(),
            restart_policy: policy,
            actr_type: "test:monitor".to_string(),
        }
    }

    /// 测试 RestartPolicy::Never：进程退出后不重启，monitor 正常结束
    #[tokio::test]
    async fn never_policy_does_not_restart() {
        let config = basic_config("/usr/bin/true", vec![], RestartPolicy::Never);
        let handle = spawn(SpawnConfig {
            binary_path: config.binary_path.clone(),
            args: config.args.clone(),
            credential: config.credential.clone(),
            extra_env: config.extra_env.clone(),
            restart_policy: config.restart_policy.clone(),
            actr_type: config.actr_type.clone(),
        })
        .await
        .expect("spawn 应成功");

        let shutdown = CancellationToken::new();
        // monitor_process 应在 true 退出后立即返回
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            monitor_process(handle, config, shutdown),
        )
        .await
        .expect("monitor 应在 5 秒内结束");

        assert!(result.is_ok(), "Never 策略下 monitor 应正常结束");
    }

    /// 测试 shutdown 信号能终止监控中的长时运行子进程
    #[tokio::test]
    async fn shutdown_cancels_monitoring() {
        // sleep 100 秒，不会自然退出
        let config = basic_config("/bin/sleep", vec!["100"], RestartPolicy::Never);
        let handle = spawn(SpawnConfig {
            binary_path: config.binary_path.clone(),
            args: config.args.clone(),
            credential: config.credential.clone(),
            extra_env: config.extra_env.clone(),
            restart_policy: config.restart_policy.clone(),
            actr_type: config.actr_type.clone(),
        })
        .await
        .expect("spawn sleep 应成功");

        let shutdown = CancellationToken::new();
        let shutdown_clone = shutdown.clone();

        // 0.5 秒后触发 shutdown
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(500)).await;
            shutdown_clone.cancel();
        });

        let result = tokio::time::timeout(
            Duration::from_secs(10),
            monitor_process(handle, config, shutdown),
        )
        .await
        .expect("shutdown 后 monitor 应在 10 秒内结束");

        assert!(result.is_ok(), "shutdown 后 monitor 应正常退出");
    }

    /// 测试 OnFailure 策略：失败后重启，但达到上限后停止
    #[tokio::test]
    async fn on_failure_retries_and_stops() {
        // false 命令退出码为 1（失败）
        let config = basic_config("/usr/bin/false", vec![], RestartPolicy::OnFailure { max_retries: 2 });
        let handle = spawn(SpawnConfig {
            binary_path: config.binary_path.clone(),
            args: config.args.clone(),
            credential: config.credential.clone(),
            extra_env: config.extra_env.clone(),
            restart_policy: config.restart_policy.clone(),
            actr_type: config.actr_type.clone(),
        })
        .await
        .expect("spawn 应成功");

        let shutdown = CancellationToken::new();
        // 最多重试 2 次，total spawn = 3（初始 + 2 次重启）
        // 每次 false 立即退出，整体应在 5 秒内完成
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            monitor_process(handle, config, shutdown),
        )
        .await
        .expect("monitor 应在 5 秒内结束");

        assert!(result.is_ok(), "达到最大重试次数后应正常退出");
    }
}
