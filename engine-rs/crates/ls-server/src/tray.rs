//! UX4（2026-11-20）：系统托盘——唯一常驻 UI（无黑框启动后的可发现入口）。
//!
//! **本模块是全仓唯二允许 `unsafe_code` 的模块之一**（workspace lint=deny +
//! 模块级 allow 收口）。FFI 清单（win32，windows-sys 0.61）：托盘线程的
//! Win32 消息泵 `GetMessageW` / `TranslateMessageW` / `DispatchMessageW`——
//! tray-icon 在创建线程要求泵消息（其回调经该窗口分发）。
//!
//! 选型纪律（roadmap 4.1）：tray-icon（tauri 团队、纯 win32）+ muda 菜单，
//! **不引入 Tauri/Electron/winit 运行时**；图标内嵌
//! `web/icons/icon-192.png`（品牌图复用），PNG→RGBA 用 png crate 解码。
//!
//! 交互（roadmap 4.2）：双击 = 打开控制台；菜单 = 打开控制台 / 复制访问地址 /
//! 退出（= 优雅关停，与网页「停止服务」同一函数，不走 HTTP 自环）。
//! 降级：创建失败（无 shell 会话等）→ 日志 warn，服务照常运行。

#![allow(unsafe_code)]

use crate::state::Shared;

static SHUTTING_DOWN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// UX4 / LS-044：优雅关停——防重入 → 停 workers → 停后台 jobs → 停 MQTT → WAL checkpoint → exit(0)。
/// 网页「停止服务」与托盘「退出」共用（同一函数，不走 HTTP 自环）。
pub fn graceful_shutdown(st: &Shared, source: &str) {
    if SHUTTING_DOWN.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    let stopped = crate::worker::stop_all_workers(st);
    st.jobs.stop();
    if let Some(bus) = &st.mqtt {
        bus.stop();
    }
    st.go2rtc.stop(); // LS-051：停 go2rtc（Job Object 兜底杀残留）
    if let Ok(s) = ls_store::Store::open(&st.db_path) {
        let _ = s.checkpoint_wal();
    }
    crate::logger::info(&format!(
        "[shutdown] {source}：已停 {stopped} 路分析 worker 与后台任务，WAL 已合并——进程退出"
    ));
    std::process::exit(0);
}

/// 启动托盘（独立消息泵线程）。返回 false = 创建失败（降级：仅日志）。
pub fn start(st: Shared, port: u16) -> bool {
    let spawned = std::thread::Builder::new()
        .name("ls-tray".into())
        .spawn(move || tray_run(st, port));
    match spawned {
        Ok(_) => true,
        Err(e) => {
            crate::logger::warn(&format!("[tray] 托盘线程创建失败（降级运行）: {e}"));
            false
        }
    }
}

#[cfg(windows)]
mod win_impl {
    use std::sync::Arc;

    use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
    use tray_icon::{TrayIcon, TrayIconBuilder, TrayIconEvent};

    use crate::state::Shared;

    /// 托盘菜单项 id（MenuEvent.id() 比对用）
    pub(super) const ID_OPEN: &str = "open";
    pub(super) const ID_COPY: &str = "copy";
    pub(super) const ID_EXIT: &str = "exit";

    /// PNG（icon-192）→ RGBA，供 Icon::from_rgba。
    pub(super) fn load_icon_rgba() -> Result<(Vec<u8>, u32, u32), String> {
        const PNG: &[u8] = include_bytes!("../../../../web/public/icons/icon-192.png");
        let decoder = png::Decoder::new(std::io::Cursor::new(PNG));
        let mut reader = decoder
            .read_info()
            .map_err(|e| format!("图标 PNG 解析失败: {e}"))?;
        let mut buf = vec![0u8; reader.output_buffer_size().unwrap_or(0)];
        let info = reader
            .next_frame(&mut buf)
            .map_err(|e| format!("图标 PNG 帧读取失败: {e}"))?;
        Ok((buf[..info.buffer_size()].to_vec(), info.width, info.height))
    }

    pub(super) fn tray_run(st: Shared, port: u16) {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            DispatchMessageW, GetMessageW, TranslateMessage, MSG,
        };
        let icon = match load_icon_rgba() {
            Ok((rgba, w, h)) => tray_icon::Icon::from_rgba(rgba, w, h).ok(),
            Err(e) => {
                crate::logger::warn(&format!("[tray] 图标加载失败（无图标托盘）: {e}"));
                None
            }
        };
        let url = format!("http://127.0.0.1:{port}/");
        let menu = Menu::new();
        let _ = menu.append(&MenuItem::with_id(ID_OPEN, "打开控制台", true, None));
        let _ = menu.append(&MenuItem::with_id(ID_COPY, "复制访问地址", true, None));
        let _ = menu.append(&PredefinedMenuItem::separator());
        let _ = menu.append(&MenuItem::with_id(ID_EXIT, "退出 LabSentinel", true, None));
        let mut builder = TrayIconBuilder::new()
            .with_id("labsentinel-tray")
            .with_tooltip(format!("LabSentinel · {url}"))
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false); // 左键留给双击开控制台
        if let Some(icon) = icon {
            builder = builder.with_icon(icon);
        }
        let tray: TrayIcon = match builder.build() {
            Ok(t) => t,
            Err(e) => {
                crate::logger::warn(&format!("[tray] 托盘创建失败（降级运行）: {e}"));
                return;
            }
        };
        let _ = tray.set_visible(true);
        crate::logger::info(&format!("[tray] 托盘就绪（双击打开 {url}）"));

        let menu_rx = MenuEvent::receiver();
        let tray_rx = TrayIconEvent::receiver();
        // Win32 消息泵：tray-icon 的隐藏窗口在本线程，事件回调经泵分发到 channel
        let mut msg: MSG = unsafe { std::mem::zeroed() };
        loop {
            // >0 = 有消息；0 = WM_QUIT；<0 = 错误（托盘失效 → 降级退出线程）
            let r = unsafe { GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) };
            if r <= 0 {
                crate::logger::warn("[tray] 消息循环退出（托盘降级）");
                return;
            }
            unsafe {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            while let Ok(ev) = menu_rx.try_recv() {
                match ev.id().as_ref() {
                    ID_OPEN => crate::open_browser(port),
                    ID_COPY => copy_url(&url),
                    ID_EXIT => super::graceful_shutdown(&Arc::clone(&st), "托盘退出"),
                    _ => {}
                }
            }
            while let Ok(ev) = tray_rx.try_recv() {
                if let TrayIconEvent::DoubleClick {
                    button: tray_icon::MouseButton::Left,
                    ..
                } = ev
                {
                    crate::open_browser(port);
                }
            }
        }
    }

    /// 复制访问地址：Set-Clipboard（无额外 FFI；CREATE_NO_WINDOW 防闪框）。
    pub(super) fn copy_url(url: &str) {
        use std::os::windows::process::CommandExt;
        let _ = std::process::Command::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                &format!("Set-Clipboard -Value '{}'", url),
            ])
            .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
            .spawn();
    }
}

#[cfg(windows)]
fn tray_run(st: Shared, port: u16) {
    win_impl::tray_run(st, port);
}

#[cfg(not(windows))]
fn tray_run(_st: Shared, _port: u16) {
    // 托盘为 Windows 优先路线（roadmap 非目标：不做 macOS/Linux 托盘）
}
