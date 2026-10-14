//! Allow-list evaluation: may this program, with these arguments, be elevated?
//!
//! This is the security boundary, so the rules are strict and fail closed:
//!
//! * Paths compare case-insensitively after lexical normalization ([`crate::path`]).
//! * A folder entry (`C:\Tools\`) allows programs directly inside that folder,
//!   not in subfolders. Use `C:\Tools\**` to include subfolders.
//! * In path patterns `*` matches within one folder name, `**` matches across
//!   folders and `?` matches one character other than `\`. Patterns must match
//!   the whole path.
//! * The root (`C:` or `\\server\share`) must be literal; relative entries and
//!   entries with wildcards in the root are rejected, so an entry can never
//!   mean "a file with this name anywhere".
//! * Arguments: an entry without arguments allows none. An entry with
//!   arguments must match the requested arguments as a whole; there `*` matches
//!   anything and `?` one character. The argument `%1` means "nothing, or
//!   exactly one absolute file path" (used for "Open with").
//! * `%VAR%` in entries is expanded by the caller-supplied function. The
//!   service passes its own (SYSTEM) environment, never the user's.

use crate::cmdline::split_arg;
use crate::path::{fold_case, normalize};
use crate::{parse_entry, AllowedPath, LogonMode};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tok {
    Lit(char),
    /// `?` in a path: one character other than `\`.
    OneInName,
    /// `?` in arguments: any one character.
    One,
    /// `*` in a path: any run of characters without `\`.
    StarInName,
    /// `**` in a path, `*` in arguments: any run of characters.
    Star,
}

fn path_tokens(pattern: &str) -> Vec<Tok> {
    let chars: Vec<char> = pattern.chars().collect();
    let mut out = Vec::with_capacity(chars.len());
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '*' if chars.get(i + 1) == Some(&'*') => {
                while chars.get(i) == Some(&'*') {
                    i += 1;
                }
                out.push(Tok::Star);
                continue;
            }
            '*' => out.push(Tok::StarInName),
            '?' => out.push(Tok::OneInName),
            c => out.push(Tok::Lit(c)),
        }
        i += 1;
    }
    out
}

fn arg_tokens(pattern: &str) -> Vec<Tok> {
    pattern
        .chars()
        .map(|c| match c {
            '*' => Tok::Star,
            '?' => Tok::One,
            c => Tok::Lit(c),
        })
        .collect()
}

/// Anchored match; `O(tokens × text)`.
fn matches(tokens: &[Tok], text: &[char]) -> bool {
    let n = text.len();
    let mut cur = vec![false; n + 1];
    cur[0] = true;
    for tok in tokens {
        let mut next = vec![false; n + 1];
        match *tok {
            Tok::Lit(c) => {
                for i in 0..n {
                    next[i + 1] = cur[i] && text[i] == c;
                }
            }
            Tok::One => next[1..].copy_from_slice(&cur[..n]),
            Tok::OneInName => {
                for i in 0..n {
                    next[i + 1] = cur[i] && text[i] != '\\';
                }
            }
            Tok::Star | Tok::StarInName => {
                let mut active = false;
                for i in 0..=n {
                    active |= cur[i];
                    next[i] = active;
                    if i < n && *tok == Tok::StarInName && text[i] == '\\' {
                        active = false;
                    }
                }
            }
        }
        if !next.iter().any(|&b| b) {
            return false;
        }
        cur = next;
    }
    cur[n]
}

/// One compiled allow-list entry.
#[derive(Debug, Clone)]
pub struct Rule {
    /// The entry as written in `AllowedPath`.
    pub entry: String,
    /// Per-entry logon override.
    pub mode: Option<LogonMode>,
    path: Vec<Tok>,
    args: ArgsRule,
}

/// Placeholder for "one file path" in an entry's arguments.
pub const FILE_ARGUMENT: &str = "%1";

#[derive(Debug, Clone)]
enum ArgsRule {
    /// No arguments allowed.
    None,
    /// No arguments, or one absolute file path (`%1`).
    File,
    Pattern(Vec<Tok>),
}

