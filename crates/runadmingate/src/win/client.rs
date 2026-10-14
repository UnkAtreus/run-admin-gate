//! `runadmingate.exe <program> [arguments]`: asks the service to start a program.

use runadmingate_core::path::normalize;
use runadmingate_core::protocol::{LaunchRequest, Response, MAX_MESSAGE, PIPE_NAME};
use runadmingate_core::SERVICE_NAME;
use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Foundation::{
    LocalFree, ERROR_ACCESS_DENIED, ERROR_FILE_NOT_FOUND, ERROR_PIPE_BUSY,
    ERROR_SERVICE_NOT_ACTIVE, ERROR_SUCCESS, HANDLE, HLOCAL,
};
use windows::Win32::Security::Authorization::{GetSecurityInfo, SE_KERNEL_OBJECT};
use windows::Win32::Security::{
    IsWellKnownSid, WinBuiltinAdministratorsSid, WinLocalSystemSid, OWNER_SECURITY_INFORMATION,
    PSECURITY_DESCRIPTOR, PSID,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, ReadFile, SearchPathW, WriteFile, FILE_FLAGS_AND_ATTRIBUTES, FILE_READ_ATTRIBUTES,
    FILE_READ_DATA, FILE_SHARE_NONE, FILE_WRITE_ATTRIBUTES, FILE_WRITE_DATA, OPEN_EXISTING,
    READ_CONTROL, SECURITY_IDENTIFICATION, SECURITY_SQOS_PRESENT, SYNCHRONIZE,
};
use windows::Win32::System::Pipes::{
    GetNamedPipeServerProcessId, SetNamedPipeHandleState, WaitNamedPipeW, PIPE_READMODE_MESSAGE,
};
use windows::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_ICONWARNING, MB_OK};
use windows_service::service::ServiceAccess;
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};

use super::util::{error_text, from_wide, report, Handle};

/// Turns what the user typed into an absolute path. The service checks the
/// result against the allow-list, so this is only a convenience.
fn resolve(program: &str) -> String {
    if normalize(program).is_some() {
        return program.to_string();
    }
    if !program.contains(['\\', '/']) {
        let mut buf = vec![0u16; 32 * 1024];
        let len = unsafe {
            SearchPathW(
                PCWSTR::null(),
                &HSTRING::from(program),
                &HSTRING::from(".exe"),
                Some(&mut buf),
                None,
            )
        };
        if len > 0 && (len as usize) < buf.len() {
            return from_wide(&buf);
        }
    }
    std::path::absolute(program)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| program.to_string())
}

/// The service creates the pipe as SYSTEM, so it is owned by SYSTEM or
/// Administrators. A standard user squatting on the name would own it
/// themselves. Unlike asking the SCM, this works when policy restricts the SCM.
fn owned_by_system(pipe: HANDLE) -> bool {
    unsafe {
        let mut owner = PSID::default();
        let mut sd = PSECURITY_DESCRIPTOR::default();
        let err = GetSecurityInfo(
            pipe,
            SE_KERNEL_OBJECT,
            OWNER_SECURITY_INFORMATION,
            Some(&mut owner),
            None,
            None,
            None,
            Some(&mut sd),
        );
        if err != ERROR_SUCCESS {
            return false;
        }
        let ok = IsWellKnownSid(owner, WinLocalSystemSid).as_bool()
            || IsWellKnownSid(owner, WinBuiltinAdministratorsSid).as_bool();
        let _ = LocalFree(HLOCAL(sd.0));
        ok
    }
}

/// The service's process ID, when the SCM lets us ask.
fn service_pid() -> Option<u32> {
    let manager =
        ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT).ok()?;
    let service = manager
        .open_service(SERVICE_NAME, ServiceAccess::QUERY_STATUS)
        .ok()?;
    service.query_status().ok()?.process_id
}

fn win32(err: &windows::core::Error) -> u32 {
    (err.code().0 as u32) & 0xffff
}

