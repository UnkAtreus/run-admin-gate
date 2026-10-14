//! Icons: copying an application's icon into another executable, and
//! rendering any file's shell icon as RGBA pixels for the UI.

use std::ffi::c_void;

use windows::core::{Interface, HSTRING, PCWSTR};
use windows::Win32::Foundation::{FreeLibrary, BOOL, HMODULE, SIZE};
use windows::Win32::Graphics::Gdi::{
    DeleteObject, GetDC, GetDIBits, GetObjectW, ReleaseDC, BITMAP, BITMAPINFO, BITMAPINFOHEADER,
    BI_RGB, DIB_RGB_COLORS, HBITMAP, HGDIOBJ,
};
use windows::Win32::System::LibraryLoader::{
    BeginUpdateResourceW, EndUpdateResourceW, EnumResourceNamesW, FindResourceW, LoadLibraryExW,
    LoadResource, LockResource, SizeofResource, UpdateResourceW, LOAD_LIBRARY_AS_DATAFILE,
    LOAD_LIBRARY_AS_IMAGE_RESOURCE,
};
use windows::Win32::UI::Shell::{
    IShellItem, IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_BIGGERSIZEOK,
    SIIGBF_ICONONLY,
};

pub fn from_wide(buf: &[u16]) -> String {
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..len])
}

const RT_ICON: u16 = 3;
const RT_GROUP_ICON: u16 = 14;
const LANG_EN_US: u16 = 0x0409;

fn int_resource(id: u16) -> PCWSTR {
    PCWSTR(id as usize as *const u16)
}

struct Library(HMODULE);

impl Drop for Library {
    fn drop(&mut self) {
        unsafe {
            let _ = FreeLibrary(self.0);
        }
    }
}

unsafe fn resource_bytes(module: HMODULE, name: PCWSTR, kind: u16) -> Option<Vec<u8>> {
    let info = FindResourceW(module, name, int_resource(kind));
    if info.is_invalid() {
        return None;
    }
    let data = LoadResource(module, info).ok()?;
    let ptr = LockResource(data) as *const u8;
    let size = SizeofResource(module, info) as usize;
    (!ptr.is_null() && size > 0).then(|| std::slice::from_raw_parts(ptr, size).to_vec())
}

/// Name of the first `RT_GROUP_ICON` (as Explorer picks it): an integer ID
/// or a string name.
unsafe fn first_group_name(module: HMODULE) -> Option<GroupName> {
    unsafe extern "system" fn callback(_: HMODULE, _: PCWSTR, name: PCWSTR, param: isize) -> BOOL {
        let slot = &mut *(param as *mut Option<GroupName>);
        *slot = Some(if (name.0 as usize) >> 16 == 0 {
            GroupName::Id(name.0 as usize as u16)
        } else {
            GroupName::Text(HSTRING::from_wide(name.as_wide()).unwrap_or_default())
        });
        BOOL::from(false) // stop after the first
    }
    let mut found: Option<GroupName> = None;
    let _ = EnumResourceNamesW(
        module,
        int_resource(RT_GROUP_ICON),
        Some(callback),
        &mut found as *mut _ as isize,
    );
    found
}

enum GroupName {
    Id(u16),
    Text(HSTRING),
}

impl GroupName {
    fn as_pcwstr(&self) -> PCWSTR {
        match self {
            GroupName::Id(id) => int_resource(*id),
            GroupName::Text(text) => PCWSTR(text.as_ptr()),
        }
    }
}

