//! Allow-list and registry model shared by the RunAdminGate tools.
//!
//! All configuration lives under `HKLM\SOFTWARE\RunAdminGate`:
//!
//! * `AllowedPath` (`REG_EXPAND_SZ`): `;`-terminated entries. An entry is an
//!   application path, a folder path ending in `\`, or a wildcard pattern, and
//!   may carry a per-entry logon override suffix (` /assystem` or ` /asadmin`).
//!   Environment variables such as `%windir%` are stored unexpanded.
//! * `LogonFlag` (`REG_EXPAND_SZ`): default logon mode, `asservice` or `asadmin`.
//!
//! Everything here is pure string handling so it can be unit-tested on any host;
//! the Windows binaries do the registry and UI work. See `docs/launcher-design.md`.

use std::fmt;

pub mod cmdline;
pub mod command;
pub mod path;
pub mod policy;
pub mod protocol;

/// Product name, as shown to users.
pub const PRODUCT_NAME: &str = "RunAdminGate";
/// Registry key below `HKEY_LOCAL_MACHINE`.
pub const REG_SUBKEY: &str = r"SOFTWARE\RunAdminGate";
/// Windows service name.
pub const SERVICE_NAME: &str = "RunAdminGate";
/// Value holding the allow-list.
pub const VALUE_ALLOWED_PATH: &str = "AllowedPath";
/// Value holding the default logon mode.
pub const VALUE_LOGON_FLAG: &str = "LogonFlag";
/// Service/launcher executable expected next to the manager.
pub const LAUNCHER_EXE: &str = "runadmingate.exe";
/// Where `runadmingate.exe /install` places the launcher, relative to `%ProgramFiles%`.
pub const INSTALLED_LAUNCHER: &str = r"RunAdminGate\runadmingate.exe";
/// Exit code (`ERROR_INSTALL_FAILURE`) the launcher returns when install/uninstall fails.
pub const EXIT_INSTALL_FAILURE: i32 = 1603;

/// Extensions the shortcut builder accepts as directly launchable.
const LAUNCHABLE_EXTENSIONS: [&str; 8] = [
    ".exe", ".com", ".cmd", ".bat", ".msi", ".msp", ".vbs", ".ps1",
];

/// How the launcher starts an allowed program.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LogonMode {
    /// Under the local SYSTEM account (`asservice` / `/assystem`).
    System,
    /// As the calling user, temporarily added to local Administrators (`asadmin` / `/asadmin`).
    Admin,
}

impl LogonMode {
    /// Value stored in `LogonFlag`.
    pub fn registry_value(self) -> &'static str {
        match self {
            LogonMode::System => "asservice",
            LogonMode::Admin => "asadmin",
        }
    }

    /// Parses a `LogonFlag` value. Matching is exact.
    pub fn from_registry_value(value: &str) -> Option<Self> {
        match value.trim_end_matches('\0') {
            "asservice" => Some(LogonMode::System),
            "asadmin" => Some(LogonMode::Admin),
            _ => None,
        }
    }

    /// Per-entry override switch appended to an `AllowedPath` entry.
    pub fn entry_switch(self) -> &'static str {
        match self {
            LogonMode::System => "/assystem",
            LogonMode::Admin => "/asadmin",
        }
    }
}

impl fmt::Display for LogonMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            LogonMode::System => "system",
            LogonMode::Admin => "administrator",
        })
    }
}

/// Returns the per-entry logon override of an `AllowedPath` entry.
///
/// `/asadmin` wins when both switches are present, matching the edit dialog.
pub fn entry_override(entry: &str) -> Option<LogonMode> {
    if entry.contains(LogonMode::Admin.entry_switch()) {
        Some(LogonMode::Admin)
    } else if entry.contains(LogonMode::System.entry_switch()) {
        Some(LogonMode::System)
    } else {
        None
    }
}

/// Removes any override switch from `entry` and trims it.
pub fn strip_override(entry: &str) -> String {
    entry
        .replace(LogonMode::System.entry_switch(), "")
        .replace(LogonMode::Admin.entry_switch(), "")
        .trim()
        .to_string()
}

