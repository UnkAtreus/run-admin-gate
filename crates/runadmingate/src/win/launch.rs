//! Starting a process on the requesting user's desktop: as SYSTEM, or as the
//! user with administrator rights.
//!
//! Administrator mode never changes the local Administrators group. The
//! service asks LSA for a fresh S4U logon of the user and adds Administrators
//! to that one token (`LocalGroups`, which needs SeTcbPrivilege). It also adds
//! the logon SID of the user's session, which is what grants access to their
//! window station and desktop.

use std::ffi::c_void;

use runadmingate_core::command::CommandSpec;
use windows::core::{Error, Result, HRESULT, HSTRING, PCWSTR, PSTR, PWSTR};
use windows::Win32::Foundation::{
    GetLastError, BOOL, ERROR_NOT_ALL_ASSIGNED, HANDLE, LUID, NTSTATUS,
};
use windows::Win32::Security::Authentication::Identity::{
    KerbS4ULogon, LsaDeregisterLogonProcess, LsaFreeReturnBuffer, LsaLogonUser,
    LsaLookupAuthenticationPackage, LsaNtStatusToWinError, LsaRegisterLogonProcess, MsV1_0S4ULogon,
    NameSamCompatible, NameUserPrincipal, TranslateNameW, KERB_S4U_LOGON, LSA_STRING,
    LSA_UNICODE_STRING, MSV1_0_S4U_LOGON, SECURITY_LOGON_TYPE,
};
use windows::Win32::Security::{
    AdjustTokenPrivileges, AllocateLocallyUniqueId, CreateWellKnownSid, DuplicateTokenEx,
    GetTokenInformation, LookupPrivilegeValueW, SecurityImpersonation, SetTokenInformation,
    TokenElevationType, TokenElevationTypeLimited, TokenLinkedToken, TokenPrimary, TokenSessionId,
    WinBuiltinAdministratorsSid, WinInteractiveSid, LUID_AND_ATTRIBUTES, PSID, QUOTA_LIMITS,
    SE_PRIVILEGE_ENABLED, SE_TCB_NAME, SID_AND_ATTRIBUTES, TOKEN_ACCESS_MASK, TOKEN_ADJUST_DEFAULT,
    TOKEN_ADJUST_PRIVILEGES, TOKEN_ADJUST_SESSIONID, TOKEN_ASSIGN_PRIMARY, TOKEN_DUPLICATE,
    TOKEN_ELEVATION_TYPE, TOKEN_GROUPS, TOKEN_LINKED_TOKEN, TOKEN_PRIVILEGES, TOKEN_QUERY,
    TOKEN_SOURCE,
};
use windows::Win32::System::Environment::{CreateEnvironmentBlock, DestroyEnvironmentBlock};
use windows::Win32::System::Threading::{
    CreateProcessAsUserW, GetCurrentProcess, OpenProcessToken, CREATE_NEW_CONSOLE,
    CREATE_UNICODE_ENVIRONMENT, PROCESS_INFORMATION, STARTUPINFOW,
};

use super::util::{wide, Handle};

const SE_GROUP_MANDATORY: u32 = 0x1;
const SE_GROUP_ENABLED_BY_DEFAULT: u32 = 0x2;
const SE_GROUP_ENABLED: u32 = 0x4;
/// Marks the logon SID in a token's groups.
pub const SE_GROUP_LOGON_ID: u32 = 0xC000_0000;

pub struct Started {
    pub pid: u32,
    pub process: Handle,
}

/// The account administrator mode starts the program as.
pub struct User {
    pub name: String,
    pub domain: String,
    /// Logon SID of the caller's session (raw SID bytes).
    pub logon_sid: Vec<u8>,
}

