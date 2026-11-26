//! LS-051：Rust 托管外部子进程的通用守护封装（go2rtc 等）。
//!
//! 设计要点：
//! - Windows 下给子进程绑 Windows Job Object（kill-on-close）——主进程正常退出 /
//!   崩溃 / taskkill 时内核自动杀掉 job 内进程，杜绝孤儿（复用 `winjob`，零新增 unsafe）；
//! - 所有 Windows 子进程加 `CREATE_NO_WINDOW`（0x0800_0000）——杜绝控制台黑框闪现；
//! - Drop 时先 kill 再 wait 兜底。

use std::process::{Child, Command, ExitStatus, Stdio};

/// 受守护的子进程。Drop 终止并回收子进程，然后关闭 Job 句柄。
pub struct ChildGuard {
    child: Option<Child>,
    #[cfg(windows)]
    _job: crate::winjob::JobHandle,
}

impl ChildGuard {
    /// 启动一个受守护的子进程：stdout/stderr 丢弃（后台服务），CREATE_NO_WINDOW + Job Object。
    pub fn spawn(cmd: Command) -> Result<Self, String> {
        Self::spawn_with_stdout(cmd, Stdio::null())
    }

    pub fn spawn_with_stdout(mut cmd: Command, stdout: Stdio) -> Result<Self, String> {
        cmd.stdout(stdout)
            .stderr(Stdio::null())
            .stdin(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            // 0x0800_0000 = CREATE_NO_WINDOW：所有 Windows 子进程必须加，杜绝黑框
            cmd.creation_flags(0x0800_0000);
            let job = crate::winjob::JobHandle::new().ok_or("无法创建子进程 Job Object")?;
            let mut child = cmd.spawn().map_err(|e| format!("子进程启动失败: {e}"))?;
            if !job.assign_child(&child) {
                let _ = child.kill();
                let _ = child.wait();
                return Err("无法将子进程加入 Job Object".into());
            }
            Ok(Self {
                child: Some(child),
                _job: job,
            })
        }
        #[cfg(not(windows))]
        {
            let child = cmd.spawn().map_err(|e| format!("子进程启动失败: {e}"))?;
            Ok(Self { child: Some(child) })
        }
    }

    /// 进程是否仍存活（不阻塞）。
    pub fn is_running(&mut self) -> bool {
        match self.child.as_mut() {
            Some(c) => matches!(c.try_wait(), Ok(None)),
            None => false,
        }
    }

    pub fn exit_status(&mut self) -> std::io::Result<Option<ExitStatus>> {
        self.child
            .as_mut()
            .ok_or_else(|| std::io::Error::other("child was stopped"))?
            .try_wait()
    }

    /// 终止并回收。
    pub fn stop(&mut self) {
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }

    /// 子进程 PID（供状态接口展示）。
    pub fn pid(&self) -> Option<u32> {
        self.child.as_ref().map(|c| c.id())
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawn_and_stop_a_short_lived_process() {
        #[cfg(windows)]
        let cmd = {
            let mut c = Command::new("cmd");
            c.args(["/c", "ping", "-n", "15", "127.0.0.1"]);
            c
        };
        #[cfg(not(windows))]
        let cmd = {
            let mut c = Command::new("sleep");
            c.arg("30");
            c
        };
        let mut guard = ChildGuard::spawn(cmd).expect("spawn ok");
        assert!(guard.pid().is_some());
        assert!(guard.is_running());
        guard.stop();
        assert!(!guard.is_running());
    }
}