/// Replaces the override switch of `entry` with `mode` (`None` = use `LogonFlag`).
pub fn with_override(entry: &str, mode: Option<LogonMode>) -> String {
    let base = strip_override(entry);
    match mode {
        Some(mode) => format!("{base} {}", mode.entry_switch()),
        None => base,
    }
}

/// Normalizes a folder chosen in the folder picker: trimmed, with a trailing `\`.
pub fn folder_entry(path: &str) -> String {
    let path = path.trim();
    if path.ends_with('\\') {
        path.to_string()
    } else {
        format!("{path}\\")
    }
}

/// Normalizes free text typed into the entry dialog.
///
/// Quotes are kept: a quoted program path separates the program from its
/// arguments unambiguously (see [`parse_entry`]).
pub fn clean_manual_entry(text: &str) -> String {
    text.trim().to_string()
}

/// One `AllowedPath` entry split into its parts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedEntry {
    /// Program path, folder or wildcard pattern, without quotes or switches.
    pub path: String,
    /// Arguments the program must be started with (empty = no arguments allowed).
    pub args: String,
    /// Per-entry logon override.
    pub mode: Option<LogonMode>,
    /// The entry names a folder (it ends with `\`).
    pub folder: bool,
}

/// Splits an entry into path, arguments and logon override.
///
/// A leading quoted path is taken verbatim (`"C:\a b\x.exe" /S`). Otherwise
/// the path ends at the first launchable extension followed by whitespace;
/// entries without one are a path only.
pub fn parse_entry(entry: &str) -> ParsedEntry {
    let mode = entry_override(entry);
    let base = strip_override(entry);
    if let Some(quoted) = base.strip_prefix('"') {
        let (path, args) = match quoted.find('"') {
            Some(end) => (&quoted[..end], quoted[end + 1..].trim()),
            None => (quoted, ""),
        };
        return ParsedEntry {
            path: path.trim().to_string(),
            args: args.to_string(),
            mode,
            folder: path.trim_end().ends_with('\\'),
        };
    }
    if base.ends_with('\\') {
        return ParsedEntry {
            path: base,
            args: String::new(),
            mode,
            folder: true,
        };
    }
    let end = program_end(&base).unwrap_or(base.len());
    ParsedEntry {
        path: base[..end].to_string(),
        args: base[end..].trim().to_string(),
        mode,
        folder: false,
    }
}

/// Byte index just past the first launchable extension that is followed by
/// whitespace or the end of `text`.
fn program_end(text: &str) -> Option<usize> {
    let lower = text.to_ascii_lowercase();
    let mut best: Option<usize> = None;
    for ext in LAUNCHABLE_EXTENSIONS {
        let mut from = 0;
        while let Some(pos) = lower[from..].find(ext) {
            let end = from + pos + ext.len();
            if lower[end..]
                .chars()
                .next()
                .map_or(true, char::is_whitespace)
            {
                best = Some(best.map_or(end, |b| b.min(end)));
                break;
            }
            from = end;
        }
    }
    best
}

/// A concrete program an `AllowedPath` entry refers to, for building a shortcut.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch {
    /// Program path, without quotes or override switch.
    pub program: String,
    /// Arguments that follow the program inside the entry, if any.
    pub args: String,
}

impl Launch {
    /// Command-line arguments for the launcher: the quoted program, then its arguments.
    pub fn launcher_args(&self) -> String {
        if self.args.is_empty() {
            format!("\"{}\"", self.program)
        } else {
            format!("\"{}\" {}", self.program, self.args)
        }
    }

    /// Shortcut file name: the program's file stem plus `.lnk`.
    pub fn shortcut_file_name(&self) -> String {
        let file = self
            .program
            .rsplit(['\\', '/'])
            .next()
            .unwrap_or(&self.program);
        let stem = match file.rfind('.') {
            Some(dot) if dot > 0 => &file[..dot],
            _ => file,
        };
        format!("{stem}.lnk")
    }
}

