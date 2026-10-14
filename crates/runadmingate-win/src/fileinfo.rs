//! Display names and icon locations of files.

use std::path::Path;

use windows::core::HSTRING;
use windows::Win32::Storage::FileSystem::{
    GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW, FILE_FLAGS_AND_ATTRIBUTES,
};
use windows::Win32::UI::Shell::{SHGetFileInfoW, SHFILEINFOW, SHGFI_ICONLOCATION};

fn query_string(block: &[u8], sub: &str) -> Option<String> {
    unsafe {
        let mut ptr = std::ptr::null_mut();
        let mut len = 0u32;
        if !VerQueryValueW(
            block.as_ptr().cast(),
            &HSTRING::from(sub),
            &mut ptr,
            &mut len,
        )
        .as_bool()
            || len == 0
        {
            return None;
        }
        let chars = std::slice::from_raw_parts(ptr as *const u16, len as usize);
        let text = String::from_utf16_lossy(chars)
            .trim_end_matches('\0')
            .trim()
            .to_string();
        (!text.is_empty()).then_some(text)
    }
}

/// The `FileDescription` of an executable ("Notepad", "Registry Editor").
pub fn description(path: &str) -> Option<String> {
    unsafe {
        let name = HSTRING::from(path);
        let size = GetFileVersionInfoSizeW(&name, None);
        if size == 0 {
            return None;
        }
        let mut block = vec![0u8; size as usize];
        GetFileVersionInfoW(&name, 0, size, block.as_mut_ptr().cast()).ok()?;

        let mut ptr = std::ptr::null_mut();
        let mut len = 0u32;
        let mut languages = vec![(0x0409u16, 0x04B0u16)];
        if VerQueryValueW(
            block.as_ptr().cast(),
            &HSTRING::from(r"\VarFileInfo\Translation"),
            &mut ptr,
            &mut len,
        )
        .as_bool()
        {
            let pairs = std::slice::from_raw_parts(ptr as *const u16, len as usize / 2);
            languages = pairs
                .chunks_exact(2)
                .map(|p| (p[0], p[1]))
                .chain(languages)
                .collect();
        }
        languages.into_iter().find_map(|(lang, cp)| {
            query_string(
                &block,
                &format!(r"\StringFileInfo\{lang:04x}{cp:04x}\FileDescription"),
            )
        })
    }
}

/// A friendly name: the description, else the file name without extension.
pub fn display_name(path: &str) -> String {
    description(path).unwrap_or_else(|| {
        Path::new(path)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string())
    })
}

/// Where Explorer takes the file's icon from (`file`, `index`).
pub fn icon_location(path: &str) -> Option<(String, i32)> {
    unsafe {
        let mut info = SHFILEINFOW::default();
        let ok = SHGetFileInfoW(
            &HSTRING::from(path),
            FILE_FLAGS_AND_ATTRIBUTES(0),
            Some(&mut info),
            std::mem::size_of::<SHFILEINFOW>() as u32,
            SHGFI_ICONLOCATION,
        );
        let file = crate::icon::from_wide(&info.szDisplayName);
        (ok != 0 && !file.is_empty()).then_some((file, info.iIcon))
    }
}
