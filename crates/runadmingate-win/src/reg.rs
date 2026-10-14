//! Thin registry wrapper.

use windows::core::{HSTRING, PCWSTR, PWSTR};
use windows::Win32::Foundation::{
    ERROR_FILE_NOT_FOUND, ERROR_MORE_DATA, ERROR_NO_MORE_ITEMS, ERROR_SUCCESS, WIN32_ERROR,
};
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteTreeW, RegDeleteValueW, RegEnumKeyExW, RegEnumValueW,
    RegOpenKeyExW, RegQueryValueExW, RegSetValueExW, HKEY, REG_CREATED_NEW_KEY,
    REG_CREATE_KEY_DISPOSITION, REG_DWORD, REG_EXPAND_SZ, REG_OPTION_NON_VOLATILE, REG_SAM_FLAGS,
    REG_SZ, REG_VALUE_TYPE,
};

pub use windows::Win32::System::Registry::{
    HKEY_CLASSES_ROOT, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_32KEY, KEY_WOW64_64KEY, KEY_WRITE,
};

pub type RegResult<T> = Result<T, WIN32_ERROR>;

pub struct Key(HKEY);

impl Drop for Key {
    fn drop(&mut self) {
        unsafe {
            let _ = RegCloseKey(self.0);
        }
    }
}

fn check(err: WIN32_ERROR) -> RegResult<()> {
    if err == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(err)
    }
}

fn ok_if_missing(result: RegResult<()>) -> RegResult<()> {
    match result {
        Err(err) if err == ERROR_FILE_NOT_FOUND => Ok(()),
        other => other,
    }
}

impl Key {
    pub fn open(root: HKEY, path: &str, access: REG_SAM_FLAGS) -> RegResult<Key> {
        let mut hkey = HKEY::default();
        check(unsafe { RegOpenKeyExW(root, &HSTRING::from(path), 0, access, &mut hkey) })?;
        Ok(Key(hkey))
    }

    /// Creates or opens `path`; the flag tells whether it was newly created.
    pub fn create(root: HKEY, path: &str, access: REG_SAM_FLAGS) -> RegResult<(Key, bool)> {
        let mut hkey = HKEY::default();
        let mut disposition = REG_CREATE_KEY_DISPOSITION::default();
        check(unsafe {
            RegCreateKeyExW(
                root,
                &HSTRING::from(path),
                0,
                PCWSTR::null(),
                REG_OPTION_NON_VOLATILE,
                access,
                None,
                &mut hkey,
                Some(&mut disposition),
            )
        })?;
        Ok((Key(hkey), disposition == REG_CREATED_NEW_KEY))
    }

    /// Reads a `REG_SZ`/`REG_EXPAND_SZ` value without expanding it.
    /// Missing values and other types read as `None`.
    pub fn read_string(&self, name: &str) -> RegResult<Option<String>> {
        let name = HSTRING::from(name);
        let mut buf: Vec<u16> = Vec::new();
        loop {
            let mut kind = REG_VALUE_TYPE::default();
            let mut size = (buf.len() * 2) as u32;
            let data = if buf.is_empty() {
                None
            } else {
                Some(buf.as_mut_ptr().cast::<u8>())
            };
            let err = unsafe {
                RegQueryValueExW(self.0, &name, None, Some(&mut kind), data, Some(&mut size))
            };
            if err == ERROR_FILE_NOT_FOUND {
                return Ok(None);
            }
            if (err == ERROR_SUCCESS || err == ERROR_MORE_DATA)
                && kind != REG_SZ
                && kind != REG_EXPAND_SZ
            {
                return Ok(None);
            }
            if err == ERROR_MORE_DATA || (err == ERROR_SUCCESS && data.is_none() && size > 0) {
                buf = vec![0; (size as usize).div_ceil(2) + 1];
                continue;
            }
            check(err)?;
            buf.truncate(size as usize / 2);
            while buf.last() == Some(&0) {
                buf.pop();
            }
            return Ok(Some(String::from_utf16_lossy(&buf)));
        }
    }

    fn set_text(&self, name: &str, value: &str, kind: REG_VALUE_TYPE) -> RegResult<()> {
        let data: Vec<u8> = value
            .encode_utf16()
            .chain([0])
            .flat_map(u16::to_le_bytes)
            .collect();
        check(unsafe { RegSetValueExW(self.0, &HSTRING::from(name), 0, kind, Some(&data)) })
    }

    /// `name` "" is the key's default value.
    pub fn set_string(&self, name: &str, value: &str) -> RegResult<()> {
        self.set_text(name, value, REG_SZ)
    }

    pub fn set_expand_string(&self, name: &str, value: &str) -> RegResult<()> {
        self.set_text(name, value, REG_EXPAND_SZ)
    }

    pub fn set_dword(&self, name: &str, value: u32) -> RegResult<()> {
        check(unsafe {
            RegSetValueExW(
                self.0,
                &HSTRING::from(name),
                0,
                REG_DWORD,
                Some(&value.to_le_bytes()),
            )
        })
    }

    pub fn delete_value(&self, name: &str) -> RegResult<()> {
        ok_if_missing(check(unsafe {
            RegDeleteValueW(self.0, &HSTRING::from(name))
        }))
    }

    pub fn subkeys(&self) -> RegResult<Vec<String>> {
        let mut out = Vec::new();
        for index in 0.. {
            let mut name = [0u16; 256];
            let mut len = name.len() as u32;
            let err = unsafe {
                RegEnumKeyExW(
                    self.0,
                    index,
                    PWSTR(name.as_mut_ptr()),
                    &mut len,
                    None,
                    PWSTR::null(),
                    None,
                    None,
                )
            };
            if err == ERROR_NO_MORE_ITEMS {
                break;
            }
            check(err)?;
            out.push(String::from_utf16_lossy(&name[..len as usize]));
        }
        Ok(out)
    }

    pub fn value_names(&self) -> RegResult<Vec<String>> {
        let mut out = Vec::new();
        for index in 0.. {
            let mut name = [0u16; 16384];
            let mut len = name.len() as u32;
            let err = unsafe {
                RegEnumValueW(
                    self.0,
                    index,
                    PWSTR(name.as_mut_ptr()),
                    &mut len,
                    None,
                    None,
                    None,
                    None,
                )
            };
            if err == ERROR_NO_MORE_ITEMS {
                break;
            }
            check(err)?;
            out.push(String::from_utf16_lossy(&name[..len as usize]));
        }
        Ok(out)
    }
}

/// Deletes `path` and everything below it; a missing key is not an error.
pub fn delete_tree(root: HKEY, path: &str) -> RegResult<()> {
    ok_if_missing(check(unsafe { RegDeleteTreeW(root, &HSTRING::from(path)) }))
}

pub fn exists(root: HKEY, path: &str, view: REG_SAM_FLAGS) -> bool {
    Key::open(root, path, KEY_READ | view).is_ok()
}
