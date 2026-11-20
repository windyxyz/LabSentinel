//! UX3（2026-11-20）：GUI 子系统的控制台与致命错误可见性。
//!
//! **本模块是全仓唯二允许 `unsafe_code` 的模块之一**（workspace lint=deny +
//! 模块级 allow 收口）。FFI 清单（全部 win32，windows-sys 0.61）：
//! - `AttachConsoleW` / `GetStdHandle` / `SetStdHandle` / `CreateFileW`——
//!   `--console` 显式附着父控制台（命令行排障通道；GUI 双击启动不调用）；
//! - `GetConsoleWindow`——探测当前是否已有控制台（CI 管道/安装器自检路径
//!   保持原生输出）；
//! - `MessageBoxW`——GUI 模式下致命启动错误的可见出口（用户没有控制台可看，
//!   沿用便携启动链排障的教训：无窗口 ≠ 无反馈）。
//!
//! 管道语义已实测（UX3.0 spike，2026-11-20）：GUI 子系统进程在父进程管道
//! 重定向时 stdout 照常捕获、exit code 照常传递——CI 冒烟/smoke_rust/安装器
//! `--selftest-http` 不受 `windows_subsystem="windows"` 影响；无句柄启动时
//! Rust std 回落 NUL 设备，`println!` 不 panic。

#![allow(unsafe_code)]

#[cfg(windows)]
pub fn is_gui_subsystem() -> bool {
    // release 构建 = windows_subsystem="windows"；debug 保留控制台便于开发
    cfg!(not(debug_assertions))
}

// 非 Windows 无 GUI 子系统概念：is_gui_subsystem 不提供（仅 Windows 分支内部
// 使用；恒 false 版本会在 linux CI 触发 dead_code -D warnings）。

/// 当前进程是否已附着控制台（CI 管道/安装器启动时 stdout 已重定向，
/// GetConsoleWindow 可能仍为 NULL——此探测只用于 --console 的幂等判定）。
#[cfg(windows)]
fn has_console_window() -> bool {
    use windows_sys::Win32::System::Console::GetConsoleWindow;
    unsafe { !GetConsoleWindow().is_null() }
}

/// `--console`：release 下附着父进程控制台并把 std 句柄重指 CONOUT$。
/// 返回是否成功（失败只影响排障输出，不影响服务）。
#[cfg(windows)]
pub fn attach_parent_console() -> bool {
    use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
    use windows_sys::Win32::Storage::FileSystem::{CreateFileW, FILE_SHARE_WRITE, OPEN_EXISTING};
    // windows-sys 0.61 起去 W 后缀：AttachConsole（非 AttachConsoleW）
    use windows_sys::Win32::System::Console::{
        AttachConsole, SetStdHandle, ATTACH_PARENT_PROCESS, STD_ERROR_HANDLE, STD_OUTPUT_HANDLE,
    };
    unsafe {
        if has_console_window() {
            return true; // 已有控制台（管道启动/重复调用）——幂等
        }
        if AttachConsole(ATTACH_PARENT_PROCESS) == 0 {
            return false;
        }
        // 重开 CONOUT$ 作为 std 句柄（Rust std 惰性取句柄——必须早于首个 println!）
        let name: Vec<u16> = "CONOUT$\0".encode_utf16().collect();
        let conout = CreateFileW(
            name.as_ptr(),
            0x4000_0000u32, // GENERIC_WRITE（windows-sys 0.61 无该裸常量，FILE_GENERIC_WRITE 是复合位）
            FILE_SHARE_WRITE,
            std::ptr::null(),
            OPEN_EXISTING,
            0,
            std::ptr::null_mut(),
        );
        if conout != INVALID_HANDLE_VALUE && !conout.is_null() {
            SetStdHandle(STD_OUTPUT_HANDLE, conout);
            SetStdHandle(STD_ERROR_HANDLE, conout);
            true
        } else {
            false
        }
    }
}

#[cfg(not(windows))]
pub fn attach_parent_console() -> bool {
    false
}

/// 致命启动错误的可见出口：GUI 模式弹原生 MessageBox（阻塞直至用户确认）；
/// 有控制台时只写 stderr。宽字符转换本地完成（无终端代码页参与）。
#[cfg(windows)]
pub fn fatal_error_box(title: &str, msg: &str) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        MessageBoxW, MB_ICONERROR, MB_OK, MB_SETFOREGROUND, MB_TOPMOST,
    };
    let wide = |s: &str| -> Vec<u16> { s.encode_utf16().chain(std::iter::once(0)).collect() };
    if is_gui_subsystem() {
        unsafe {
            MessageBoxW(
                std::ptr::null_mut(),
                wide(msg).as_ptr(),
                wide(title).as_ptr(),
                MB_ICONERROR | MB_OK | MB_TOPMOST | MB_SETFOREGROUND,
            );
        }
    } else {
        eprintln!("[{title}] {msg}");
    }
}

#[cfg(not(windows))]
pub fn fatal_error_box(title: &str, msg: &str) {
    eprintln!("[{title}] {msg}");
}
