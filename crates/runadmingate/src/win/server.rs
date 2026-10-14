//! Named-pipe server run by the service.
//!
//! * The pipe DACL lets interactive users read and write but not create pipe
//!   instances, so nobody else can serve clients under this name.
//! * The first instance uses `FILE_FLAG_FIRST_PIPE_INSTANCE`: if someone
//!   squatted the name before the service started, the service refuses to run.
//! * Remote clients are rejected; every read and write has a timeout and the
//!   number of concurrent clients is capped.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use runadmingate_core::protocol::{LaunchRequest, Refusal, Response, MAX_MESSAGE, PIPE_NAME};
use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Foundation::{
    LocalFree, BOOL, ERROR_IO_PENDING, ERROR_MORE_DATA, ERROR_PIPE_CONNECTED, ERROR_TIMEOUT,
    HANDLE, HLOCAL, WAIT_OBJECT_0, WIN32_ERROR,
};
use windows::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
use windows::Win32::Storage::FileSystem::{
    ReadFile, WriteFile, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED, PIPE_ACCESS_DUPLEX,
};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_MESSAGE,
    PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_MESSAGE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};
use windows::Win32::System::Threading::{
    CreateEventW, WaitForMultipleObjects, WaitForSingleObject,
};
use windows::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};

use super::eventlog::{self, id, Level};
use super::handler;
use super::util::Handle;

/// SYSTEM and Administrators: full. Interactive users: read/write data and
/// attributes only (0x120183; no FILE_CREATE_PIPE_INSTANCE). Network: denied.
const PIPE_SDDL: &str = "D:P(D;;GA;;;NU)(A;;GA;;;SY)(A;;GA;;;BA)(A;;0x120183;;;IU)";
const MAX_CLIENTS: usize = 16;
const IO_TIMEOUT: Duration = Duration::from_secs(10);

struct SecurityDescriptor(PSECURITY_DESCRIPTOR);

impl Drop for SecurityDescriptor {
    fn drop(&mut self) {
        unsafe {
            let _ = LocalFree(HLOCAL(self.0 .0));
        }
    }
}

fn pipe_security() -> windows::core::Result<SecurityDescriptor> {
    let mut sd = PSECURITY_DESCRIPTOR::default();
    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            &HSTRING::from(PIPE_SDDL),
            SDDL_REVISION_1,
            &mut sd,
            None,
        )?;
    }
    Ok(SecurityDescriptor(sd))
}

fn new_event() -> windows::core::Result<Handle> {
    Ok(Handle(unsafe {
        CreateEventW(None, BOOL::from(true), BOOL::from(false), PCWSTR::null())?
    }))
}

fn win32(err: &windows::core::Error) -> WIN32_ERROR {
    WIN32_ERROR((err.code().0 as u32) & 0xffff)
}

/// Waits for an overlapped operation; cancels it on timeout. Returns bytes transferred.
fn finish_io(
    pipe: HANDLE,
    ov: &OVERLAPPED,
    started: windows::core::Result<()>,
    timeout: Duration,
) -> Result<u32, WIN32_ERROR> {
    let mut transferred = 0u32;
    unsafe {
        match started {
            Ok(()) => {}
            Err(err) if win32(&err) == ERROR_IO_PENDING => {
                if WaitForSingleObject(ov.hEvent, timeout.as_millis() as u32) != WAIT_OBJECT_0 {
                    let _ = CancelIoEx(pipe, Some(ov));
                    // Wait for the cancellation so the buffer and OVERLAPPED stay valid.
                    let _ = GetOverlappedResult(pipe, ov, &mut transferred, BOOL::from(true));
                    return Err(ERROR_TIMEOUT);
                }
            }
            Err(err) => return Err(win32(&err)),
        }
        GetOverlappedResult(pipe, ov, &mut transferred, BOOL::from(false))
            .map_err(|e| win32(&e))?;
    }
    Ok(transferred)
}

fn read_message(pipe: HANDLE, timeout: Duration) -> Result<Vec<u8>, WIN32_ERROR> {
    let event = new_event().map_err(|e| win32(&e))?;
    let mut buf = vec![0u8; MAX_MESSAGE];
    let mut ov = OVERLAPPED {
        hEvent: event.raw(),
        ..Default::default()
    };
    let started = unsafe { ReadFile(pipe, Some(&mut buf), None, Some(&mut ov)) };
    match finish_io(pipe, &ov, started, timeout) {
        Ok(n) => {
            buf.truncate(n as usize);
            Ok(buf)
        }
        Err(err) if err == ERROR_MORE_DATA => Err(ERROR_MORE_DATA),
        Err(err) => Err(err),
    }
}

