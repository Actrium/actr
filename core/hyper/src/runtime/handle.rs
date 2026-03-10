use async_trait::async_trait;
use tokio::sync::Mutex;
use tracing::{error, warn};

use crate::error::{HyperError, HyperResult};

/// ActrSystem 句柄 trait（Mode 1 / Mode 3）
///
/// Hyper 通过此接口管理同进程内的 ActrSystem 生命周期。
/// Mode 1（Native）：ActrSystem+Workload 编译进同一 binary，直接实现此 trait。
/// Mode 3（WASM）：ActrSystem native shell 包装 WASM 实例后实现此 trait。
#[async_trait]
pub trait ActrSystemHandle: Send + Sync {
    /// 启动 ActrSystem
    async fn start(&self) -> HyperResult<()>;

    /// 优雅关闭 ActrSystem，等待 in-flight 消息处理完成
    async fn shutdown(&self) -> HyperResult<()>;

    /// 是否健康（用于 Hyper 侧监控）
    fn is_healthy(&self) -> bool;

    /// ActrSystem 唯一标识（调试用）
    fn id(&self) -> &str;
}

/// Mode 2（Process）子进程句柄
///
/// Hyper 通过此句柄管理子进程的生命周期：spawn、health check、restart policy。
/// 子进程内的 ActrSystem 直连 signaling，消息流量不经过 Hyper。
pub struct ChildProcessHandle {
    /// 子进程 PID
    pub pid: u32,
    /// 子进程对应的 ActrType（调试/日志用）
    pub actr_type: String,
    /// 子进程状态
    pub state: ChildProcessState,
    /// tokio 子进程句柄（持有所有权用于 wait/kill）
    pub(crate) child: Option<Mutex<tokio::process::Child>>,
}

impl std::fmt::Debug for ChildProcessHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChildProcessHandle")
            .field("pid", &self.pid)
            .field("actr_type", &self.actr_type)
            .field("state", &self.state)
            .field("child", &self.child.as_ref().map(|_| "<Child>"))
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChildProcessState {
    /// 正在运行
    Running,
    /// 已退出，exit code
    Exited(i32),
    /// 异常终止（信号等，无法获取退出码）
    Crashed,
}

impl ChildProcessHandle {
    /// 仅用于测试或不需要真实子进程句柄的场景
    pub fn new(pid: u32, actr_type: impl Into<String>) -> Self {
        Self {
            pid,
            actr_type: actr_type.into(),
            state: ChildProcessState::Running,
            child: None,
        }
    }

    /// 从真实 tokio Child 构造
    pub fn from_child(
        pid: u32,
        actr_type: impl Into<String>,
        child: tokio::process::Child,
    ) -> Self {
        Self {
            pid,
            actr_type: actr_type.into(),
            state: ChildProcessState::Running,
            child: Some(Mutex::new(child)),
        }
    }

    pub fn is_running(&self) -> bool {
        self.state == ChildProcessState::Running
    }

    /// 等待子进程退出，返回最终状态
    ///
    /// 若已无 child 句柄（已被消费），直接返回当前 state。
    pub async fn wait(&mut self) -> HyperResult<ChildProcessState> {
        let Some(child_mutex) = &self.child else {
            return Ok(self.state.clone());
        };

        let mut child = child_mutex.lock().await;
        match child.wait().await {
            Ok(status) => {
                let state = if let Some(code) = status.code() {
                    ChildProcessState::Exited(code)
                } else {
                    // 被信号终止，无退出码
                    ChildProcessState::Crashed
                };
                self.state = state.clone();
                Ok(state)
            }
            Err(e) => {
                error!(
                    pid = self.pid,
                    actr_type = %self.actr_type,
                    error = %e,
                    "等待子进程退出时发生错误"
                );
                self.state = ChildProcessState::Crashed;
                Err(HyperError::Runtime(format!("wait() 失败: {e}")))
            }
        }
    }

