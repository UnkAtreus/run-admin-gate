//! Decides and executes one launch request.

use runadmingate_core::command::{self, BuildError};
use runadmingate_core::path::normalize;
use runadmingate_core::policy::{Decision, DenyReason, Policy};
use runadmingate_core::protocol::{LaunchRequest, Refusal, Response};
use runadmingate_core::LogonMode;
use windows::core::HSTRING;
use windows::Win32::Foundation::{GetLastError, ERROR_NO_SUCH_LOGON_SESSION, HANDLE};
use windows::Win32::Security::{
    GetLengthSid, GetTokenInformation, LookupAccountSidW, RevertToSelf, TokenGroups,
    TokenSessionId, TokenUser, SID_NAME_USE, TOKEN_GROUPS, TOKEN_QUERY, TOKEN_USER,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, GetFileInformationByHandle, GetFinalPathNameByHandleW, BY_HANDLE_FILE_INFORMATION,
    FILE_ATTRIBUTE_DIRECTORY, FILE_FLAGS_AND_ATTRIBUTES, FILE_GENERIC_READ, FILE_NAME_NORMALIZED,
    FILE_SHARE_READ, GETFINALPATHNAMEBYHANDLE_FLAGS, OPEN_EXISTING, VOLUME_NAME_DOS,
};
use windows::Win32::System::Environment::ExpandEnvironmentStringsW;
use windows::Win32::System::Pipes::ImpersonateNamedPipeClient;
use windows::Win32::System::SystemInformation::GetSystemDirectoryW;
use windows::Win32::System::Threading::{
    GetCurrentThread, OpenThreadToken, WaitForSingleObject, INFINITE,
};

use super::eventlog::{self, id, Level};
use super::launch;
use super::util::{error_text, from_wide, Handle};

/// Who sent the request, read from the pipe client's token.
struct Caller {
    session_id: u32,
    /// `DOMAIN\name`, for the event log.
    account: String,
    /// Set when the account name resolved and the session has a logon SID.
    user: Option<launch::User>,
}

/// The logon SID among the token's groups, as raw SID bytes.
unsafe fn logon_sid(token: HANDLE) -> Option<Vec<u8>> {
    let mut len = 0u32;
    let _ = GetTokenInformation(token, TokenGroups, None, 0, &mut len);
    let mut buf = vec![0u64; (len as usize).div_ceil(8)];
    GetTokenInformation(
        token,
        TokenGroups,
        Some(buf.as_mut_ptr().cast()),
        len,
        &mut len,
    )
    .ok()?;
    let groups = &*(buf.as_ptr() as *const TOKEN_GROUPS);
    let entries = std::slice::from_raw_parts(groups.Groups.as_ptr(), groups.GroupCount as usize);
    let entry = entries
        .iter()
        .find(|g| g.Attributes & launch::SE_GROUP_LOGON_ID == launch::SE_GROUP_LOGON_ID)?;
    let size = GetLengthSid(entry.Sid) as usize;
    Some(std::slice::from_raw_parts(entry.Sid.0 as *const u8, size).to_vec())
}

fn caller(pipe: HANDLE) -> Option<Caller> {
    unsafe {
        ImpersonateNamedPipeClient(pipe).ok()?;
        let mut token = HANDLE::default();
        let opened = OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, true, &mut token);
        if RevertToSelf().is_err() {
            // Continuing while impersonating would act with the caller's rights.
            std::process::abort();
        }
        opened.ok()?;
        let token = Handle(token);

        let mut session_id = 0u32;
        let mut len = 0u32;
        GetTokenInformation(
            token.raw(),
            TokenSessionId,
            Some(&mut session_id as *mut u32 as *mut _),
            4,
            &mut len,
        )
        .ok()?;

        let session_logon_sid = logon_sid(token.raw());
        let mut buf = vec![0u8; 256];
        let mut account = String::from("unknown user");
        let mut admin_user = None;
        if GetTokenInformation(
            token.raw(),
            TokenUser,
            Some(buf.as_mut_ptr().cast()),
            buf.len() as u32,
            &mut len,
        )
        .is_ok()
        {
            let user = &*(buf.as_ptr() as *const TOKEN_USER);
            let mut name = [0u16; 256];
            let mut domain = [0u16; 256];
            let (mut name_len, mut domain_len) = (name.len() as u32, domain.len() as u32);
            let mut use_ = SID_NAME_USE::default();
            if LookupAccountSidW(
                None,
                user.User.Sid,
                windows::core::PWSTR(name.as_mut_ptr()),
                &mut name_len,
                windows::core::PWSTR(domain.as_mut_ptr()),
                &mut domain_len,
                &mut use_,
            )
            .is_ok()
            {
                let (name, domain) = (from_wide(&name), from_wide(&domain));
                account = format!("{domain}\\{name}");
                admin_user = session_logon_sid.map(|logon_sid| launch::User {
                    name,
                    domain,
                    logon_sid,
                });
            }
        }
        Some(Caller {
            session_id,
            account,
            user: admin_user,
        })
    }
}