fn write_message(pipe: HANDLE, data: &[u8], timeout: Duration) -> Result<(), WIN32_ERROR> {
    let event = new_event().map_err(|e| win32(&e))?;
    let mut ov = OVERLAPPED {
        hEvent: event.raw(),
        ..Default::default()
    };
    let started = unsafe { WriteFile(pipe, Some(data), None, Some(&mut ov)) };
    finish_io(pipe, &ov, started, timeout).map(|_| ())
}

fn serve_client(pipe: Handle) {
    let response = match read_message(pipe.raw(), IO_TIMEOUT) {
        Ok(bytes) => match LaunchRequest::decode(&bytes) {
            Ok(request) => handler::handle(pipe.raw(), &request),
            Err(_) => Response::Refused {
                reason: Refusal::BadRequest,
                detail: String::new(),
            },
        },
        Err(_) => Response::Refused {
            reason: Refusal::BadRequest,
            detail: String::new(),
        },
    };
    if write_message(pipe.raw(), &response.encode(), IO_TIMEOUT).is_ok() {
        // Let the client read before disconnecting (disconnect discards unread data).
        let _ = read_message(pipe.raw(), Duration::from_secs(5));
    }
    unsafe {
        let _ = DisconnectNamedPipe(pipe.raw());
    }
}

/// Serves requests until `stop` is signaled. Errors only if the pipe can't be created.
pub fn serve(stop: HANDLE) -> windows::core::Result<()> {
    let sd = pipe_security()?;
    let attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: sd.0 .0,
        bInheritHandle: BOOL::from(false),
    };
    let active = Arc::new(AtomicUsize::new(0));
    let name = HSTRING::from(PIPE_NAME);
    let mut first = true;

    loop {
        let mut open_mode = PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED;
        if first {
            open_mode |= FILE_FLAG_FIRST_PIPE_INSTANCE;
        }
        let pipe = unsafe {
            CreateNamedPipeW(
                &name,
                open_mode,
                PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                PIPE_UNLIMITED_INSTANCES,
                MAX_MESSAGE as u32,
                MAX_MESSAGE as u32,
                0,
                Some(&attributes),
            )
        };
        if pipe.is_invalid() {
            let err = windows::core::Error::from_win32();
            if first {
                return Err(err);
            }
            eventlog::report(
                Level::Error,
                id::SERVICE_ERROR,
                &format!("CreateNamedPipe failed: {err}"),
            );
            if unsafe { WaitForSingleObject(stop, 1000) } == WAIT_OBJECT_0 {
                return Ok(());
            }
            continue;
        }
        first = false;
        let pipe = Handle(pipe);

        let event = new_event()?;
        let mut ov = OVERLAPPED {
            hEvent: event.raw(),
            ..Default::default()
        };
        let connected = match unsafe { ConnectNamedPipe(pipe.raw(), Some(&mut ov)) } {
            Ok(()) => true,
            Err(err) if win32(&err) == ERROR_PIPE_CONNECTED => true,
            Err(err) if win32(&err) == ERROR_IO_PENDING => {
                let waited = unsafe {
                    WaitForMultipleObjects(&[event.raw(), stop], BOOL::from(false), u32::MAX)
                };
                if waited == WAIT_OBJECT_0 {
                    let mut n = 0u32;
                    unsafe {
                        GetOverlappedResult(pipe.raw(), &ov, &mut n, BOOL::from(false)).is_ok()
                    }
                } else {
                    unsafe {
                        let _ = CancelIoEx(pipe.raw(), Some(&ov));
                        let mut n = 0u32;
                        let _ = GetOverlappedResult(pipe.raw(), &ov, &mut n, BOOL::from(true));
                    }
                    return Ok(());
                }
            }
            Err(_) => false,
        };
        if !connected {
            continue;
        }
        if active.load(Ordering::SeqCst) >= MAX_CLIENTS {
            unsafe {
                let _ = DisconnectNamedPipe(pipe.raw());
            }
            continue;
        }
        active.fetch_add(1, Ordering::SeqCst);
        let counter = Arc::clone(&active);
        std::thread::spawn(move || {
            serve_client(pipe);
            counter.fetch_sub(1, Ordering::SeqCst);
        });
    }
}