/// A primary copy of `token` bound to `session_id`.
fn primary_for_session(token: HANDLE, session_id: u32) -> Result<Handle> {
    unsafe {
        let access: TOKEN_ACCESS_MASK = TOKEN_QUERY
            | TOKEN_DUPLICATE
            | TOKEN_ASSIGN_PRIMARY
            | TOKEN_ADJUST_DEFAULT
            | TOKEN_ADJUST_SESSIONID;
        let mut primary = HANDLE::default();
        DuplicateTokenEx(
            token,
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

/// Starts `spec` with `token` on `winsta0\default`, with the token's own
/// profile environment (never the caller's environment).
fn spawn(token: &Handle, spec: &CommandSpec) -> Result<Started> {
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

/// Starts `spec` as SYSTEM in `session_id`.
pub fn start_as_system(session_id: u32, spec: &CommandSpec) -> Result<Started> {
    let own = unsafe {
        let mut own = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_DUPLICATE | TOKEN_QUERY, &mut own)?;
        Handle(own)
    };
    spawn(&primary_for_session(own.raw(), session_id)?, spec)
}

/// Starts `spec` in `session_id` as `user`, with administrator rights.
pub fn start_as_admin(session_id: u32, user: &User, spec: &CommandSpec) -> Result<Started> {
    let token = admin_logon(user)?;
    let token = full_token(token)?;
    spawn(&primary_for_session(token.raw(), session_id)?, spec)
}

fn nt(status: NTSTATUS) -> Result<()> {
    if status.0 >= 0 {
        return Ok(());
    }
    let code = unsafe { LsaNtStatusToWinError(status) };
    Err(Error::from(HRESULT::from_win32(code)))
}

fn lsa_string(text: &'static [u8]) -> LSA_STRING {
    LSA_STRING {
        Length: text.len() as u16,
        MaximumLength: text.len() as u16,
        Buffer: PSTR(text.as_ptr() as *mut u8),
    }
}

fn enable_privilege(name: PCWSTR) -> Result<()> {
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY,
            &mut token,
        )?;
        let token = Handle(token);
        let mut luid = LUID::default();
        LookupPrivilegeValueW(None, name, &mut luid)?;
        let wanted = TOKEN_PRIVILEGES {
            PrivilegeCount: 1,
            Privileges: [LUID_AND_ATTRIBUTES {
                Luid: luid,
                Attributes: SE_PRIVILEGE_ENABLED,
            }],
        };
        AdjustTokenPrivileges(token.raw(), false, Some(&wanted), 0, None, None)?;
        // Succeeds without enabling anything when the privilege is missing.
        if GetLastError() == ERROR_NOT_ALL_ASSIGNED {
            return Err(Error::from(HRESULT::from_win32(ERROR_NOT_ALL_ASSIGNED.0)));
        }
        Ok(())
    }
}

/// A trusted LSA connection; deregistered on drop.
struct Lsa(HANDLE);

impl Drop for Lsa {
    fn drop(&mut self) {
        unsafe {
            let _ = LsaDeregisterLogonProcess(self.0);
        }
    }
}

impl Lsa {
    fn connect() -> Result<Lsa> {
        enable_privilege(SE_TCB_NAME)?;
        let name = lsa_string(b"RunAdminGate");
        let mut handle = HANDLE::default();
        let mut mode = 0u32;
        nt(unsafe { LsaRegisterLogonProcess(&name, &mut handle, &mut mode) })?;
        Ok(Lsa(handle))
    }

    fn package(&self, name: &'static [u8]) -> Result<u32> {
        let name = lsa_string(name);
        let mut id = 0u32;
        nt(unsafe { LsaLookupAuthenticationPackage(self.0, &name, &mut id) })?;
        Ok(id)
    }
}

/// An S4U logon structure followed by its two strings, in one buffer, as
/// LsaLogonUser expects. Returns the buffer and its length in bytes.
fn s4u_buffer<T>(
    first: &str,
    second: &str,
    fill: impl FnOnce(&mut T, LSA_UNICODE_STRING, LSA_UNICODE_STRING),
) -> (Vec<u64>, usize) {
    let a: Vec<u16> = first.encode_utf16().collect();
    let b: Vec<u16> = second.encode_utf16().collect();
    let header = std::mem::size_of::<T>();
    let len = header + (a.len() + b.len()) * 2;
    let mut buf = vec![0u64; len.div_ceil(8)];
    unsafe {
        let base = buf.as_mut_ptr() as *mut u8;
        let pa = base.add(header) as *mut u16;
        std::ptr::copy_nonoverlapping(a.as_ptr(), pa, a.len());
        let pb = pa.add(a.len());
        std::ptr::copy_nonoverlapping(b.as_ptr(), pb, b.len());
        let string = |p: *mut u16, n: usize| LSA_UNICODE_STRING {
            Length: (n * 2) as u16,
            MaximumLength: (n * 2) as u16,
            Buffer: PWSTR(p),
        };
        fill(
            &mut *(base as *mut T),
            string(pa, a.len()),
            string(pb, b.len()),
        );
    }
    (buf, len)
}

/// `DOMAIN\name` as a user principal name, for Kerberos S4U.
fn principal_name(user: &User) -> String {
    let sam = HSTRING::from(format!("{}\\{}", user.domain, user.name));
    let mut buf = [0u16; 512];
    let mut len = buf.len() as u32;
    let ok = unsafe {
        TranslateNameW(
            &sam,
            NameSamCompatible,
            NameUserPrincipal,
            PWSTR(buf.as_mut_ptr()),
            &mut len,
        )
    };
    if ok.as_bool() && len > 1 {
        String::from_utf16_lossy(&buf[..len as usize - 1])
    } else {
        format!("{}@{}", user.name, user.domain)
    }
}

/// S4U logon of `user` with Administrators and the session's logon SID added.
fn admin_logon(user: &User) -> Result<Handle> {
    let lsa = Lsa::connect()?;
    let computer = std::env::var("COMPUTERNAME").unwrap_or_default();
    let local = user.domain.eq_ignore_ascii_case(&computer);
    let (package, (submit, submit_len)) = if local {
        (
            lsa.package(b"MICROSOFT_AUTHENTICATION_PACKAGE_V1_0")?,
            s4u_buffer::<MSV1_0_S4U_LOGON>(&user.name, &user.domain, |s, upn, domain| {
                s.MessageType = MsV1_0S4ULogon;
                s.UserPrincipalName = upn;
                s.DomainName = domain;
            }),
        )
    } else {
        (
            lsa.package(b"Kerberos")?,
            s4u_buffer::<KERB_S4U_LOGON>(&principal_name(user), "", |s, upn, realm| {
                s.MessageType = KerbS4ULogon;
                s.ClientUpn = upn;
                s.ClientRealm = realm;
            }),
        )
    };

    let mut admins = [0u8; 68];
    let mut interactive = [0u8; 68];
    let mut logon_sid = user.logon_sid.clone();
    let mut source = TOKEN_SOURCE::default();
    unsafe {
        for (kind, buf) in [
            (WinBuiltinAdministratorsSid, &mut admins),
            (WinInteractiveSid, &mut interactive),
        ] {
            let mut len = buf.len() as u32;
            CreateWellKnownSid(kind, None, PSID(buf.as_mut_ptr().cast()), &mut len)?;
        }
        for (dst, src) in source.SourceName.iter_mut().zip(b"RAGate\0\0") {
            *dst = *src as i8;
        }
        AllocateLocallyUniqueId(&mut source.SourceIdentifier)?;
    }
    let enabled = SE_GROUP_MANDATORY | SE_GROUP_ENABLED | SE_GROUP_ENABLED_BY_DEFAULT;
    let mut extra = vec![
        (PSID(admins.as_mut_ptr().cast()), enabled),
        (
            PSID(logon_sid.as_mut_ptr().cast()),
            enabled | SE_GROUP_LOGON_ID,
        ),
    ];
    let with_admins = token_groups(&extra);
    // Batch and network logons lack INTERACTIVE; add it so the token matches
    // a normal sign-in for ACLs that grant INTERACTIVE.
    extra.push((PSID(interactive.as_mut_ptr().cast()), enabled));
    let with_interactive = token_groups(&extra);

    let origin = lsa_string(b"RunAdminGate");
    // S4U may refuse the interactive type; batch and network are the fallbacks.
    let mut last = Error::empty();
    for (logon_type, groups) in [
        (SECURITY_LOGON_TYPE::Interactive, &with_admins),
        (SECURITY_LOGON_TYPE::Batch, &with_interactive),
        (SECURITY_LOGON_TYPE::Network, &with_interactive),
    ] {
        let mut profile: *mut c_void = std::ptr::null_mut();
        let mut profile_len = 0u32;
        let mut logon_id = LUID::default();
        let mut token = HANDLE::default();
        let mut quotas = QUOTA_LIMITS::default();
        let mut substatus = 0i32;
        let status = unsafe {
            LsaLogonUser(
                lsa.0,
                &origin,
                logon_type,
                package,
                submit.as_ptr().cast(),
                submit_len as u32,
                Some(groups.as_ptr().cast()),
                &source,
                &mut profile,
                &mut profile_len,
                &mut logon_id,
                &mut token,
                &mut quotas,
                &mut substatus,
            )
        };
        if !profile.is_null() {
            unsafe {
                let _ = LsaFreeReturnBuffer(profile);
            }
        }
        match nt(status) {
            Ok(()) => return Ok(Handle(token)),
            Err(err) => last = err,
        }
    }
    Err(last)
}

/// A TOKEN_GROUPS buffer listing `groups`.
fn token_groups(groups: &[(PSID, u32)]) -> Vec<u64> {
    let len = std::mem::size_of::<TOKEN_GROUPS>()
        + groups.len().saturating_sub(1) * std::mem::size_of::<SID_AND_ATTRIBUTES>();
    let mut buf = vec![0u64; len.div_ceil(8)];
    unsafe {
        let list = buf.as_mut_ptr() as *mut TOKEN_GROUPS;
        (*list).GroupCount = groups.len() as u32;
        let entries = (*list).Groups.as_mut_ptr();
        for (i, (sid, attributes)) in groups.iter().enumerate() {
            *entries.add(i) = SID_AND_ATTRIBUTES {
                Sid: *sid,
                Attributes: *attributes,
            };
        }
    }
    buf
}

/// With UAC on, a logon that includes Administrators returns the filtered
/// token; the full one is linked to it.
fn full_token(token: Handle) -> Result<Handle> {
    unsafe {
        let mut kind = TOKEN_ELEVATION_TYPE::default();
        let mut len = 0u32;
        GetTokenInformation(
            token.raw(),
            TokenElevationType,
            Some(&mut kind as *mut TOKEN_ELEVATION_TYPE as *mut c_void),
            std::mem::size_of::<TOKEN_ELEVATION_TYPE>() as u32,
            &mut len,
        )?;
        if kind != TokenElevationTypeLimited {
            return Ok(token);
        }
        let mut linked = TOKEN_LINKED_TOKEN::default();
        GetTokenInformation(
            token.raw(),
            TokenLinkedToken,
            Some(&mut linked as *mut TOKEN_LINKED_TOKEN as *mut c_void),
            std::mem::size_of::<TOKEN_LINKED_TOKEN>() as u32,
            &mut len,
        )?;
        Ok(Handle(linked.LinkedToken))
    }
}