/// Opens the program so it can't be replaced while it runs: read access,
/// sharing only with readers. Returns the handle and the canonical path.
fn open_program(path: &str) -> Result<(Handle, String), u32> {
    unsafe {
        let handle = CreateFileW(
            &HSTRING::from(path),
            FILE_GENERIC_READ.0,
            FILE_SHARE_READ,
            None,
            OPEN_EXISTING,
            FILE_FLAGS_AND_ATTRIBUTES(0),
            None,
        )
        .map_err(|e| (e.code().0 as u32) & 0xffff)?;
        let handle = Handle(handle);
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        GetFileInformationByHandle(handle.raw(), &mut info).map_err(|_| GetLastError().0)?;
        if info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY.0 != 0 {
            return Err(windows::Win32::Foundation::ERROR_DIRECTORY.0);
        }
        let mut buf = vec![0u16; 32 * 1024];
        let len = GetFinalPathNameByHandleW(
            handle.raw(),
            &mut buf,
            GETFINALPATHNAMEBYHANDLE_FLAGS(FILE_NAME_NORMALIZED.0 | VOLUME_NAME_DOS.0),
        );
        if len == 0 || len as usize >= buf.len() {
            return Err(GetLastError().0);
        }
        Ok((handle, from_wide(&buf[..len as usize])))
    }
}

/// Expands `%VAR%` with the service's own (SYSTEM) environment.
fn expand(text: &str) -> String {
    unsafe {
        let source = HSTRING::from(text);
        let mut buf = vec![0u16; 32 * 1024];
        let len = ExpandEnvironmentStringsW(&source, Some(&mut buf));
        if len == 0 || len as usize > buf.len() {
            return text.to_string();
        }
        from_wide(&buf)
    }
}

fn system_dir() -> String {
    let mut buf = [0u16; 260];
    let len = unsafe { GetSystemDirectoryW(Some(&mut buf)) } as usize;
    from_wide(&buf[..len.min(buf.len())])
}

fn refuse(caller: &str, program: &str, reason: Refusal, note: &str) -> Response {
    eventlog::report(
        Level::Warning,
        id::REFUSED,
        &format!(
            "Refused for {caller}: {program}\n{}{note}",
            reason.message()
        ),
    );
    Response::Refused {
        reason,
        detail: program.to_string(),
    }
}

fn fail(caller: &str, program: &str, code: u32) -> Response {
    let detail = error_text(code);
    eventlog::report(
        Level::Error,
        id::FAILED,
        &format!("Could not start for {caller}: {program}\n{detail}"),
    );
    Response::Failed { code, detail }
}

pub fn handle(pipe: HANDLE, request: &LaunchRequest) -> Response {
    let Some(who) = caller(pipe) else {
        eventlog::report(
            Level::Warning,
            id::REFUSED,
            "Refused a request: the caller could not be identified.",
        );
        return Response::Refused {
            reason: Refusal::BadRequest,
            detail: String::new(),
        };
    };
    let account = who.account.as_str();
    if who.session_id == 0 {
        return refuse(account, &request.program, Refusal::NotInteractive, "");
    }
    if normalize(&request.program).is_none() {
        return refuse(account, &request.program, Refusal::NotAbsolute, "");
    }

    // Held until the child exits, so the file can't be swapped or edited meanwhile.
    let (file, canonical) = match open_program(&request.program) {
        Ok(opened) => opened,
        Err(code) => return fail(account, &request.program, code),
    };
    let canonical = normalize(&canonical)
        .map(|p| p.to_path_string())
        .unwrap_or(canonical);

    let (allowed, default_mode) = match runadmingate_win::config::load() {
        Ok(config) => config,
        Err(err) => return fail(account, &canonical, err.0),
    };
    let (policy, errors) = Policy::compile(&allowed, default_mode, expand);
    if !errors.is_empty() {
        let list: Vec<String> = errors
            .iter()
            .map(|e| format!("{}: {}", e.entry, e.reason))
            .collect();
        eventlog::report(
            Level::Warning,
            id::CONFIG_WARNING,
            &format!("Ignored AllowedPath entries:\n{}", list.join("\n")),
        );
    }

    let (mode, args) = match policy.evaluate(&canonical, &request.args) {
        Decision::Allow { mode, args, .. } => (mode, args),
        Decision::Deny(DenyReason::NotAbsolute) => {
            return refuse(account, &canonical, Refusal::NotAbsolute, "")
        }
        Decision::Deny(DenyReason::NotListed) => {
            let note = if request.args.trim().is_empty() {
                String::new()
            } else {
                format!("\nArguments: {}", request.args.trim())
            };
            return refuse(account, &canonical, Refusal::NotListed, &note);
        }
    };
    let spec = match command::build(&system_dir(), &canonical, &args) {
        Ok(spec) => spec,
        Err(BuildError::UnsupportedType | BuildError::BadPath) => {
            return refuse(account, &canonical, Refusal::UnsupportedType, "")
        }
        Err(BuildError::UnsafeForCmd | BuildError::UnsafeHostOption) => {
            return refuse(account, &canonical, Refusal::UnsafeArguments, "")
        }
    };

    let (started, how) = match mode {
        LogonMode::System => (launch::start_as_system(who.session_id, &spec), "as SYSTEM"),
        LogonMode::Admin => match &who.user {
            Some(user) => (
                launch::start_as_admin(who.session_id, user, &spec),
                "as the user with administrator rights",
            ),
            None => return fail(account, &canonical, ERROR_NO_SUCH_LOGON_SESSION.0),
        },
    };
    match started {
        Ok(started) => {
            eventlog::report(
                Level::Info,
                id::STARTED,
                &format!(
                    "Started {how} for {account} (session {}): {}\nProcess ID {}",
                    who.session_id, spec.command_line, started.pid
                ),
            );
            let pid = started.pid;
            std::thread::spawn(move || {
                unsafe { WaitForSingleObject(started.process.raw(), INFINITE) };
                drop(file);
            });
            Response::Started { pid, mode }
        }
        Err(err) => fail(account, &canonical, (err.code().0 as u32) & 0xffff),
    }
}
