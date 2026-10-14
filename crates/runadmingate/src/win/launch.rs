//! Starting a process as SYSTEM on the requesting user's desktop.

use std::ffi::c_void;

use runadmingate_core::command::CommandSpec;
use windows::core::{Result, HSTRING, PWSTR};
use windows::Win32::Foundation::{BOOL, HANDLE};
use windows::Win32::Security::{
    DuplicateTokenEx, SecurityImpersonation, SetTokenInformation, TokenPrimary, TokenSessionId,
    TOKEN_ACCESS_MASK, TOKEN_ADJUST_DEFAULT, TOKEN_ADJUST_SESSIONID, TOKEN_ASSIGN_PRIMARY,
    TOKEN_DUPLICATE, TOKEN_QUERY,
};
use windows::Win32::System::Environment::{CreateEnvironmentBlock, DestroyEnvironmentBlock};
use windows::Win32::System::Threading::{
    CreateProcessAsUserW, GetCurrentProcess, OpenProcessToken, CREATE_NEW_CONSOLE,
    CREATE_UNICODE_ENVIRONMENT, PROCESS_INFORMATION, STARTUPINFOW,
};

use super::util::{wide, Handle};

pub struct Started {
    pub pid: u32,
    pub process: Handle,
}

/// The service's own SYSTEM token, as a primary token bound to `session_id`.
fn system_token_for_session(session_id: u32) -> Result<Handle> {
    unsafe {
        let mut own = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_DUPLICATE | TOKEN_QUERY, &mut own)?;
        let own = Handle(own);
        let access: TOKEN_ACCESS_MASK = TOKEN_QUERY
            | TOKEN_DUPLICATE
            | TOKEN_ASSIGN_PRIMARY
            | TOKEN_ADJUST_DEFAULT
            | TOKEN_ADJUST_SESSIONID;
        let mut primary = HANDLE::default();
        DuplicateTokenEx(
            own.raw(),
            access,
            None,
            SecurityImpersonation,
            TokenPrimary,
            &mut primary,
        )?;
        let primary = Handle(primary);
        // Needs SeTcbPrivilege, which LocalSystem holds.
        SetTokenInformation(
            primary.raw(),
            TokenSessionId,
            &session_id as *const u32 as *const c_void,
            std::mem::size_of::<u32>() as u32,
        )?;
        Ok(primary)
    }
}

/// Starts `spec` as SYSTEM in `session_id` on `winsta0\default`, with the
/// SYSTEM profile environment (never the caller's environment).
pub fn start_as_system(session_id: u32, spec: &CommandSpec) -> Result<Started> {
    let token = system_token_for_session(session_id)?;
    unsafe {
        let mut env: *mut c_void = std::ptr::null_mut();
        CreateEnvironmentBlock(&mut env, token.raw(), BOOL::from(false))?;

        let mut desktop = wide(r"winsta0\default");
        let startup = STARTUPINFOW {
            cb: std::mem::size_of::<STARTUPINFOW>() as u32,
            lpDesktop: PWSTR(desktop.as_mut_ptr()),
            ..Default::default()
        };
        let mut info = PROCESS_INFORMATION::default();
        let mut command_line = wide(&spec.command_line);
        let result = CreateProcessAsUserW(
            token.raw(),
            &HSTRING::from(spec.application.as_str()),
            PWSTR(command_line.as_mut_ptr()),
            None,
            None,
            BOOL::from(false),
            CREATE_UNICODE_ENVIRONMENT | CREATE_NEW_CONSOLE,
            Some(env),
            &HSTRING::from(spec.current_dir.as_str()),
            &startup,
            &mut info,
        );
        let _ = DestroyEnvironmentBlock(env);
        result?;
        drop(Handle(info.hThread));
        Ok(Started {
            pid: info.dwProcessId,
            process: Handle(info.hProcess),
        })
    }
}
