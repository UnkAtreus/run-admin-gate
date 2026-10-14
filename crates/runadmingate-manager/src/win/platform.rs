//! Service state, the launcher process, and small Win32 helpers.

use std::path::PathBuf;
use std::process::Command;

use runadmingate_core::{LAUNCHER_EXE, SERVICE_NAME};
use windows::core::HSTRING;
use windows::Win32::System::Environment::ExpandEnvironmentStringsW;
use windows_service::service::{ServiceAccess, ServiceState};
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Service {
    NotInstalled,
    Stopped,
    Running,
    Busy,
}

pub fn service() -> Service {
    let Ok(manager) = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
    else {
        return Service::NotInstalled;
    };
    let Ok(service) = manager.open_service(SERVICE_NAME, ServiceAccess::QUERY_STATUS) else {
        return Service::NotInstalled;
    };
    match service.query_status().map(|s| s.current_state) {
        Ok(ServiceState::Running) => Service::Running,
        Ok(ServiceState::Stopped) => Service::Stopped,
        Ok(_) => Service::Busy,
        Err(_) => Service::NotInstalled,
    }
}

/// `runadmingate.exe` shipped next to the manager.
pub fn bundled_launcher() -> Option<PathBuf> {
    let path = std::env::current_exe().ok()?.with_file_name(LAUNCHER_EXE);
    path.exists().then_some(path)
}

#[derive(Debug, Clone, Copy)]
pub enum LauncherAction {
    Install,
    Uninstall,
}

/// Runs `runadmingate.exe /install|/uninstall /quiet` and returns its message.
pub fn run_launcher(action: LauncherAction) -> Result<String, String> {
    let launcher = bundled_launcher()
        .or_else(|| runadmingate_win::shell::installed_launcher().filter(|p| p.exists()))
        .ok_or_else(|| format!("{LAUNCHER_EXE} was not found next to the manager."))?;
    let switch = match action {
        LauncherAction::Install => "/install",
        LauncherAction::Uninstall => "/uninstall",
    };
    let output = Command::new(&launcher)
        .args([switch, "/quiet"])
        .output()
        .map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if output.status.success() {
        Ok(text)
    } else if text.is_empty() {
        Err(format!(
            "{LAUNCHER_EXE} failed (exit code {}).",
            output.status.code().unwrap_or(-1)
        ))
    } else {
        Err(text)
    }
}

/// Expands `%VAR%` with this process's environment (for display only; the
/// service expands with its own environment).
pub fn expand(text: &str) -> String {
    if !text.contains('%') {
        return text.to_string();
    }
    unsafe {
        let mut buf = vec![0u16; 32 * 1024];
        let len = ExpandEnvironmentStringsW(&HSTRING::from(text), Some(&mut buf)) as usize;
        if len == 0 || len > buf.len() {
            return text.to_string();
        }
        String::from_utf16_lossy(&buf[..len - 1])
    }
}
