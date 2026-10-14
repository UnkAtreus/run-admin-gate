//! `/install` and `/uninstall`. Exit code 0 on success, 1603 on failure
//! (the manager GUI checks for 1603).

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use runadmingate_core::{LogonMode, EXIT_INSTALL_FAILURE, INSTALLED_LAUNCHER, SERVICE_NAME};
use windows::core::HSTRING;
use windows::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_DELAY_UNTIL_REBOOT};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::UI::Shell::{FOLDERID_ProgramFiles, SHGetKnownFolderPath, KF_FLAG_DEFAULT};
use windows::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_ICONINFORMATION, MB_OK};
use windows_service::service::{
    ServiceAccess, ServiceAction, ServiceActionType, ServiceErrorControl, ServiceFailureActions,
    ServiceFailureResetPeriod, ServiceInfo, ServiceStartType, ServiceState, ServiceType,
};
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};

use super::registry;
use super::util::report;

const DESCRIPTION: &str =
    "Lets standard users start allow-listed programs with SYSTEM or administrator rights.";

pub struct InstallOptions {
    pub quiet: bool,
    pub allowed: Vec<String>,
    pub mode: Option<LogonMode>,
}

/// Parses the arguments after `/install`.
pub fn parse_options(args: &[String]) -> Result<InstallOptions, String> {
    let mut options = InstallOptions {
        quiet: false,
        allowed: Vec::new(),
        mode: None,
    };
    for arg in args {
        let lower = arg.to_ascii_lowercase();
        if lower == "/quiet" {
            options.quiet = true;
        } else if lower == "/asadmin" {
            options.mode = Some(LogonMode::Admin);
        } else if lower == "/assystem" || lower == "/asservice" {
            options.mode = Some(LogonMode::System);
        } else if lower.starts_with("/allowedpath:") {
            let value = &arg["/allowedpath:".len()..];
            options.allowed.extend(
                value
                    .split(';')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(String::from),
            );
        } else {
            return Err(format!("Unknown option: {arg}"));
        }
    }
    Ok(options)
}

fn program_files() -> Result<PathBuf, String> {
    unsafe {
        let raw = SHGetKnownFolderPath(&FOLDERID_ProgramFiles, KF_FLAG_DEFAULT, None)
            .map_err(|e| e.message())?;
        let dir = raw.to_string();
        CoTaskMemFree(Some(raw.0 as *const _));
        dir.map(PathBuf::from).map_err(|e| e.to_string())
    }
}

fn installed_path() -> Result<PathBuf, String> {
    Ok(program_files()?.join(INSTALLED_LAUNCHER))
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a
            .to_string_lossy()
            .eq_ignore_ascii_case(&b.to_string_lossy()),
        _ => false,
    }
}

fn wait_for_state(
    service: &windows_service::service::Service,
    wanted: ServiceState,
    timeout: Duration,
) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        match service.query_status() {
            Ok(status) if status.current_state == wanted => return true,
            Ok(_) => std::thread::sleep(Duration::from_millis(250)),
            Err(_) => return false,
        }
    }
    false
}

fn stop_if_running(manager: &ServiceManager) {
    let access = ServiceAccess::QUERY_STATUS | ServiceAccess::STOP;
    if let Ok(service) = manager.open_service(SERVICE_NAME, access) {
        if matches!(service.query_status(), Ok(s) if s.current_state != ServiceState::Stopped) {
            let _ = service.stop();
            wait_for_state(&service, ServiceState::Stopped, Duration::from_secs(15));
        }
    }
}

