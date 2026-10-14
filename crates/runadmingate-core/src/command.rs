//! Builds the exact process to start for an allowed program.
//!
//! Scripts and installers need a host program. The host is always taken from
//! the system directory the service passes in, never from `PATH`, and the
//! command line is built here so the service and its tests share one
//! implementation.

use crate::path::normalize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgramKind {
    Executable,
    Batch,
    PowerShell,
    VbScript,
    MsiPackage,
    MsiPatch,
}

impl ProgramKind {
    pub fn of(program: &str) -> Option<ProgramKind> {
        let lower = program.to_ascii_lowercase();
        let ext = lower.rsplit_once('.').map(|(_, ext)| ext)?;
        Some(match ext {
            "exe" | "com" => ProgramKind::Executable,
            "bat" | "cmd" => ProgramKind::Batch,
            "ps1" => ProgramKind::PowerShell,
            "vbs" => ProgramKind::VbScript,
            "msi" => ProgramKind::MsiPackage,
            "msp" => ProgramKind::MsiPatch,
            _ => return None,
        })
    }
}

/// Process to create: `CreateProcess(application, command_line, current_dir)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandSpec {
    pub kind: ProgramKind,
    pub application: String,
    pub command_line: String,
    pub current_dir: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildError {
    /// The program path is not absolute or has no folder.
    BadPath,
    /// The file type cannot be started.
    UnsupportedType,
    /// The path or arguments contain characters that `cmd.exe` would interpret.
    UnsafeForCmd,
    /// The arguments contain a script host option (`//...`).
    UnsafeHostOption,
}

/// Characters `cmd.exe` interprets even inside a batch argument list.
const CMD_SPECIAL: [char; 10] = ['&', '|', '<', '>', '^', '%', '!', '"', '\r', '\n'];

