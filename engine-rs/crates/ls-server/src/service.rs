//! Native Windows Service Control Manager integration for the all-users installation.

use std::ffi::OsString;
use std::fs;
use std::io;
use std::os::windows::{fs::MetadataExt, process::CommandExt};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use tokio::sync::watch;
use windows_service::service::{
    Service, ServiceAccess, ServiceAction, ServiceActionType, ServiceControl, ServiceControlAccept,
    ServiceErrorControl, ServiceExitCode, ServiceFailureActions, ServiceFailureResetPeriod,
    ServiceInfo, ServiceStartType, ServiceState, ServiceStatus, ServiceType,
};
use windows_service::service_control_handler::{
    self, ServiceControlHandlerResult, ServiceStatusHandle,
};
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};
use windows_service::{define_windows_service, service_dispatcher, Error};

const NAME: &str = "LabSentinel";
const RUN_ARG: &str = "--service-run";
const ACCOUNT: &str = r"NT SERVICE\LabSentinel";
const STATUS_DIR: &str = "service-status";
const TIMEOUT: Duration = Duration::from_secs(40);
const CAMERA_PREFIX: &str = "service.camera.resume.";

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Command {
    Running,
    Restart,
    Stop,
}

static CONTROL: OnceLock<(ServiceStatusHandle, watch::Sender<Command>)> = OnceLock::new();
static CAMERA_INTENT: Mutex<()> = Mutex::new(());

define_windows_service!(ffi_service_main, service_main);

pub(crate) fn active() -> bool {
    CONTROL.get().is_some()
}

fn resumable(kind: &str) -> bool {
    matches!(kind, "rtsp" | "usb" | "folder")
}

pub(crate) fn camera_intent_guard() -> Option<MutexGuard<'static, ()>> {
    active().then(|| {
        CAMERA_INTENT
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    })
}

pub(crate) fn remember_camera(
    store: &ls_store::Store,
    id: i64,
    kind: &str,
    running: bool,
) -> rusqlite::Result<()> {
    if active() && resumable(kind) {
        let key = format!("{CAMERA_PREFIX}{id}");
        if running {
            store.setting_set(&key, "1")?;
        } else {
            store.settings_delete(&[key])?;
        }
    }
    Ok(())
}

pub(crate) fn resume_cameras(st: &crate::Shared) {
    let result = ls_store::Store::open(&st.db_path).and_then(|store| {
        for (key, value) in store.settings_by_prefix(CAMERA_PREFIX)? {
            let Some(id) = key
                .strip_prefix(CAMERA_PREFIX)
                .and_then(|id| id.parse().ok())
            else {
                continue;
            };
            if value != "1" {
                continue;
            }
            match crate::worker::WorkerConfig::from_db(&store, id) {
                Ok(Some(config)) if resumable(&config.spec.type_) => {
                    if let Err(error) = crate::worker::start_worker(st, config) {
                        crate::logger::warn(&format!("[service] cam-{id} 恢复失败: {error}"));
                    }
                }
                Err(error) => {
                    crate::logger::warn(&format!("[service] cam-{id} 配置读取失败: {error}"))
                }
                _ => {}
            }
        }
        Ok(())
    });
    if let Err(error) = result {
        crate::logger::error(&format!("[service] 读取待恢复机位失败: {error}"));
    }
}

pub(crate) fn request(command: Command) -> bool {
    CONTROL.get().is_some_and(|(_, sender)| {
        sender.send_if_modified(|current| {
            if *current == command || *current == Command::Stop {
                return false;
            }
            *current = command;
            true
        })
    })
}

fn status(
    handle: ServiceStatusHandle,
    state: ServiceState,
    code: u32,
) -> windows_service::Result<()> {
    let pending = matches!(
        state,
        ServiceState::StartPending | ServiceState::StopPending
    );
    handle.set_service_status(ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: state,
        controls_accepted: if state == ServiceState::Running {
            ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN
        } else {
            ServiceControlAccept::empty()
        },
        exit_code: if code == 0 {
            ServiceExitCode::Win32(0)
        } else {
            ServiceExitCode::ServiceSpecific(code)
        },
        checkpoint: u32::from(pending),
        wait_hint: if pending { TIMEOUT } else { Duration::ZERO },
        process_id: None,
    })
}

