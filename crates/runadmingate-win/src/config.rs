//! The allow-list and default mode in `HKLM\SOFTWARE\RunAdminGate` (64-bit view).

use runadmingate_core::{AllowedPath, LogonMode, REG_SUBKEY, VALUE_ALLOWED_PATH, VALUE_LOGON_FLAG};

use crate::reg::{self, Key, RegResult, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_64KEY, KEY_WRITE};

fn read(
    path: &str,
    view: windows::Win32::System::Registry::REG_SAM_FLAGS,
) -> RegResult<(AllowedPath, Option<LogonMode>)> {
    let key = Key::open(HKEY_LOCAL_MACHINE, path, KEY_READ | view)?;
    let allowed = AllowedPath::parse(&key.read_string(VALUE_ALLOWED_PATH)?.unwrap_or_default());
    let mode = key
        .read_string(VALUE_LOGON_FLAG)?
        .as_deref()
        .and_then(LogonMode::from_registry_value);
    Ok((allowed, mode))
}

/// The service counts as installed when its key exists (`/install` creates it).
pub fn is_installed() -> bool {
    reg::exists(HKEY_LOCAL_MACHINE, REG_SUBKEY, KEY_WOW64_64KEY)
}

pub fn load() -> RegResult<(AllowedPath, Option<LogonMode>)> {
    read(REG_SUBKEY, KEY_WOW64_64KEY)
}

/// Read-modify-write of `AllowedPath`. The key must already exist.
pub fn update_allowed<R>(edit: impl FnOnce(&mut AllowedPath) -> R) -> RegResult<R> {
    let key = Key::open(
        HKEY_LOCAL_MACHINE,
        REG_SUBKEY,
        KEY_READ | KEY_WRITE | KEY_WOW64_64KEY,
    )?;
    let mut allowed = AllowedPath::parse(&key.read_string(VALUE_ALLOWED_PATH)?.unwrap_or_default());
    let result = edit(&mut allowed);
    key.set_expand_string(VALUE_ALLOWED_PATH, &allowed.to_registry_string())?;
    Ok(result)
}

pub fn save_logon_mode(mode: LogonMode) -> RegResult<()> {
    let (key, _) = Key::create(HKEY_LOCAL_MACHINE, REG_SUBKEY, KEY_WRITE | KEY_WOW64_64KEY)?;
    key.set_expand_string(VALUE_LOGON_FLAG, mode.registry_value())
}

/// Creates the key if needed, then adds `extra` entries and applies `mode`
/// (SYSTEM when no mode was ever set).
pub fn ensure(extra: &[String], mode: Option<LogonMode>) -> RegResult<()> {
    let (key, _) = Key::create(
        HKEY_LOCAL_MACHINE,
        REG_SUBKEY,
        KEY_READ | KEY_WRITE | KEY_WOW64_64KEY,
    )?;
    let mut allowed = AllowedPath::parse(&key.read_string(VALUE_ALLOWED_PATH)?.unwrap_or_default());
    let current = key
        .read_string(VALUE_LOGON_FLAG)?
        .as_deref()
        .and_then(LogonMode::from_registry_value);
    for entry in extra {
        if allowed.position(entry).is_none() {
            allowed.push(entry);
        }
    }
    key.set_expand_string(VALUE_ALLOWED_PATH, &allowed.to_registry_string())?;
    key.set_expand_string(
        VALUE_LOGON_FLAG,
        mode.or(current)
            .unwrap_or(LogonMode::System)
            .registry_value(),
    )
}

pub fn delete() -> RegResult<()> {
    reg::delete_tree(HKEY_LOCAL_MACHINE, REG_SUBKEY)
}