/// Builds the command for `program` (canonical path) with raw `args`.
/// `system_dir` is `GetSystemDirectoryW()`, e.g. `C:\Windows\System32`.
pub fn build(system_dir: &str, program: &str, args: &str) -> Result<CommandSpec, BuildError> {
    let path = normalize(program).ok_or(BuildError::BadPath)?;
    let current_dir = path.parent().ok_or(BuildError::BadPath)?.to_path_string();
    let program = path.to_path_string();
    let kind = ProgramKind::of(&program).ok_or(BuildError::UnsupportedType)?;
    let args = args.trim();
    let tail = |prefix: String| {
        if args.is_empty() {
            prefix
        } else {
            format!("{prefix} {args}")
        }
    };
    let host = |name: &str| format!(r"{}\{name}", system_dir.trim_end_matches('\\'));

    let (application, command_line) = match kind {
        ProgramKind::Executable => (program.clone(), tail(format!("\"{program}\""))),
        ProgramKind::Batch => {
            // One quoted path (what a `%1` entry passes) is fine if the path
            // itself has nothing cmd.exe would interpret.
            let quoted_path = args
                .strip_prefix('"')
                .and_then(|a| a.strip_suffix('"'))
                .filter(|inner| !inner.contains(CMD_SPECIAL));
            if program.contains(CMD_SPECIAL)
                || (quoted_path.is_none() && args.contains(CMD_SPECIAL))
            {
                return Err(BuildError::UnsafeForCmd);
            }
            let cmd = host("cmd.exe");
            // /c strips the outer quotes: cmd sees "program" args.
            let inner = tail(format!("\"{program}\""));
            (cmd.clone(), format!("\"{cmd}\" /d /v:off /c \"{inner}\""))
        }
        ProgramKind::PowerShell => {
            let ps = host(r"WindowsPowerShell\v1.0\powershell.exe");
            let prefix =
                format!("\"{ps}\" -NoLogo -NoProfile -ExecutionPolicy Bypass -File \"{program}\"");
            (ps, tail(prefix))
        }
        ProgramKind::VbScript => {
            if args.split_whitespace().any(|a| a.starts_with("//")) {
                return Err(BuildError::UnsafeHostOption);
            }
            let ws = host("wscript.exe");
            (ws.clone(), tail(format!("\"{ws}\" //NoLogo \"{program}\"")))
        }
        ProgramKind::MsiPackage | ProgramKind::MsiPatch => {
            let msi = host("msiexec.exe");
            let action = if kind == ProgramKind::MsiPackage {
                "/i"
            } else {
                "/p"
            };
            (
                msi.clone(),
                tail(format!("\"{msi}\" {action} \"{program}\"")),
            )
        }
    };
    Ok(CommandSpec {
        kind,
        application,
        command_line,
        current_dir,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SYS: &str = r"C:\Windows\System32";

    #[test]
    fn executables() {
        let c = build(SYS, r"C:\Program Files\App\app.EXE", " -x ").unwrap();
        assert_eq!(c.application, r"C:\Program Files\App\app.EXE");
        assert_eq!(c.command_line, r#""C:\Program Files\App\app.EXE" -x"#);
        assert_eq!(c.current_dir, r"C:\Program Files\App");
    }

    #[test]
    fn scripts_use_system_hosts() {
        let c = build(SYS, r"\\srv\s\fix (x86).cmd", "").unwrap();
        assert_eq!(c.application, r"C:\Windows\System32\cmd.exe");
        assert_eq!(
            c.command_line,
            r#""C:\Windows\System32\cmd.exe" /d /v:off /c ""\\srv\s\fix (x86).cmd"""#
        );
        let c = build(SYS, r"C:\s\a.ps1", "-Name x").unwrap();
        assert!(c
            .command_line
            .ends_with(r#"-NoProfile -ExecutionPolicy Bypass -File "C:\s\a.ps1" -Name x"#));
        assert_eq!(
            c.application,
            r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe"
        );
        let c = build(SYS, r"C:\s\setup.msi", "/qn").unwrap();
        assert_eq!(
            c.command_line,
            r#""C:\Windows\System32\msiexec.exe" /i "C:\s\setup.msi" /qn"#
        );
        let c = build(SYS, r"C:\s\fix.msp", "").unwrap();
        assert_eq!(
            c.command_line,
            r#""C:\Windows\System32\msiexec.exe" /p "C:\s\fix.msp""#
        );
        let c = build(SYS, r"C:\s\a.vbs", "").unwrap();
        assert_eq!(
            c.command_line,
            r#""C:\Windows\System32\wscript.exe" //NoLogo "C:\s\a.vbs""#
        );
    }

    #[test]
    fn batch_with_one_quoted_path() {
        let c = build(SYS, r"C:\s\view.cmd", r#""C:\Users\me\a b.log""#).unwrap();
        assert_eq!(
            c.command_line,
            r#""C:\Windows\System32\cmd.exe" /d /v:off /c ""C:\s\view.cmd" "C:\Users\me\a b.log"""#
        );
        assert_eq!(
            build(SYS, r"C:\s\view.cmd", r#""C:\a&b.log""#),
            Err(BuildError::UnsafeForCmd)
        );
        assert_eq!(
            build(SYS, r"C:\s\view.cmd", r#""C:\%TEMP%\x.log""#),
            Err(BuildError::UnsafeForCmd)
        );
    }

    #[test]
    fn rejects_cmd_injection() {
        for args in [
            "& calc", "a | b", "%PATH%", "!x!", "a\"b", "x > y", "x ^& y",
        ] {
            assert_eq!(
                build(SYS, r"C:\s\a.bat", args),
                Err(BuildError::UnsafeForCmd),
                "{args}"
            );
        }
        assert_eq!(
            build(SYS, r"C:\s\a&b.bat", ""),
            Err(BuildError::UnsafeForCmd)
        );
        assert_eq!(
            build(SYS, r"C:\s\a.vbs", "//E:jscript"),
            Err(BuildError::UnsafeHostOption)
        );
    }

    #[test]
    fn rejects_other_types_and_bad_paths() {
        assert_eq!(
            build(SYS, r"C:\s\a.txt", ""),
            Err(BuildError::UnsupportedType)
        );
        assert_eq!(
            build(SYS, r"C:\s\noext", ""),
            Err(BuildError::UnsupportedType)
        );
        assert_eq!(build(SYS, "a.exe", ""), Err(BuildError::BadPath));
        assert_eq!(build(SYS, r"C:\", ""), Err(BuildError::BadPath));
    }
}