fn service_main(_: Vec<OsString>) {
    let Ok(handle) = service_control_handler::register(NAME, |event| match event {
        ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
        ServiceControl::Stop | ServiceControl::Shutdown => {
            request(Command::Stop);
            ServiceControlHandlerResult::NoError
        }
        _ => ServiceControlHandlerResult::NotImplemented,
    }) else {
        return;
    };
    let (tx, _) = watch::channel(Command::Running);
    if CONTROL.set((handle, tx)).is_err() || status(handle, ServiceState::StartPending, 0).is_err()
    {
        return;
    }
    let code = (|| -> io::Result<i32> {
        std::env::set_current_dir(install_root()?)?;
        let inherited = crate::envfile::NETWORK_KEYS.map(std::env::var_os);
        let (_, tx) = CONTROL.get().expect("service control handler");
        loop {
            let runtime = crate::server_runtime()?;
            let code = runtime.block_on(crate::run_server());
            runtime.shutdown_timeout(Duration::from_secs(2));
            if code != 0
                || !tx.send_if_modified(|current| {
                    if *current != Command::Restart {
                        return false;
                    }
                    *current = Command::Running;
                    true
                })
            {
                return Ok(code);
            }
            for (key, value) in crate::envfile::NETWORK_KEYS.into_iter().zip(&inherited) {
                match value {
                    Some(value) => std::env::set_var(key, value),
                    None => std::env::remove_var(key),
                }
            }
        }
    })()
    .unwrap_or_else(|error| {
        crate::logger::error(&format!("[service] {error}"));
        1
    });
    let _ = status(handle, ServiceState::Stopped, code as u32);
}

pub(crate) fn data_dir(root: &Path) -> PathBuf {
    root.join("service-data")
}

pub(crate) fn running(root: &Path, port: u16) -> windows_service::Result<()> {
    let (handle, _) = CONTROL.get().expect("service control handler");
    if let Err(error) = fs::write(root.join(STATUS_DIR).join("port"), port.to_string()) {
        crate::logger::warn(&format!("[service] 无法更新控制台快捷方式端口: {error}"));
    }
    status(*handle, ServiceState::Running, 0)
}

pub(crate) async fn wait_for_stop() {
    let (handle, sender) = CONTROL.get().expect("service control handler");
    if sender
        .subscribe()
        .wait_for(|cmd| *cmd != Command::Running)
        .await
        .is_ok_and(|cmd| *cmd == Command::Stop)
    {
        let _ = status(*handle, ServiceState::StopPending, 0);
    }
}

pub(crate) fn install_root() -> io::Result<PathBuf> {
    std::env::current_exe()?
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| io::Error::other("缺少 LabSentinel 安装目录"))
}

fn check_directory(dir: &Path) -> io::Result<()> {
    let info = fs::symlink_metadata(dir)?;
    if !info.is_dir() || info.file_attributes() & 0x400 != 0 {
        return Err(io::Error::other(format!(
            "服务路径须为普通目录: {}",
            dir.display()
        )));
    }
    Ok(())
}

fn check_install_root(root: &Path) -> io::Result<()> {
    let fixed_root = ["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432"]
        .into_iter()
        .filter_map(std::env::var_os)
        .any(|base| {
            Path::new(&base)
                .join(NAME)
                .to_string_lossy()
                .eq_ignore_ascii_case(&root.to_string_lossy())
        });
    if !fixed_root {
        return Err(io::Error::other(
            "原生服务须安装到默认 Program Files\\LabSentinel 目录",
        ));
    }
    check_directory(root)
}

fn provision_directory(root: &Path, readable: bool) -> io::Result<()> {
    match fs::create_dir(root) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    check_directory(root)?;
    let mut cmd = std::process::Command::new("icacls.exe");
    cmd.arg(root).args([
        "/inheritance:r",
        "/grant:r",
        "*S-1-5-18:(OI)(CI)F",
        "*S-1-5-32-544:(OI)(CI)F",
        r"NT SERVICE\LabSentinel:(OI)(CI)M",
    ]);
    if readable {
        cmd.arg("*S-1-5-32-545:(OI)(CI)RX");
    }
    if !cmd.creation_flags(0x0800_0000).output()?.status.success() {
        return Err(io::Error::other(format!(
            "设置服务目录权限失败: {}",
            root.display()
        )));
    }
    Ok(())
}

fn is_win32_error(error: &Error, code: i32) -> bool {
    matches!(error, Error::Winapi(err) if err.raw_os_error() == Some(code))
}

