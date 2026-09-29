//! Shell file icons via SHGetFileInfoW. Jumbo image list when the shell has one.

use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use windows::core::PCWSTR;
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, DeleteDC, DeleteObject, GetDIBits, GetObjectW, BITMAP, BITMAPINFO,
    BITMAPINFOHEADER, DIB_RGB_COLORS, HBITMAP, HGDIOBJ,
};
use windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_NORMAL;
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};
use windows::Win32::UI::Controls::{IImageList, ILD_TRANSPARENT};
use windows::Win32::UI::Shell::{
    SHGetFileInfoW, SHGetImageList, SHFILEINFOW, SHGFI_ICON, SHGFI_LARGEICON, SHGFI_SYSICONINDEX,
    SHGFI_USEFILEATTRIBUTES, SHIL_EXTRALARGE, SHIL_JUMBO,
};
use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, GetIconInfo, HICON, ICONINFO};

use super::raster;

pub fn file_png(path: &Path) -> Option<Vec<u8>> {
    if !path.exists() {
        return None;
    }
    shell_png(path, false)
}

pub fn type_png(ext: &str, _mime: &str) -> Option<Vec<u8>> {
    let name = if ext.is_empty() {
        "file".to_string()
    } else {
        format!("file{ext}")
    };
    shell_png(Path::new(&name), true)
}

struct ComGuard(bool);
impl Drop for ComGuard {
    fn drop(&mut self) {
        if self.0 {
            unsafe { CoUninitialize() };
        }
    }
}

fn shell_png(path: &Path, use_attrs: bool) -> Option<Vec<u8>> {
    let _com = ComGuard(unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok());
    if let Some(index) = sys_index(path, use_attrs) {
        for list in [SHIL_JUMBO, SHIL_EXTRALARGE] {
            if let Some(png) = image_list_png(list, index) {
                return Some(png);
            }
        }
    }
    large_icon_png(path, use_attrs)
}

fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

fn sys_index(path: &Path, use_attrs: bool) -> Option<i32> {
    let wide = wide(path);
    let mut info = SHFILEINFOW::default();
    let mut flags = SHGFI_SYSICONINDEX;
    if use_attrs {
        flags |= SHGFI_USEFILEATTRIBUTES;
    }
    let rc = unsafe {
        SHGetFileInfoW(
            PCWSTR(wide.as_ptr()),
            FILE_ATTRIBUTE_NORMAL,
            Some(std::ptr::addr_of_mut!(info)),
            std::mem::size_of::<SHFILEINFOW>() as u32,
            flags,
        )
    };
    if rc == 0 {
        return None;
    }
    if !info.hIcon.is_invalid() {
        unsafe {
            let _ = DestroyIcon(info.hIcon);
        }
    }
    Some(info.iIcon)
}

fn image_list_png(list_id: u32, index: i32) -> Option<Vec<u8>> {
    let list: IImageList = unsafe { SHGetImageList(list_id as i32) }.ok()?;
    let icon = unsafe { list.GetIcon(index, ILD_TRANSPARENT.0) }.ok()?;
    let png = hicon_png(icon);
    unsafe {
        let _ = DestroyIcon(icon);
    }
    png
}

fn large_icon_png(path: &Path, use_attrs: bool) -> Option<Vec<u8>> {
    let wide = wide(path);
    let mut info = SHFILEINFOW::default();
    let mut flags = SHGFI_ICON | SHGFI_LARGEICON;
    if use_attrs {
        flags |= SHGFI_USEFILEATTRIBUTES;
    }
    let rc = unsafe {
        SHGetFileInfoW(
            PCWSTR(wide.as_ptr()),
            FILE_ATTRIBUTE_NORMAL,
            Some(std::ptr::addr_of_mut!(info)),
            std::mem::size_of::<SHFILEINFOW>() as u32,
            flags,
        )
    };
    if rc == 0 || info.hIcon.is_invalid() {
        return None;
    }
    let png = hicon_png(info.hIcon);
    unsafe {
        let _ = DestroyIcon(info.hIcon);
    }
    png
}

struct BitmapGuard(HBITMAP);
impl Drop for BitmapGuard {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            unsafe {
                let _ = DeleteObject(HGDIOBJ::from(self.0));
            }
        }
    }
}

fn hicon_png(icon: HICON) -> Option<Vec<u8>> {
    if icon.is_invalid() {
        return None;
    }
    let mut info = ICONINFO::default();
    unsafe { GetIconInfo(icon, std::ptr::addr_of_mut!(info)) }.ok()?;
    let _color = BitmapGuard(info.hbmColor);
    let _mask = BitmapGuard(info.hbmMask);
    if info.hbmColor.is_invalid() {
        return None;
    }
    unsafe {
        let mut bmp = BITMAP::default();
        if GetObjectW(
            HGDIOBJ::from(info.hbmColor),
            std::mem::size_of::<BITMAP>() as i32,
            Some(&mut bmp as *mut BITMAP as *mut _),
        ) == 0
        {
            return None;
        }
        let width = usize::try_from(bmp.bmWidth).ok()?;
        let height = usize::try_from(bmp.bmHeight).ok()?;
        if width == 0 || height == 0 || width > 1024 || height > 1024 {
            return None;
        }
        let mut bi = BITMAPINFO::default();
        bi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bi.bmiHeader.biWidth = bmp.bmWidth;
        bi.bmiHeader.biHeight = -bmp.bmHeight;
        bi.bmiHeader.biPlanes = 1;
        bi.bmiHeader.biBitCount = 32;
        bi.bmiHeader.biCompression = 0;
        let mut pixels = vec![0u8; width * height * 4];
        let hdc = CreateCompatibleDC(None);
        if hdc.is_invalid() {
            return None;
        }
        let lines = GetDIBits(
            hdc,
            info.hbmColor,
            0,
            height as u32,
            Some(pixels.as_mut_ptr().cast()),
            std::ptr::addr_of_mut!(bi),
            DIB_RGB_COLORS,
        );
        let _ = DeleteDC(hdc);
        if lines == 0 {
            return None;
        }
        raster::bgra_to_rgba(&mut pixels);
        raster::standardize(bmp.bmWidth as u32, bmp.bmHeight as u32, &pixels)
    }
}
