//! Service Control Manager integration (`runadmingate.exe /service`).

use std::ffi::OsString;
use std::time::Duration;

use runadmingate_core::SERVICE_NAME;
use windows::core::PCWSTR;
use windows::Win32::Foundation::BOOL;
use windows::Win32::System::Threading::{CreateEventW, SetEvent};
use windows_service::service::{
    ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus, ServiceType,
};
use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
use windows_service::{define_windows_service, service_dispatcher};

use super::eventlog::{self, id, Level};
use super::server;
use super::util::Handle;

define_windows_service!(ffi_service_main, service_main);

/// Entry point when started by the SCM. Returns the process exit code.
pub fn run() -> i32 {
    match service_dispatcher::start(SERVICE_NAME, ffi_service_main) {
        Ok(()) => 0,
        Err(_) => 1063, // ERROR_FAILED_SERVICE_CONTROLLER_CONNECT: not started by the SCM
    }
}

fn service_main(_arguments: Vec<OsString>) {
    if let Err(err) = run_service() {
        eventlog::report(
            Level::Error,
            id::SERVICE_ERROR,
            &format!("Service stopped with an error: {err}"),
        );
    }
}

fn status(state: ServiceState, code: u32) -> ServiceStatus {
    ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: state,
        controls_accepted: if state == ServiceState::Running {
            ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN
        } else {
            ServiceControlAccept::empty()
        },
        exit_code: ServiceExitCode::Win32(code),
        checkpoint: 0,
        wait_hint: Duration::from_secs(5),
        process_id: None,
    }
}

fn run_service() -> Result<(), Box<dyn std::error::Error>> {
    let stop =
        Handle(unsafe { CreateEventW(None, BOOL::from(true), BOOL::from(false), PCWSTR::null())? });
    let stop_raw = stop.raw().0 as usize;
    let status_handle =
        service_control_handler::register(SERVICE_NAME, move |control| match control {
            ServiceControl::Stop | ServiceControl::Shutdown => {
                unsafe {
                    let _ = SetEvent(windows::Win32::Foundation::HANDLE(stop_raw as *mut _));
                }
                ServiceControlHandlerResult::NoError
            }
            ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
            _ => ServiceControlHandlerResult::NotImplemented,
        })?;

    status_handle.set_service_status(status(ServiceState::Running, 0))?;
    eventlog::report(
        Level::Info,
        id::SERVICE_RUNNING,
        "RunAdminGate service started.",
    );

    let result = server::serve(stop.raw());
    let code = match &result {
        Ok(()) => 0,
        Err(err) => {
            eventlog::report(
                Level::Error,
                id::SERVICE_ERROR,
                &format!("Could not create the RunAdminGate pipe ({err}). Another process may be using its name."),
            );
            (err.code().0 as u32) & 0xffff
        }
    };
    status_handle.set_service_status(status(ServiceState::Stopped, code))?;
    eventlog::report(
        Level::Info,
        id::SERVICE_STOPPED,
        "RunAdminGate service stopped.",
    );
    Ok(())
}
