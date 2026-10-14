//! What the list shows: one `Item` per `AllowedPath` entry.

use std::path::Path;

use runadmingate_core::command::ProgramKind;
use runadmingate_core::policy::{Policy, FILE_ARGUMENT};
use runadmingate_core::{parse_entry, AllowedPath, LogonMode, ParsedEntry};
use runadmingate_win::fileinfo;
use runadmingate_win::openwith::{self, OpenWithApp};

use super::platform::expand;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Program,
    Script,
    Folder { recursive: bool },
    Pattern,
}

/// Which arguments an entry allows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Args {
    None,
    File,
    Custom,
}

#[derive(Debug, Clone)]
pub struct Item {
    /// The entry exactly as stored; identifies the item across reloads.
    pub raw: String,
    pub parsed: ParsedEntry,
    pub kind: Kind,
    /// Program or folder path with variables expanded.
    pub path: String,
    pub name: String,
    pub issue: Option<String>,
    pub open_with: Option<OpenWithApp>,
}

impl Item {
    pub fn args(&self) -> Args {
        match self.parsed.args.trim_matches('"') {
            "" => Args::None,
            FILE_ARGUMENT => Args::File,
            _ => Args::Custom,
        }
    }

    /// The path shown under the name.
    pub fn subtitle(&self) -> String {
        match self.kind {
            Kind::Folder { recursive: true } => format!("{} (and subfolders)", self.path),
            Kind::Pattern => self.parsed.path.clone(),
            _ => self.path.clone(),
        }
    }

    pub fn is_program(&self) -> bool {
        matches!(self.kind, Kind::Program | Kind::Script)
    }

    pub fn matches(&self, search: &str) -> bool {
        let search = search.trim().to_lowercase();
        search.is_empty()
            || self.name.to_lowercase().contains(&search)
            || self.raw.to_lowercase().contains(&search)
    }
}

fn folder_parts(path: &str) -> Option<(String, bool)> {
    if let Some(base) = path.strip_suffix(r"\**") {
        return Some((format!("{base}\\"), true));
    }
    path.ends_with('\\').then(|| (path.to_string(), false))
}

pub fn build(allowed: &AllowedPath, default_mode: Option<LogonMode>) -> Vec<Item> {
    let (_, errors) = Policy::compile(allowed, default_mode, expand);
    let open_with = openwith::list();
    allowed
        .entries()
        .iter()
        .map(|raw| {
            let parsed = parse_entry(raw);
            let expanded = expand(&parsed.path);
            let (kind, path) = if let Some((folder, recursive)) = folder_parts(&expanded) {
                (Kind::Folder { recursive }, folder)
            } else if expanded.contains(['*', '?']) {
                (Kind::Pattern, expanded.clone())
            } else {
                let kind = match ProgramKind::of(&expanded) {
                    Some(ProgramKind::Executable) | None => Kind::Program,
                    Some(_) => Kind::Script,
                };
                (kind, expanded.clone())
            };
            let name = match kind {
                Kind::Program | Kind::Script => fileinfo::display_name(&path),
                Kind::Folder { .. } => Path::new(path.trim_end_matches('\\'))
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.clone()),
                Kind::Pattern => path
                    .rsplit('\\')
                    .find(|part| !part.is_empty())
                    .unwrap_or("Rule")
                    .to_string(),
            };
            let issue = errors
                .iter()
                .find(|e| &e.entry == raw)
                .map(|e| e.reason.to_string());
            let missing =
                matches!(kind, Kind::Program | Kind::Script) && !Path::new(&path).exists();
            let issue = issue.or_else(|| {
                missing.then(|| "The file doesn't exist on this computer.".to_string())
            });
            let open_with = open_with
                .iter()
                .find(|app| app.program.eq_ignore_ascii_case(&path))
                .cloned();
            Item {
                raw: raw.clone(),
                parsed,
                kind,
                path,
                name,
                issue,
                open_with,
            }
        })
        .collect()
}

/// Writes an entry: the path, optional arguments and logon switch.
pub fn entry_text(path: &str, args: &str, mode: Option<LogonMode>) -> String {
    let mut text = if args.is_empty() {
        path.to_string()
    } else {
        format!("\"{path}\" {args}")
    };
    if let Some(mode) = mode {
        text.push(' ');
        text.push_str(mode.entry_switch());
    }
    text
}

/// Program entries keep their original (unexpanded) path text when rewritten.
pub fn with_args(item: &Item, args: &str) -> String {
    entry_text(&item.parsed.path, args, item.parsed.mode)
}

pub fn with_recursive(item: &Item, recursive: bool) -> String {
    let base = item
        .parsed
        .path
        .trim_end_matches("**")
        .trim_end_matches('\\');
    let path = if recursive {
        format!("{base}\\**")
    } else {
        format!("{base}\\")
    };
    entry_text(&path, "", item.parsed.mode)
}