    /// 终止子进程：先发 SIGTERM，等待最多 5 秒，超时后发 SIGKILL
    pub async fn kill(&mut self) -> HyperResult<()> {
        let Some(child_mutex) = &self.child else {
            // 已无句柄，视为已终止
            return Ok(());
        };

        let mut child = child_mutex.lock().await;

        // 发送 SIGTERM（Unix）/ TerminateProcess（Windows）
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;
            // 向子进程发 SIGTERM
            if let Err(e) = nix_kill(self.pid, libc_sigterm()) {
                warn!(
                    pid = self.pid,
                    actr_type = %self.actr_type,
                    error = %e,
                    "发送 SIGTERM 失败，直接 SIGKILL"
                );
                let _ = child.kill().await;
                self.state = ChildProcessState::Crashed;
                return Ok(());
            }

            warn!(
                pid = self.pid,
                actr_type = %self.actr_type,
                "已发送 SIGTERM，等待子进程优雅退出（最多 5 秒）"
            );

            // 等待最多 5 秒
            match tokio::time::timeout(
                std::time::Duration::from_secs(5),
                child.wait(),
            )
            .await
            {
                Ok(Ok(status)) => {
                    let code = status.code()
                        .or_else(|| status.signal().map(|s| -s));
                    self.state = match code {
                        Some(0) => ChildProcessState::Exited(0),
                        Some(c) => ChildProcessState::Exited(c),
                        None => ChildProcessState::Crashed,
                    };
                    warn!(
                        pid = self.pid,
                        actr_type = %self.actr_type,
                        state = ?self.state,
                        "子进程在 SIGTERM 后已退出"
                    );
                    return Ok(());
                }
                Ok(Err(e)) => {
                    error!(
                        pid = self.pid,
                        error = %e,
                        "等待子进程退出时出错，升级为 SIGKILL"
                    );
                }
                Err(_timeout) => {
                    warn!(
                        pid = self.pid,
                        actr_type = %self.actr_type,
                        "SIGTERM 后 5 秒未退出，升级为 SIGKILL"
                    );
                }
            }

            // SIGTERM 超时，发 SIGKILL
            if let Err(e) = child.kill().await {
                error!(
                    pid = self.pid,
                    error = %e,
                    "SIGKILL 失败"
                );
            }
            let _ = child.wait().await;
            self.state = ChildProcessState::Crashed;
        }

        #[cfg(not(unix))]
        {
            warn!(
                pid = self.pid,
                actr_type = %self.actr_type,
                "非 Unix 平台，直接 kill 子进程"
            );
            if let Err(e) = child.kill().await {
                error!(pid = self.pid, error = %e, "kill 子进程失败");
                return Err(HyperError::Runtime(format!("kill 失败: {e}")));
            }
            let _ = child.wait().await;
            self.state = ChildProcessState::Crashed;
        }

        Ok(())
    }

    /// 非阻塞检查进程是否还在运行
    ///
    /// 使用 `try_wait()` 轮询，不阻塞当前线程。
    pub fn try_check_alive(&mut self) -> bool {
        let Some(child_mutex) = &self.child else {
            return self.state == ChildProcessState::Running;
        };

        // try_lock 失败说明另一个任务正在 wait，保守返回 true
        let Ok(mut child) = child_mutex.try_lock() else {
            return true;
        };

        match child.try_wait() {
            Ok(None) => true, // 进程仍在运行
            Ok(Some(status)) => {
                self.state = if let Some(code) = status.code() {
                    ChildProcessState::Exited(code)
                } else {
                    ChildProcessState::Crashed
                };
                false
            }
            Err(e) => {
                error!(
                    pid = self.pid,
                    error = %e,
                    "try_wait() 出错，保守认为进程已退出"
                );
                self.state = ChildProcessState::Crashed;
                false
            }
        }
    }
}

/// Unix 平台：通过 libc 发送 SIGTERM
#[cfg(unix)]
fn nix_kill(pid: u32, sig: i32) -> Result<(), String> {
    // SAFETY: kill(2) 是标准 POSIX 系统调用
    let ret = unsafe { libc::kill(pid as i32, sig) };
    if ret == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error().to_string())
    }
}

#[cfg(unix)]
fn libc_sigterm() -> i32 {
    libc::SIGTERM
}

/// Mode 3（WASM）实例句柄
///
/// ActrSystem native shell 创建并持有此句柄。
/// 热更新时，旧实例卸载后创建新实例句柄。
#[derive(Debug)]
pub struct WasmInstanceHandle {
    /// WASM 实例唯一 ID（每次 load 生成）
    pub instance_id: String,
    /// 对应的 ActrType
    pub actr_type: String,
}

impl WasmInstanceHandle {
    pub fn new(instance_id: impl Into<String>, actr_type: impl Into<String>) -> Self {
        Self {
            instance_id: instance_id.into(),
            actr_type: actr_type.into(),
        }
    }
}
