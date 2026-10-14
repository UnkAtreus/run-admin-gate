//! "Open with" entries that start a program through RunAdminGate.
//!
//! Explorer merges Open-with handlers that share an executable and always
//! shows the executable's own icon. So every app gets its own copy of the
//! launcher in `%ProgramFiles%\RunAdminGate\apps\<id>.exe`, carrying the app's
//! icon. When run under that name the launcher looks the target up in
//! `HKLM\SOFTWARE\RunAdminGate\OpenWith\<id>` (writable by administrators
//! only) and sends a normal request to the service, which still checks the
//! allow-list.
//!
//! Registered per app:
//! * `HKLM\SOFTWARE\Classes\RunAdminGate.<id>`: ProgID with name, icon and
//!   `shell\open\command = "<stub>" "%1"`.
//! * `HKLM\SOFTWARE\Classes\.<ext>\OpenWithProgids\RunAdminGate.<id>` per file type.
//! * `HKLM\SOFTWARE\Classes\Applications\<id>.exe`: name for "Choose another app".

use std::path::{Path, PathBuf};

use runadmingate_core::REG_SUBKEY;

use crate::reg::{
    self, Key, HKEY_CLASSES_ROOT, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_64KEY, KEY_WRITE,
};
use crate::{fileinfo, icon, shell};

const CLASSES: &str = r"SOFTWARE\Classes";

fn map_key() -> String {
    format!(r"{REG_SUBKEY}\OpenWith")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenWithApp {
    /// Stub file stem, e.g. `notepad-admin`.
    pub id: String,
    pub program: String,
    /// Shown in the Open with menu, e.g. "Notepad (admin)".
    pub name: String,
    /// Lowercase, with dot: `.txt`.
    pub extensions: Vec<String>,
}

impl OpenWithApp {
    pub fn prog_id(&self) -> String {
        format!("RunAdminGate.{}", self.id)
    }
}

pub fn stub_dir() -> Option<PathBuf> {
    Some(shell::program_files()?.join("RunAdminGate").join("apps"))
}

pub fn stub_path(id: &str) -> Option<PathBuf> {
    Some(stub_dir()?.join(format!("{id}.exe")))
}

/// Normalizes ".TXT", "txt", " *.log " to ".txt" / ".log"; drops invalid ones.
pub fn normalize_extensions(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for part in text.split([',', ';', ' ']) {
        let ext = part
            .trim()
            .trim_start_matches('*')
            .trim_start_matches('.')
            .to_ascii_lowercase();
        if ext.is_empty()
            || !ext
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            continue;
        }
        let ext = format!(".{ext}");
        if !out.contains(&ext) {
            out.push(ext);
        }
    }
    out
}

pub fn list() -> Vec<OpenWithApp> {
    let Ok(key) = Key::open(HKEY_LOCAL_MACHINE, &map_key(), KEY_READ | KEY_WOW64_64KEY) else {
        return Vec::new();
    };
    key.subkeys()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|id| lookup(&id))
        .collect()
}

pub fn lookup(id: &str) -> Option<OpenWithApp> {
    let key = Key::open(
        HKEY_LOCAL_MACHINE,
        &format!(r"{}\{id}", map_key()),
        KEY_READ | KEY_WOW64_64KEY,
    )
    .ok()?;
    Some(OpenWithApp {
        id: id.to_string(),
        program: key.read_string("Program").ok()??,
        name: key.read_string("Name").ok()?.unwrap_or_default(),
        extensions: normalize_extensions(&key.read_string("Extensions").ok()?.unwrap_or_default()),
    })
}

pub fn find_by_program(program: &str) -> Option<OpenWithApp> {
    list()
        .into_iter()
        .find(|app| app.program.eq_ignore_ascii_case(program))
}

