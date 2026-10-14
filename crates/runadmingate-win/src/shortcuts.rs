//! Shortcuts the manager created, so `/uninstall` can remove them.

use runadmingate_core::REG_SUBKEY;

use crate::reg::{Key, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_64KEY, KEY_WRITE};

fn key_path() -> String {
    format!(r"{REG_SUBKEY}\Shortcuts")
}

/// Remembers a created `.lnk` (value name = full path).
pub fn record(lnk: &str) {
    if let Ok((key, _)) = Key::create(HKEY_LOCAL_MACHINE, &key_path(), KEY_WRITE | KEY_WOW64_64KEY)
    {
        let _ = key.set_string(lnk, "");
    }
}

pub fn list() -> Vec<String> {
    Key::open(HKEY_LOCAL_MACHINE, &key_path(), KEY_READ | KEY_WOW64_64KEY)
        .and_then(|key| key.value_names())
        .unwrap_or_default()
}

/// Deletes every recorded shortcut file (the record goes with the config key).
pub fn remove_all() {
    for lnk in list() {
        let _ = std::fs::remove_file(lnk);
    }
}