fn connect() -> Result<Handle, u32> {
    let name = HSTRING::from(PIPE_NAME);
    for _ in 0..3 {
        let opened = unsafe {
            CreateFileW(
                &name,
                (FILE_READ_DATA
                    | FILE_WRITE_DATA
                    | FILE_READ_ATTRIBUTES
                    | FILE_WRITE_ATTRIBUTES
                    | READ_CONTROL
                    | SYNCHRONIZE)
                    .0,
                FILE_SHARE_NONE,
                None,
                OPEN_EXISTING,
                // Let the server identify us, but not act as us.
                FILE_FLAGS_AND_ATTRIBUTES(SECURITY_SQOS_PRESENT.0 | SECURITY_IDENTIFICATION.0),
                None,
            )
        };
        match opened {
            Ok(handle) => return Ok(Handle(handle)),
            Err(err) if win32(&err) == ERROR_PIPE_BUSY.0 => unsafe {
                let _ = WaitNamedPipeW(&name, 5000);
            },
            Err(err) => return Err(win32(&err)),
        }
    }
    Err(ERROR_PIPE_BUSY.0)
}

fn exchange(pipe: HANDLE, request: &LaunchRequest) -> Result<Response, u32> {
    unsafe {
        let mode = PIPE_READMODE_MESSAGE;
        SetNamedPipeHandleState(pipe, Some(&mode), None, None).map_err(|e| win32(&e))?;
        let bytes = request.encode();
        let mut written = 0u32;
        WriteFile(pipe, Some(&bytes), Some(&mut written), None).map_err(|e| win32(&e))?;
        let mut buf = vec![0u8; MAX_MESSAGE];
        let mut read = 0u32;
        ReadFile(pipe, Some(&mut buf), Some(&mut read), None).map_err(|e| win32(&e))?;
        buf.truncate(read as usize);
        Response::decode(&buf).map_err(|_| ERROR_ACCESS_DENIED.0)
    }
}

/// Returns the process exit code: 0 started, 5 refused, otherwise a Win32 error.
pub fn run(program: &str, args: &str, quiet: bool) -> i32 {
    let show = |text: &str, style| {
        report(text, MB_OK | style, quiet);
    };
    let request = LaunchRequest {
        program: resolve(program),
        args: args.trim().to_string(),
    };

    let pipe = match connect() {
        Ok(pipe) => pipe,
        Err(code) if code == ERROR_FILE_NOT_FOUND.0 => {
            show("The RunAdminGate service is not running.", MB_ICONERROR);
            return ERROR_SERVICE_NOT_ACTIVE.0 as i32;
        }
        Err(code) => {
            show(
                &format!(
                    "Can't reach the RunAdminGate service.\n{}",
                    error_text(code)
                ),
                MB_ICONERROR,
            );
            return code as i32;
        }
    };

    // Only talk to the real service, not to someone squatting on the pipe name:
    // the pipe must be owned by SYSTEM, and when the SCM answers, the server
    // process must be the service.
    let mut server_pid = 0u32;
    let genuine = owned_by_system(pipe.raw())
        && unsafe { GetNamedPipeServerProcessId(pipe.raw(), &mut server_pid) }.is_ok()
        && service_pid().map_or(true, |pid| pid == server_pid);
    if !genuine {
        show(
            "The RunAdminGate pipe is not served by the RunAdminGate service.",
            MB_ICONERROR,
        );
        return ERROR_ACCESS_DENIED.0 as i32;
    }

    match exchange(pipe.raw(), &request) {
        Ok(Response::Started { .. }) => 0,
        Ok(Response::Refused { reason, detail }) => {
            show(&format!("{}\n\n{detail}", reason.message()), MB_ICONWARNING);
            ERROR_ACCESS_DENIED.0 as i32
        }
        Ok(Response::Failed { code, detail }) => {
            show(
                &format!(
                    "The program could not be started.\n{}\n\n{detail}",
                    request.program
                ),
                MB_ICONERROR,
            );
            code as i32
        }
        Err(code) => {
            show(
                &format!(
                    "Communication with the RunAdminGate service failed.\n{}",
                    error_text(code)
                ),
                MB_ICONERROR,
            );
            code as i32
        }
    }
}