impl ArgsRule {
    /// Returns the arguments to pass on, or `None` if `args` doesn't match.
    fn accept(&self, args: &str) -> Option<String> {
        match self {
            ArgsRule::None => args.is_empty().then(String::new),
            ArgsRule::Pattern(tokens) => {
                let text: Vec<char> = fold_case(args).chars().collect();
                matches(tokens, &text).then(|| args.to_string())
            }
            ArgsRule::File => {
                if args.is_empty() {
                    return Some(String::new());
                }
                let (file, rest) = split_arg(args, false)?;
                if !rest.trim().is_empty() {
                    return None;
                }
                let path = normalize(&file)?;
                if path.components.is_empty() {
                    return None;
                }
                // Re-quoted from the normalized path, so the program receives
                // exactly one argument whatever quoting the caller used.
                Some(format!("\"{}\"", path.to_path_string()))
            }
        }
    }
}

/// An entry that was ignored, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleError {
    pub entry: String,
    pub reason: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// `args` are the arguments to start the program with.
    Allow {
        mode: LogonMode,
        entry: String,
        args: String,
    },
    Deny(DenyReason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DenyReason {
    /// The requested program is not an absolute path.
    NotAbsolute,
    /// No entry matches the program and its arguments.
    NotListed,
}

#[derive(Debug, Clone)]
pub struct Policy {
    rules: Vec<Rule>,
    default_mode: LogonMode,
}

impl Policy {
    /// Compiles `AllowedPath`. `default_mode` comes from `LogonFlag`
    /// (`None` means SYSTEM). Invalid entries are
    /// returned separately and never match.
    pub fn compile(
        allowed: &AllowedPath,
        default_mode: Option<LogonMode>,
        expand: impl Fn(&str) -> String,
    ) -> (Policy, Vec<RuleError>) {
        let mut rules = Vec::new();
        let mut errors = Vec::new();
        for entry in allowed.entries() {
            match compile_rule(entry, &expand) {
                Ok(rule) => rules.push(rule),
                Err(reason) => errors.push(RuleError {
                    entry: entry.clone(),
                    reason,
                }),
            }
        }
        let policy = Policy {
            rules,
            default_mode: default_mode.unwrap_or(LogonMode::System),
        };
        (policy, errors)
    }

    pub fn rules(&self) -> &[Rule] {
        &self.rules
    }

    /// Decides a request. `program` should be the canonical path of the file
    /// that will run; `args` is the raw argument string after the program.
    pub fn evaluate(&self, program: &str, args: &str) -> Decision {
        let Some(path) = normalize(program) else {
            return Decision::Deny(DenyReason::NotAbsolute);
        };
        let text: Vec<char> = fold_case(&path.to_path_string()).chars().collect();
        let args = args.trim();
        for rule in &self.rules {
            if !matches(&rule.path, &text) {
                continue;
            }
            if let Some(args) = rule.args.accept(args) {
                return Decision::Allow {
                    mode: rule.mode.unwrap_or(self.default_mode),
                    entry: rule.entry.clone(),
                    args,
                };
            }
        }
        Decision::Deny(DenyReason::NotListed)
    }
}

fn compile_rule(entry: &str, expand: &impl Fn(&str) -> String) -> Result<Rule, &'static str> {
    let parsed = parse_entry(entry);
    let expanded = expand(&parsed.path);
    let path = normalize(&expanded)
        .ok_or("not an absolute path (use C:\\... or \\\\server\\share\\...)")?;
    if path.root.contains(['*', '?']) {
        return Err("wildcards are not allowed in the drive or share name");
    }
    let mut pattern = path.to_path_string();
    if parsed.folder {
        if !path.components.is_empty() {
            pattern.push('\\');
        }
        pattern.push('*');
    } else if path.components.is_empty() {
        return Err("a drive or share root is not a program; end folders with \\");
    }
    Ok(Rule {
        entry: entry.to_string(),
        mode: parsed.mode,
        path: path_tokens(&fold_case(&pattern)),
        args: match parsed.args.trim_matches('"') {
            "" => ArgsRule::None,
            FILE_ARGUMENT => ArgsRule::File,
            _ => ArgsRule::Pattern(arg_tokens(&fold_case(&parsed.args))),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(raw: &str) -> Policy {
        let env = |s: &str| {
            s.replace("%windir%", r"C:\Windows")
                .replace("%ProgramFiles%", r"C:\Program Files")
        };
        let (policy, errors) = Policy::compile(&AllowedPath::parse(raw), None, env);
        assert!(errors.is_empty(), "{errors:?}");
        policy
    }

    fn allowed(p: &Policy, program: &str, args: &str) -> bool {
        matches!(p.evaluate(program, args), Decision::Allow { .. })
    }

    #[test]
    fn exact_program_case_insensitive() {
        let p = policy(r"%windir%\System32\regedt32.exe;");
        assert!(allowed(&p, r"C:\WINDOWS\system32\REGEDT32.EXE", ""));
        assert!(allowed(&p, r"\\?\C:\Windows\System32\regedt32.exe", ""));
        assert!(!allowed(&p, r"C:\Windows\System32\regedt32.exe.bak", ""));
        assert!(!allowed(&p, r"D:\Windows\System32\regedt32.exe", ""));
        assert!(!allowed(&p, r"C:\Windows\System32\cmd.exe", ""));
    }

    #[test]
    fn folder_is_not_recursive_and_respects_boundaries() {
        let p = policy(r"C:\Tools\;");
        assert!(allowed(&p, r"C:\Tools\a.exe", ""));
        assert!(!allowed(&p, r"C:\Tools\sub\a.exe", ""));
        assert!(!allowed(&p, r"C:\ToolsEvil\a.exe", ""));
        assert!(!allowed(&p, r"C:\Tools", ""));
        let p = policy(r"C:\Tools\**;");
        assert!(allowed(&p, r"C:\Tools\sub\deep\a.exe", ""));
        assert!(!allowed(&p, r"C:\ToolsEvil\a.exe", ""));
    }

    #[test]
    fn dot_dot_cannot_escape_an_allowed_folder() {
        let p = policy(r"C:\Tools\;");
        assert!(!allowed(&p, r"C:\Tools\..\Users\me\evil.exe", ""));
        assert!(allowed(&p, r"C:\Tools\x\..\a.exe", ""));
    }

    #[test]
    fn wildcards() {
        let p = policy(r"\\srv\upd\setupVers*.exe;\\srv\apps\*\run?.cmd;");
        assert!(allowed(&p, r"\\SRV\upd\setupVers12.exe", ""));
        assert!(!allowed(
            &p,
            r"\\srv\upd\setupVers12.exe\..\..\other\x.exe",
            ""
        ));
        assert!(!allowed(&p, r"\\srv\upd\sub\setupVers1.exe", ""));
        assert!(!allowed(&p, r"\\srv\upd\setupVers1.exe.cmd", ""));
        assert!(allowed(&p, r"\\srv\apps\word\run1.cmd", ""));
        assert!(!allowed(&p, r"\\srv\apps\a\b\run1.cmd", ""));
        assert!(!allowed(&p, r"\\srv\apps\word\run12.cmd", ""));
    }

    #[test]
    fn arguments() {
        let p = policy(r#""C:\Inst\setup.exe" /S /D=*;C:\Tools\app.exe;"#);
        assert!(allowed(&p, r"C:\Inst\setup.exe", "/s /d=C:\\x"));
        assert!(!allowed(&p, r"C:\Inst\setup.exe", ""));
        assert!(!allowed(&p, r"C:\Inst\setup.exe", "/S"));
        assert!(allowed(&p, r"C:\Tools\app.exe", "   "));
        assert!(!allowed(&p, r"C:\Tools\app.exe", "--evil"));
    }

    #[test]
    fn modes() {
        let raw = r"C:\a.exe /asadmin;C:\b.exe;";
        let (p, _) = Policy::compile(&AllowedPath::parse(raw), Some(LogonMode::Admin), |s| {
            s.to_string()
        });
        assert_eq!(
            p.evaluate(r"C:\a.exe", ""),
            Decision::Allow {
                mode: LogonMode::Admin,
                entry: r"C:\a.exe /asadmin".into(),
                args: String::new()
            }
        );
        let (p, _) = Policy::compile(
            &AllowedPath::parse(r"C:\b.exe /assystem;C:\c.exe"),
            Some(LogonMode::Admin),
            |s| s.to_string(),
        );
        assert!(matches!(
            p.evaluate(r"C:\b.exe", ""),
            Decision::Allow {
                mode: LogonMode::System,
                ..
            }
        ));
        assert!(matches!(
            p.evaluate(r"C:\c.exe", ""),
            Decision::Allow {
                mode: LogonMode::Admin,
                ..
            }
        ));
        let (p, _) = Policy::compile(&AllowedPath::parse(r"C:\c.exe"), None, |s| s.to_string());
        assert!(matches!(
            p.evaluate(r"C:\c.exe", ""),
            Decision::Allow {
                mode: LogonMode::System,
                ..
            }
        ));
    }

    #[test]
    fn rejects_unsafe_entries() {
        let raw = r"*\updates\tool*.exe;regedt32.exe;\\*\share\x.exe;C:\;%UNDEFINED%\x.exe;";
        let (p, errors) = Policy::compile(&AllowedPath::parse(raw), None, |s| s.to_string());
        let rejected: Vec<&str> = errors.iter().map(|e| e.entry.as_str()).collect();
        assert_eq!(
            rejected,
            [
                r"*\updates\tool*.exe",
                "regedt32.exe",
                r"\\*\share\x.exe",
                r"%UNDEFINED%\x.exe"
            ]
        );
        assert_eq!(p.rules().len(), 1);
        assert!(allowed(&p, r"C:\x.exe", ""));
        assert!(!allowed(&p, r"C:\sub\x.exe", ""));
    }

    #[test]
    fn file_argument() {
        let p = policy(r#"C:\Windows\System32\notepad.exe %1;"C:\Tools\view.cmd" "%1";"#);
        let args = |program: &str, args: &str| match p.evaluate(program, args) {
            Decision::Allow { args, .. } => Some(args),
            Decision::Deny(_) => None,
        };
        let np = r"C:\Windows\System32\notepad.exe";
        assert_eq!(args(np, "").as_deref(), Some(""));
        assert_eq!(
            args(np, r#""C:\Users\me\a b.txt""#).as_deref(),
            Some(r#""C:\Users\me\a b.txt""#)
        );
        assert_eq!(
            args(np, r"C:\x\..\log.txt").as_deref(),
            Some(r#""C:\log.txt""#)
        );
        assert_eq!(
            args(np, r#""\\srv\s\f.ini""#).as_deref(),
            Some(r#""\\srv\s\f.ini""#)
        );
        // Anything but one absolute path is refused.
        assert_eq!(args(np, "/A"), None);
        assert_eq!(args(np, r#""C:\a.txt" /S"#), None);
        assert_eq!(args(np, "a.txt"), None);
        assert_eq!(args(np, r#""C:\""#), None);
        assert!(args(r"C:\Tools\view.cmd", r#""C:\d\r.log""#).is_some());
    }

    #[test]
    fn denies_relative_requests() {
        let p = policy(r"C:\Tools\;");
        assert_eq!(
            p.evaluate("a.exe", ""),
            Decision::Deny(DenyReason::NotAbsolute)
        );
        assert_eq!(
            p.evaluate(r"C:\Other\a.exe", ""),
            Decision::Deny(DenyReason::NotListed)
        );
    }

    #[test]
    fn glob_engine() {
        let t = |p: &str, s: &str| matches(&path_tokens(p), &s.chars().collect::<Vec<_>>());
        assert!(t("A*B", "AB"));
        assert!(t("A*B", "AXXB"));
        assert!(!t("A*B", "AX\\B"));
        assert!(t("A**B", "AX\\B"));
        assert!(t("A***B", "AX\\Y\\B"));
        assert!(!t("A?B", "A\\B"));
        assert!(t("*", ""));
        assert!(!t("A", ""));
        assert!(t("**\\X", "A\\B\\X"));
    }
}