fn launch_path_matches(registered: &Path, exe: &Path) -> bool {
    let registered = registered.to_string_lossy();
    let expected = exe.to_string_lossy();
    registered
        .strip_suffix(" --service-run")
        .is_some_and(|path| path.trim_matches('"').eq_ignore_ascii_case(&expected))
}

fn open_owned(manager: &ServiceManager, access: ServiceAccess) -> Result<Option<Service>, String> {
    let handle = match manager.open_service(NAME, access | ServiceAccess::QUERY_CONFIG) {
        Ok(handle) => handle,
        Err(error) if is_win32_error(&error, 1060) => return Ok(None),
        Err(error) => return Err(format!("打开服务失败: {error:?}")),
    };
    let config = handle
        .query_config()
        .map_err(|error| format!("读取 SCM 配置失败: {error:?}"))?;
    let exe = std::env::current_exe().map_err(|error| error.to_string())?;
    if !launch_path_matches(&config.executable_path, &exe)
        || !config
            .account_name
            .is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case(ACCOUNT))
    {
        return Err("SCM 中已有同名服务，所属程序或服务账户不匹配".into());
    }
    Ok(Some(handle))
}

fn wait_for_state(service: &Service, target: ServiceState) -> Result<(), String> {
    let until = Instant::now() + TIMEOUT;
    loop {
        let current = service
            .query_status()
            .map_err(|err| format!("读取服务状态失败: {err:?}"))?;
        if current.current_state == target {
            return Ok(());
        }
        if current.current_state == ServiceState::Stopped || Instant::now() > until {
            return Err(format!(
                "服务未进入 {target:?}: {:?} ({:?})",
                current.current_state, current.exit_code
            ));
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

fn stop(service: &Service) -> Result<(), String> {
    let state = service
        .query_status()
        .map_err(|err| format!("读取服务状态失败: {err:?}"))?
        .current_state;
    if state == ServiceState::StartPending {
        let _ = wait_for_state(service, ServiceState::Running);
    }
    if !matches!(state, ServiceState::Stopped | ServiceState::StopPending) {
        service
            .stop()
            .or_else(|err| {
                if is_win32_error(&err, 1062) {
                    service.query_status()
                } else {
                    Err(err)
                }
            })
            .map_err(|err| format!("停止服务失败: {err:?}"))?;
    }
    wait_for_state(service, ServiceState::Stopped)
}

fn install(manager: &ServiceManager, root: &Path) -> Result<(), String> {
    check_install_root(root).map_err(|err| err.to_string())?;
    let access = ServiceAccess::QUERY_STATUS
        | ServiceAccess::START
        | ServiceAccess::STOP
        | ServiceAccess::CHANGE_CONFIG
        | ServiceAccess::DELETE;
    let (handle, created) = match open_owned(manager, access)? {
        Some(handle) => (handle, false),
        None => (
            manager
                .create_service(
                    &ServiceInfo {
                        name: NAME.into(),
                        display_name: NAME.into(),
                        service_type: ServiceType::OWN_PROCESS,
                        start_type: ServiceStartType::AutoStart,
                        error_control: ServiceErrorControl::Normal,
                        executable_path: std::env::current_exe().map_err(|err| err.to_string())?,
                        launch_arguments: vec![RUN_ARG.into()],
                        dependencies: Vec::new(),
                        account_name: Some(ACCOUNT.into()),
                        account_password: None,
                    },
                    access,
                )
                .map_err(|err| format!("注册服务失败: {err:?}"))?,
            true,
        ),
    };
    let configured = (|| -> Result<(), String> {
        let data = data_dir(root);
        provision_directory(&data, false).map_err(|err| err.to_string())?;
        provision_directory(&root.join(STATUS_DIR), true).map_err(|err| err.to_string())?;
        let network = crate::envfile::network_path(&data.to_string_lossy());
        let mut content = match fs::read_to_string(&network) {
            Ok(content) => content,
            Err(err) if err.kind() == io::ErrorKind::NotFound => String::new(),
            Err(err) => return Err(err.to_string()),
        };
        if crate::envfile::get_key(&content, "LS_ADMIN_TOKEN").is_none_or(|key| key.len() < 16) {
            content = crate::envfile::set_key(
                &content,
                "LS_ADMIN_TOKEN",
                &crate::envfile::random_token(),
            );
            crate::envfile::write_env(&network, &content).map_err(|err| err.to_string())?;
        }
        handle
            .set_delayed_auto_start(true)
            .and_then(|()| {
                handle.update_failure_actions(ServiceFailureActions {
                    reset_period: ServiceFailureResetPeriod::After(Duration::from_secs(120)),
                    reboot_msg: None,
                    command: None,
                    actions: Some(vec![
                        ServiceAction {
                            action_type: ServiceActionType::Restart,
                            delay: Duration::from_secs(2),
                        },
                        ServiceAction {
                            action_type: ServiceActionType::Restart,
                            delay: Duration::from_secs(10),
                        },
                        ServiceAction {
                            action_type: ServiceActionType::None,
                            delay: Duration::ZERO,
                        },
                    ]),
                })
            })
            .map_err(|err| format!("设置服务恢复策略失败: {err:?}"))?;
        if handle
            .query_status()
            .map_err(|err| format!("查询服务失败: {err:?}"))?
            .current_state
            == ServiceState::Stopped
        {
            handle
                .start::<&str>(&[])
                .map_err(|err| format!("启动服务失败: {err:?}"))?;
        }
        wait_for_state(&handle, ServiceState::Running)
    })();
    if configured.is_err() && created {
        let _ = stop(&handle);
        let _ = handle.delete();
    }
    configured
}

pub(crate) fn cli() -> Option<i32> {
    let arg = std::env::args()
        .skip(1)
        .find(|arg| arg.starts_with("--service-"))?;
    let result = cli_command(&arg);
    Some(match result {
        Ok(code) => code,
        Err(error) => {
            eprintln!("[service] {error}");
            1
        }
    })
}

fn cli_command(arg: &str) -> Result<i32, String> {
    if arg == RUN_ARG {
        service_dispatcher::start(NAME, ffi_service_main)
            .map_err(|err| format!("连接 Windows SCM 失败: {err:?}"))?;
        return Ok(0);
    }
    let root = install_root().map_err(|err| err.to_string())?;
    let mut access = ServiceManagerAccess::CONNECT;
    if arg == "--service-install" {
        access |= ServiceManagerAccess::CREATE_SERVICE;
    }
    let manager = ServiceManager::local_computer(None::<&str>, access)
        .map_err(|err| format!("连接 Windows SCM 失败: {err:?}"))?;
    match arg {
        "--service-install" => install(&manager, &root)?,
        "--service-open" | "--service-status" => {
            let Some(handle) = open_owned(&manager, ServiceAccess::QUERY_STATUS)? else {
                return Ok(3);
            };
            let status = handle
                .query_status()
                .map_err(|err| format!("读取服务状态失败: {err:?}"))?;
            if arg == "--service-status" {
                println!("{:?}", status.current_state);
            } else if status.current_state == ServiceState::Running {
                let port = fs::read_to_string(root.join(STATUS_DIR).join("port"))
                    .ok()
                    .and_then(|value| value.trim().parse().ok())
                    .unwrap_or(8000);
                crate::open_browser(port);
            } else {
                crate::wincon::fatal_error_box(
                    NAME,
                    "Windows 服务未运行。请在 Windows 服务管理器中启动 LabSentinel。",
                );
                return Ok(1);
            }
        }
        "--service-stop" | "--service-uninstall" => {
            let remove = arg == "--service-uninstall";
            let mut access = ServiceAccess::QUERY_STATUS | ServiceAccess::STOP;
            if remove {
                access |= ServiceAccess::DELETE;
            }
            if let Some(handle) = open_owned(&manager, access)? {
                stop(&handle)?;
                if remove {
                    handle
                        .delete()
                        .map_err(|err| format!("卸载服务失败: {err:?}"))?;
                }
            }
        }
        _ => return Err(format!("未知服务命令: {arg}")),
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_resume_excludes_offline_video_jobs() {
        assert!(!resumable("file"));
        for kind in ["rtsp", "usb", "folder"] {
            assert!(resumable(kind));
        }
    }

    #[test]
    fn service_only_controls_registration_owned_by_current_binary() {
        let exe = Path::new(r"C:\Program Files (x86)\LabSentinel\labsentinel-server.exe");
        assert!(launch_path_matches(
            Path::new(
                r#""C:\Program Files (x86)\LabSentinel\labsentinel-server.exe" --service-run"#
            ),
            exe
        ));
        assert!(!launch_path_matches(
            Path::new(
                r#""C:\Program Files (x86)\LabSentinel\labsentinel-server.exe" --service-run --launch"#
            ),
            exe
        ));
        assert!(!launch_path_matches(
            Path::new(r#""C:\Elsewhere\labsentinel-server.exe" --service-run"#),
            exe
        ));
    }
}