fn new_id(program: &str) -> String {
    let stem: String = Path::new(program)
        .file_stem()
        .map(|s| s.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let stem = if stem.trim_matches('-').is_empty() {
        "app".to_string()
    } else {
        stem
    };
    let taken: Vec<String> = list().into_iter().map(|a| a.id).collect();
    let mut id = format!("{stem}-admin");
    let mut n = 2;
    while taken.contains(&id) {
        id = format!("{stem}-admin-{n}");
        n += 1;
    }
    id
}

fn err(context: &str, e: impl std::fmt::Display) -> String {
    format!("{context}: {e}")
}

fn reg_err(context: &str, e: windows::Win32::Foundation::WIN32_ERROR) -> String {
    format!(
        "{context}: {}",
        std::io::Error::from_raw_os_error(e.0 as i32)
    )
}

/// Copies `launcher` to the app's stub and gives it the app's icon.
fn write_stub(launcher: &Path, app: &OpenWithApp) -> Result<PathBuf, String> {
    let stub = stub_path(&app.id).ok_or("Program Files folder not found")?;
    std::fs::create_dir_all(stub.parent().unwrap())
        .map_err(|e| err("Can't create the apps folder", e))?;
    std::fs::copy(launcher, &stub).map_err(|e| err("Can't copy the launcher", e))?;
    let (icon_file, _) = fileinfo::icon_location(&app.program).unwrap_or((app.program.clone(), 0));
    // Without an icon the stub keeps the launcher's; not worth failing over.
    let _ = icon::copy_icon(&icon_file, &stub.to_string_lossy());
    Ok(stub)
}

/// Adds (or updates) the Open-with entry for `program` on `extensions`.
/// `launcher` is the installed `runadmingate.exe`.
pub fn register(
    launcher: &Path,
    program: &str,
    extensions: &[String],
) -> Result<OpenWithApp, String> {
    let existing = find_by_program(program);
    if let Some(old) = &existing {
        remove_registration(old);
    }
    let app = OpenWithApp {
        id: existing.map(|a| a.id).unwrap_or_else(|| new_id(program)),
        program: program.to_string(),
        name: format!("{} (admin)", fileinfo::display_name(program)),
        extensions: extensions.to_vec(),
    };
    let stub = write_stub(launcher, &app)?;
    let stub = stub.to_string_lossy();
    let access = KEY_WRITE | KEY_WOW64_64KEY;

    let (map, _) = Key::create(
        HKEY_LOCAL_MACHINE,
        &format!(r"{}\{}", map_key(), app.id),
        access,
    )
    .map_err(|e| reg_err("Can't save the Open with entry", e))?;
    for (name, value) in [
        ("Program", app.program.as_str()),
        ("Name", &app.name),
        ("Extensions", &app.extensions.join(";")),
    ] {
        map.set_string(name, value)
            .map_err(|e| reg_err("Can't save the Open with entry", e))?;
    }

    let command = format!("\"{stub}\" \"%1\"");
    let icon = format!("\"{}\",0", app.program);
    let prog = format!(r"{CLASSES}\{}", app.prog_id());
    let application = format!(r"{CLASSES}\Applications\{}.exe", app.id);
    let write = |path: &str, values: &[(&str, &str)]| -> Result<(), String> {
        let (key, _) = Key::create(HKEY_LOCAL_MACHINE, path, access)
            .map_err(|e| reg_err("Can't register the file types", e))?;
        for (name, value) in values {
            key.set_string(name, value)
                .map_err(|e| reg_err("Can't register the file types", e))?;
        }
        Ok(())
    };
    write(&prog, &[("", &app.name), ("FriendlyAppName", &app.name)])?;
    write(&format!(r"{prog}\DefaultIcon"), &[("", &icon)])?;
    write(
        &format!(r"{prog}\shell\open"),
        &[("FriendlyAppName", &app.name)],
    )?;
    write(&format!(r"{prog}\shell\open\command"), &[("", &command)])?;
    write(&application, &[("FriendlyAppName", &app.name)])?;
    write(
        &format!(r"{application}\shell\open\command"),
        &[("", &command)],
    )?;
    for ext in &app.extensions {
        write(
            &format!(r"{CLASSES}\{ext}\OpenWithProgids"),
            &[(&app.prog_id(), "")],
        )?;
        write(&format!(r"{application}\SupportedTypes"), &[(ext, "")])?;
    }
    shell::notify_associations_changed();
    Ok(app)
}

fn remove_registration(app: &OpenWithApp) {
    for ext in &app.extensions {
        if let Ok(key) = Key::open(
            HKEY_LOCAL_MACHINE,
            &format!(r"{CLASSES}\{ext}\OpenWithProgids"),
            KEY_WRITE | KEY_WOW64_64KEY,
        ) {
            let _ = key.delete_value(&app.prog_id());
        }
    }
    let _ = reg::delete_tree(HKEY_LOCAL_MACHINE, &format!(r"{CLASSES}\{}", app.prog_id()));
    let _ = reg::delete_tree(
        HKEY_LOCAL_MACHINE,
        &format!(r"{CLASSES}\Applications\{}.exe", app.id),
    );
    let _ = reg::delete_tree(HKEY_LOCAL_MACHINE, &format!(r"{}\{}", map_key(), app.id));
}

/// Removes one entry and its stub.
pub fn unregister(id: &str) {
    if let Some(app) = lookup(id) {
        remove_registration(&app);
    }
    if let Some(stub) = stub_path(id) {
        let _ = std::fs::remove_file(stub);
    }
    shell::notify_associations_changed();
}

/// Removes every entry and the apps folder (used by `/uninstall`).
pub fn remove_all() {
    for app in list() {
        remove_registration(&app);
    }
    if let Some(dir) = stub_dir() {
        let _ = std::fs::remove_dir_all(dir);
    }
    shell::notify_associations_changed();
}

/// Rewrites every stub from `launcher`, e.g. after an upgrade.
pub fn refresh_stubs(launcher: &Path) {
    for app in list() {
        let _ = write_stub(launcher, &app);
    }
}

/// File types the program already handles, from its own registration and
/// from the ProgIDs whose open command runs it.
pub fn suggest_extensions(program: &str) -> Vec<String> {
    let exe = Path::new(program)
        .file_name()
        .map(|f| f.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    if exe.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    if let Ok(key) = Key::open(
        HKEY_CLASSES_ROOT,
        &format!(r"Applications\{exe}\SupportedTypes"),
        KEY_READ,
    ) {
        out.extend(key.value_names().unwrap_or_default());
    }
    if let Ok(root) = Key::open(HKEY_CLASSES_ROOT, "", KEY_READ) {
        for ext in root
            .subkeys()
            .unwrap_or_default()
            .into_iter()
            .filter(|k| k.starts_with('.'))
        {
            let Ok(key) = Key::open(HKEY_CLASSES_ROOT, &ext, KEY_READ) else {
                continue;
            };
            let Some(prog_id) = key.read_string("").ok().flatten().filter(|p| !p.is_empty()) else {
                continue;
            };
            let command = Key::open(
                HKEY_CLASSES_ROOT,
                &format!(r"{prog_id}\shell\open\command"),
                KEY_READ,
            )
            .ok()
            .and_then(|k| k.read_string("").ok().flatten())
            .unwrap_or_default()
            .to_ascii_lowercase();
            if command.contains(&exe) && !command.contains("runadmingate") {
                out.push(ext);
            }
        }
    }
    normalize_extensions(&out.join(";"))
}

#[cfg(test)]
mod tests {
    use super::normalize_extensions;

    #[test]
    fn extensions() {
        assert_eq!(
            normalize_extensions(" .TXT, log;*.ini  bad/ext ..md"),
            [".txt", ".log", ".ini", ".md"]
        );
    }
}