fn do_install(options: &InstallOptions) -> Result<String, String> {
    let manager = ServiceManager::local_computer(
        None::<&str>,
        ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE,
    )
    .map_err(|e| {
        format!("Can't open the Service Control Manager. Run this as an administrator.\n{e}")
    })?;
    stop_if_running(&manager);

    let target = installed_path()?;
    let current = std::env::current_exe().map_err(|e| e.to_string())?;
    if !same_file(&current, &target) {
        std::fs::create_dir_all(target.parent().unwrap())
            .map_err(|e| format!("Can't create {}: {e}", target.display()))?;
        // The service may need a moment to release the old binary.
        let mut copied = std::fs::copy(&current, &target);
        for _ in 0..20 {
            if copied.is_ok() {
                break;
            }
            std::thread::sleep(Duration::from_millis(250));
            copied = std::fs::copy(&current, &target);
        }
        copied.map_err(|e| format!("Can't copy the launcher to {}: {e}", target.display()))?;
    }

    let info = ServiceInfo {
        name: OsString::from(SERVICE_NAME),
        display_name: OsString::from(SERVICE_NAME),
        service_type: ServiceType::OWN_PROCESS,
        start_type: ServiceStartType::AutoStart,
        error_control: ServiceErrorControl::Normal,
        executable_path: target.clone(),
        launch_arguments: vec![OsString::from("/service")],
        dependencies: vec![],
        account_name: None,
        account_password: None,
    };
    let access = ServiceAccess::QUERY_STATUS
        | ServiceAccess::START
        | ServiceAccess::STOP
        | ServiceAccess::CHANGE_CONFIG;
    let service = match manager.open_service(SERVICE_NAME, access) {
        Ok(service) => {
            service
                .change_config(&info)
                .map_err(|e| format!("Can't update the service: {e}"))?;
            service
        }
        Err(_) => manager
            .create_service(&info, access)
            .map_err(|e| format!("Can't create the service: {e}"))?,
    };
    let _ = service.set_description(DESCRIPTION);
    let restart = ServiceAction {
        action_type: ServiceActionType::Restart,
        delay: Duration::from_secs(5),
    };
    let _ = service.update_failure_actions(ServiceFailureActions {
        reset_period: ServiceFailureResetPeriod::After(Duration::from_secs(86400)),
        reboot_msg: None,
        command: None,
        actions: Some(vec![restart.clone(), restart]),
    });

    runadmingate_win::config::ensure(&options.allowed, options.mode).map_err(|e| {
        format!(
            "Can't write HKLM\\SOFTWARE\\RunAdminGate: {}",
            super::util::error_text(e.0)
        )
    })?;
    let _ = registry::register_event_source();
    // Open-with stubs are copies of the launcher; keep them on this version.
    runadmingate_win::openwith::refresh_stubs(&target);

    service
        .start(&[] as &[&str])
        .map_err(|e| format!("Can't start the service: {e}"))?;
    if !wait_for_state(&service, ServiceState::Running, Duration::from_secs(15)) {
        return Err(
            "The service did not reach the running state. See the Application event log.".into(),
        );
    }

    Ok(format!(
        "RunAdminGate is installed and running.\n{}",
        target.display()
    ))
}

fn do_uninstall() -> Result<String, String> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        .map_err(|e| {
            format!("Can't open the Service Control Manager. Run this as an administrator.\n{e}")
        })?;
    stop_if_running(&manager);
    if let Ok(service) = manager.open_service(SERVICE_NAME, ServiceAccess::DELETE) {
        service
            .delete()
            .map_err(|e| format!("Can't delete the service: {e}"))?;
    }
    runadmingate_win::openwith::remove_all();
    runadmingate_win::shortcuts::remove_all();
    runadmingate_win::config::delete().map_err(|e| {
        format!(
            "Can't remove HKLM\\SOFTWARE\\RunAdminGate: {}",
            super::util::error_text(e.0)
        )
    })?;
    let _ = registry::unregister_event_source();

    let target = installed_path()?;
    let current = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut note = String::new();
    if target.exists() {
        // The service process can hold its image for a moment after it
        // reports STOPPED, so retry before falling back to a reboot.
        let mut removed = false;
        if !same_file(&current, &target) {
            for _ in 0..40 {
                if std::fs::remove_file(&target).is_ok() {
                    removed = true;
                    break;
                }
                std::thread::sleep(Duration::from_millis(250));
            }
        }
        if removed {
            let _ = std::fs::remove_dir(target.parent().unwrap());
        } else {
            unsafe {
                let _ = MoveFileExW(
                    &HSTRING::from(target.as_os_str()),
                    None,
                    MOVEFILE_DELAY_UNTIL_REBOOT,
                );
                let _ = MoveFileExW(
                    &HSTRING::from(target.parent().unwrap().as_os_str()),
                    None,
                    MOVEFILE_DELAY_UNTIL_REBOOT,
                );
            }
            note = "\nThe program files are removed at the next restart.".into();
        }
    }
    Ok(format!(
        "RunAdminGate is uninstalled. Its allow-list, Open with entries and shortcuts were removed.{note}"
    ))
}

pub fn install(args: &[String]) -> i32 {
    let options = match parse_options(args) {
        Ok(options) => options,
        Err(err) => {
            report(
                &err,
                MB_OK | MB_ICONERROR,
                args.iter().any(|a| a.eq_ignore_ascii_case("/quiet")),
            );
            return EXIT_INSTALL_FAILURE;
        }
    };
    finish(do_install(&options), options.quiet)
}

pub fn uninstall(args: &[String]) -> i32 {
    let quiet = args.iter().any(|a| a.eq_ignore_ascii_case("/quiet"));
    finish(do_uninstall(), quiet)
}

fn finish(result: Result<String, String>, quiet: bool) -> i32 {
    match result {
        Ok(text) => {
            report(&text, MB_OK | MB_ICONINFORMATION, quiet);
            0
        }
        Err(text) => {
            report(&text, MB_OK | MB_ICONERROR, quiet);
            EXIT_INSTALL_FAILURE
        }
    }
}