/// Splits an entry into a launchable program and its arguments.
///
/// Returns `None` for folders, wildcard patterns and entries without a
/// launchable extension; those need the user to pick a concrete file.
pub fn launch_target(entry: &str) -> Option<Launch> {
    let parsed = parse_entry(entry);
    if parsed.folder
        || parsed.path.contains(['*', '?'])
        || program_end(&parsed.path) != Some(parsed.path.len())
    {
        return None;
    }
    Some(Launch {
        program: parsed.path,
        args: parsed.args,
    })
}

/// The ordered allow-list stored in `AllowedPath`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AllowedPath {
    entries: Vec<String>,
}

impl AllowedPath {
    /// Parses the raw registry string. Empty segments, doubled `;` and
    /// surrounding whitespace are dropped; trailing NULs are ignored.
    pub fn parse(raw: &str) -> Self {
        AllowedPath {
            entries: split_entries(raw.trim_end_matches('\0')),
        }
    }

    /// Serializes back to the registry form: every entry followed by `;`.
    pub fn to_registry_string(&self) -> String {
        self.entries.iter().map(|e| format!("{e};")).collect()
    }

    pub fn entries(&self) -> &[String] {
        &self.entries
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Index of the first entry equal to `entry`.
    pub fn position(&self, entry: &str) -> Option<usize> {
        self.entries.iter().position(|e| e == entry)
    }

    /// Appends `text`, which may hold several `;`-separated entries.
    /// Returns the index of the last entry added, if any.
    pub fn push(&mut self, text: &str) -> Option<usize> {
        let added = split_entries(text);
        if added.is_empty() {
            return None;
        }
        self.entries.extend(added);
        Some(self.entries.len() - 1)
    }

    /// Replaces the entry at `index` with the entries in `text`.
    /// An empty `text` removes the entry. Returns the index of the last new entry.
    pub fn replace(&mut self, index: usize, text: &str) -> Option<usize> {
        if index >= self.entries.len() {
            return None;
        }
        let added = split_entries(text);
        let count = added.len();
        self.entries.splice(index..=index, added);
        (count > 0).then(|| index + count - 1)
    }

    /// Removes and returns the entry at `index`.
    pub fn remove(&mut self, index: usize) -> Option<String> {
        (index < self.entries.len()).then(|| self.entries.remove(index))
    }
}

fn split_entries(text: &str) -> Vec<String> {
    text.split(';')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_registry_value() {
        let raw = "C:\\Program Files (x86)\\Accounting\\update.exe;C:\\Program Files\\Tools\\ /asadmin;%windir%\\System32\\regedt32.exe;D:\\Installers\\setup*.exe;\\\\server\\updates\\;\\\\server\\software\\ /asadmin;\0";
        let ap = AllowedPath::parse(raw);
        assert_eq!(ap.entries().len(), 6);
        assert_eq!(ap.entries()[2], "%windir%\\System32\\regedt32.exe");
        assert_eq!(ap.to_registry_string(), raw.trim_end_matches('\0'));
    }

    #[test]
    fn parse_normalizes() {
        let ap = AllowedPath::parse("  a;;;b ;  ; c  ");
        assert_eq!(ap.entries(), ["a", "b", "c"]);
        assert_eq!(ap.to_registry_string(), "a;b;c;");
        assert!(AllowedPath::parse("").is_empty());
        assert!(AllowedPath::parse(";;").is_empty());
    }

    #[test]
    fn push_replace_remove() {
        let mut ap = AllowedPath::parse("a;b;");
        assert_eq!(ap.push("c;d"), Some(3));
        assert_eq!(ap.push("  ;"), None);
        assert_eq!(ap.replace(1, "x;y"), Some(2));
        assert_eq!(ap.entries(), ["a", "x", "y", "c", "d"]);
        assert_eq!(ap.replace(0, ""), None);
        assert_eq!(ap.entries(), ["x", "y", "c", "d"]);
        assert_eq!(ap.replace(9, "z"), None);
        assert_eq!(ap.remove(1).as_deref(), Some("y"));
        assert_eq!(ap.remove(9), None);
        assert_eq!(ap.position("c"), Some(1));
    }

    #[test]
    fn logon_flag_values() {
        assert_eq!(
            LogonMode::from_registry_value("asservice"),
            Some(LogonMode::System)
        );
        assert_eq!(
            LogonMode::from_registry_value("asadmin\0"),
            Some(LogonMode::Admin)
        );
        assert_eq!(LogonMode::from_registry_value("AsAdmin"), None);
        assert_eq!(LogonMode::Admin.registry_value(), "asadmin");
    }

    #[test]
    fn overrides_round_trip() {
        let e = "C:\\tools\\ /asadmin";
        assert_eq!(entry_override(e), Some(LogonMode::Admin));
        assert_eq!(
            with_override(e, Some(LogonMode::System)),
            "C:\\tools\\ /assystem"
        );
        assert_eq!(with_override(e, None), "C:\\tools\\");
        assert_eq!(
            entry_override("x /assystem /asadmin"),
            Some(LogonMode::Admin)
        );
        assert_eq!(entry_override("C:\\app.exe"), None);
    }

    #[test]
    fn folder_and_manual_entries() {
        assert_eq!(folder_entry(" C:\\Data "), "C:\\Data\\");
        assert_eq!(folder_entry("\\\\srv\\share\\"), "\\\\srv\\share\\");
        assert_eq!(
            clean_manual_entry(" \"C:\\a b\\x.exe\" "),
            "\"C:\\a b\\x.exe\""
        );
    }

    #[test]
    fn parses_entries() {
        let e = parse_entry("\"C:\\my.exe tools\\app.exe\" /S /asadmin");
        assert_eq!(e.path, "C:\\my.exe tools\\app.exe");
        assert_eq!(e.args, "/S");
        assert_eq!(e.mode, Some(LogonMode::Admin));
        assert!(!e.folder);

        let e = parse_entry("C:\\Program Files\\Common7\\ /asadmin");
        assert_eq!(
            (e.path.as_str(), e.folder),
            ("C:\\Program Files\\Common7\\", true)
        );

        let e = parse_entry("\\\\srv\\upd\\setup*.msi /qn");
        assert_eq!(
            (e.path.as_str(), e.args.as_str()),
            ("\\\\srv\\upd\\setup*.msi", "/qn")
        );

        let e = parse_entry("C:\\Tools\\**");
        assert_eq!(
            (e.path.as_str(), e.args.as_str(), e.folder),
            ("C:\\Tools\\**", "", false)
        );

        let e = parse_entry("\"C:\\unterminated\\x.exe");
        assert_eq!(e.path, "C:\\unterminated\\x.exe");

        let l = launch_target("\"C:\\a b\\x.exe\" -q").unwrap();
        assert_eq!(l.launcher_args(), "\"C:\\a b\\x.exe\" -q");
    }

    #[test]
    fn launch_targets() {
        let l = launch_target("%windir%\\System32\\regedt32.exe").unwrap();
        assert_eq!(l.launcher_args(), "\"%windir%\\System32\\regedt32.exe\"");
        assert_eq!(l.shortcut_file_name(), "regedt32.lnk");

        let l = launch_target("C:\\x\\setup.EXE /S /D=1 /asadmin").unwrap();
        assert_eq!(l.program, "C:\\x\\setup.EXE");
        assert_eq!(l.args, "/S /D=1");
        assert_eq!(l.launcher_args(), "\"C:\\x\\setup.EXE\" /S /D=1");

        let l = launch_target("C:\\my.exe.tools\\run.cmd").unwrap();
        assert_eq!(l.program, "C:\\my.exe.tools\\run.cmd");
        assert_eq!(l.shortcut_file_name(), "run.lnk");

        assert_eq!(launch_target("\\\\server\\updates\\"), None);
        assert_eq!(launch_target("C:\\Program Files\\Common7\\ /asadmin"), None);
        assert_eq!(launch_target("v:\\Download\\setupVers*.exe"), None);
        assert_eq!(launch_target("C:\\readme.txt"), None);
    }
}