/// Copies the main icon of `source` (exe or dll) into `target` (an exe we own),
/// replacing icon group 1. Returns false if `source` has no icon.
pub fn copy_icon(source: &str, target: &str) -> windows::core::Result<bool> {
    unsafe {
        let module = LoadLibraryExW(
            &HSTRING::from(source),
            None,
            LOAD_LIBRARY_AS_DATAFILE | LOAD_LIBRARY_AS_IMAGE_RESOURCE,
        )?;
        let library = Library(module);
        let Some(group_name) = first_group_name(library.0) else {
            return Ok(false);
        };
        let Some(mut group) = resource_bytes(library.0, group_name.as_pcwstr(), RT_GROUP_ICON)
        else {
            return Ok(false);
        };
        // GRPICONDIR: reserved, type, count (u16 each), then 14-byte entries
        // whose last two bytes are the RT_ICON id.
        if group.len() < 6 {
            return Ok(false);
        }
        let count = u16::from_le_bytes([group[4], group[5]]) as usize;
        if group.len() < 6 + count * 14 {
            return Ok(false);
        }
        let mut images = Vec::with_capacity(count);
        for i in 0..count {
            let at = 6 + i * 14 + 12;
            let id = u16::from_le_bytes([group[at], group[at + 1]]);
            let Some(image) = resource_bytes(library.0, int_resource(id), RT_ICON) else {
                return Ok(false);
            };
            images.push(image);
            // Renumber so the copies don't collide with the target's resources.
            let new_id = (1000 + i) as u16;
            group[at..at + 2].copy_from_slice(&new_id.to_le_bytes());
        }
        drop(library);

        let update = BeginUpdateResourceW(&HSTRING::from(target), BOOL::from(false))?;
        for (i, image) in images.iter().enumerate() {
            UpdateResourceW(
                update,
                int_resource(RT_ICON),
                int_resource((1000 + i) as u16),
                LANG_EN_US,
                Some(image.as_ptr() as *const c_void),
                image.len() as u32,
            )?;
        }
        UpdateResourceW(
            update,
            int_resource(RT_GROUP_ICON),
            int_resource(1),
            LANG_EN_US,
            Some(group.as_ptr() as *const c_void),
            group.len() as u32,
        )?;
        EndUpdateResourceW(update, BOOL::from(false))?;
        Ok(true)
    }
}

/// A square RGBA image of the file's shell icon (straight alpha).
pub struct Rgba {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// Renders the icon Explorer shows for `path` (any file or folder) at `size` px.
pub fn rgba(path: &str, size: i32) -> Option<Rgba> {
    unsafe {
        let item: IShellItem = SHCreateItemFromParsingName(&HSTRING::from(path), None).ok()?;
        let factory: IShellItemImageFactory = item.cast().ok()?;
        let bitmap: HBITMAP = factory
            .GetImage(
                SIZE { cx: size, cy: size },
                SIIGBF_ICONONLY | SIIGBF_BIGGERSIZEOK,
            )
            .ok()?;
        let result = bitmap_to_rgba(bitmap);
        let _ = DeleteObject(HGDIOBJ(bitmap.0));
        result
    }
}

unsafe fn bitmap_to_rgba(bitmap: HBITMAP) -> Option<Rgba> {
    let mut info = BITMAP::default();
    if GetObjectW(
        HGDIOBJ(bitmap.0),
        std::mem::size_of::<BITMAP>() as i32,
        Some(&mut info as *mut _ as *mut c_void),
    ) == 0
    {
        return None;
    }
    let (width, height) = (info.bmWidth, info.bmHeight.abs());
    let mut header = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            biHeight: -height, // top-down
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut pixels = vec![0u8; (width * height * 4) as usize];
    let dc = GetDC(None);
    let lines = GetDIBits(
        dc,
        bitmap,
        0,
        height as u32,
        Some(pixels.as_mut_ptr() as *mut c_void),
        &mut header,
        DIB_RGB_COLORS,
    );
    ReleaseDC(None, dc);
    if lines == 0 {
        return None;
    }
    let has_alpha = pixels.chunks_exact(4).any(|p| p[3] != 0);
    for px in pixels.chunks_exact_mut(4) {
        px.swap(0, 2); // BGRA -> RGBA
        if !has_alpha {
            px[3] = 255;
        } else if px[3] != 0 && px[3] != 255 {
            // The shell hands out premultiplied alpha.
            let a = px[3] as u32;
            for c in &mut px[..3] {
                *c = ((*c as u32 * 255 + a / 2) / a).min(255) as u8;
            }
        }
    }
    Some(Rgba {
        width: width as u32,
        height: height as u32,
        pixels,
    })
}
