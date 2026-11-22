//! UX 轮追加（2026-11-21，用户实测故障根治）：ffmpeg 孤儿进程治理——
//! Windows Job Object（kill-on-close）。
//!
//! 故障史：server 被任务管理器/taskkill 强杀时，其 ffmpeg 子进程**不随之死**
//! （taskkill /IM 只匹配 server 进程名；/T 杀树要调用方记得用）→ 孤儿 ffmpeg
//! 独占 dshow 摄像头 → 后续所有实例"摄像头被占用"卡 starting（本次实测踩坑）。
//!
//! 语义：每个 Ingest 持一个 Job 句柄（`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`），
//! 所有重连的 ffmpeg 子进程都 assign 进去——**server 进程以任何方式死亡**
//! （正常退出/崩溃/taskkill/任务管理器）时其句柄表销毁 → job 最后一个句柄
//! 关闭 → job 内全部 ffmpeg 被 Windows 内核杀死。无需看门狗线程，零泄漏面。
//!
//! **本模块是 ls-ingest 唯一允许 `unsafe_code` 的模块**（workspace lint=deny +
//! 模块级 allow；与 ls-server 的 wincon.rs/tray.rs 同收口模式）。FFI 清单
//! （windows-sys 0.61）：`CreateJobObjectW` / `SetInformationJobObject`
//! （JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE）/ `AssignProcessToJobObject` /
//! `CloseHandle`。

#![allow(unsafe_code)]

/// Job 句柄封装：Drop 关句柄 → 内核杀 job 内全部子进程。
#[cfg(windows)]
pub struct JobHandle {
    // HANDLE 是进程级的句柄值，可由任意线程关闭。
    handle: isize,
}

#[cfg(windows)]
impl JobHandle {
    /// 创建 kill-on-close job（失败返回 None——降级为旧行为，仅日志可查）。
    pub fn new() -> Option<Self> {
        use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
        use windows_sys::Win32::System::JobObjects::{
            CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        };
        unsafe {
            let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if handle.is_null() || handle == INVALID_HANDLE_VALUE {
                return None;
            }
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let ok = SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const core::ffi::c_void,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );
            if ok == 0 {
                windows_sys::Win32::Foundation::CloseHandle(handle);
                return None;
            }
            Some(JobHandle {
                handle: handle as isize,
            })
        }
    }

    /// 把子进程 assign 进 job（spawn 后立刻调用；子进程已退出时返回 false——
    /// 无害，job 语义不受影响）。
    pub fn assign_child(&self, child: &std::process::Child) -> bool {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::System::JobObjects::AssignProcessToJobObject;
        unsafe { AssignProcessToJobObject(self.handle as _, child.as_raw_handle() as _) != 0 }
    }
}

#[cfg(windows)]
impl Drop for JobHandle {
    fn drop(&mut self) {
        unsafe {
            // 最后一个句柄关闭 → JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE 触发，
            // job 内所有进程被内核终止（server 死亡/Ingest drop 两路都到这）。
            windows_sys::Win32::Foundation::CloseHandle(self.handle as _);
        }
    }
}

// 非 Windows：无 job 概念（POSIX 侧可用 process group——当前 Windows 优先路线
// 暂不做；server 正常退出路径已有显式 kill_slot）。
#[cfg(not(windows))]
pub struct JobHandle;

#[cfg(not(windows))]
impl JobHandle {
    pub fn new() -> Option<Self> {
        None
    }

    pub fn assign_child(&self, _child: &std::process::Child) -> bool {
        false
    }
}
