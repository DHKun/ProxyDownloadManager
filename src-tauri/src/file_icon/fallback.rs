//! Built-in icons used only after the system provider fails.

use super::raster::{self, ICON_PX};
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    Generic,
    Executable,
    Archive,
    Video,
    Audio,
    Image,
    Document,
    Pdf,
    Installer,
    DiskImage,
}

pub fn category(file_name: &str, mime: &str) -> Category {
    let ext = super::key::normalize_extension(file_name);
    let ext = ext.trim_start_matches('.').to_ascii_lowercase();
    match ext.as_str() {
        "exe" | "msi" | "bat" | "cmd" | "com" | "appimage" | "bin" => return Category::Executable,
        "deb" | "rpm" | "pkg" | "apk" => return Category::Installer,
        "dmg" | "iso" | "img" => return Category::DiskImage,
        "zip" | "7z" | "rar" | "tar" | "gz" | "tgz" | "bz2" | "xz" | "zst" => {
            return Category::Archive
        }
        "mp4" | "mkv" | "webm" | "avi" | "mov" | "m4v" | "flv" | "wmv" => return Category::Video,
        "mp3" | "flac" | "wav" | "ogg" | "aac" | "m4a" | "opus" => return Category::Audio,
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "svg" | "ico" | "heic" => {
            return Category::Image
        }
        "pdf" => return Category::Pdf,
        "txt" | "md" | "log" | "doc" | "docx" | "odt" | "rtf" | "xls" | "xlsx" | "csv" | "ppt"
        | "pptx" => {
            return Category::Document;
        }
        _ => {}
    }
    let mime = mime.to_ascii_lowercase();
    if mime.starts_with("video/") {
        Category::Video
    } else if mime.starts_with("audio/") {
        Category::Audio
    } else if mime.starts_with("image/") {
        Category::Image
    } else if mime.contains("pdf") {
        Category::Pdf
    } else if mime.contains("zip") || mime.contains("archive") || mime.contains("compressed") {
        Category::Archive
    } else if mime.contains("executable") || mime.contains("x-msdos") {
        Category::Executable
    } else if mime.starts_with("text/") || mime.contains("document") || mime.contains("word") {
        Category::Document
    } else {
        Category::Generic
    }
}

pub fn png_for(file_name: &str, mime: &str) -> Vec<u8> {
    png_for_category(category(file_name, mime))
}

pub fn generic_png() -> Vec<u8> {
    png_for_category(Category::Generic)
}

fn png_for_category(cat: Category) -> Vec<u8> {
    let slot = match cat {
        Category::Generic => &CACHE[0],
        Category::Executable => &CACHE[1],
        Category::Archive => &CACHE[2],
        Category::Video => &CACHE[3],
        Category::Audio => &CACHE[4],
        Category::Image => &CACHE[5],
        Category::Document => &CACHE[6],
        Category::Pdf => &CACHE[7],
        Category::Installer => &CACHE[8],
        Category::DiskImage => &CACHE[9],
    };
    slot.get_or_init(|| draw(cat)).clone()
}

static CACHE: [OnceLock<Vec<u8>>; 10] = [
    OnceLock::new(),
    OnceLock::new(),
    OnceLock::new(),
    OnceLock::new(),
    OnceLock::new(),
    OnceLock::new(),
    OnceLock::new(),
    OnceLock::new(),
    OnceLock::new(),
    OnceLock::new(),
];

const S: i32 = ICON_PX as i32;
const INK: [u8; 4] = [150, 150, 150, 255];

struct Canvas {
    px: Vec<u8>,
}

impl Canvas {
    fn new() -> Self {
        Self {
            px: vec![0; (ICON_PX as usize) * (ICON_PX as usize) * 4],
        }
    }

    fn put(&mut self, x: i32, y: i32, c: [u8; 4]) {
        if x < 0 || y < 0 || x >= S || y >= S || c[3] == 0 {
            return;
        }
        let i = ((y as usize) * ICON_PX as usize + x as usize) * 4;
        self.px[i..i + 4].copy_from_slice(&c);
    }

    fn clear(&mut self, x: i32, y: i32) {
        if x < 0 || y < 0 || x >= S || y >= S {
            return;
        }
        let i = ((y as usize) * ICON_PX as usize + x as usize) * 4;
        self.px[i..i + 4].fill(0);
    }

    fn fill_rect(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, c: [u8; 4]) {
        for y in y0..y1 {
            for x in x0..x1 {
                self.put(x, y, c);
            }
        }
    }

    fn clear_rect(&mut self, x0: i32, y0: i32, x1: i32, y1: i32) {
        for y in y0..y1 {
            for x in x0..x1 {
                self.clear(x, y);
            }
        }
    }

    fn clear_circle(&mut self, cx: i32, cy: i32, r: i32) {
        let r2 = r * r;
        for y in (cy - r)..(cy + r) {
            for x in (cx - r)..(cx + r) {
                let dx = x - cx;
                let dy = y - cy;
                if dx * dx + dy * dy <= r2 {
                    self.clear(x, y);
                }
            }
        }
    }

    fn stroke_rect(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, t: i32, c: [u8; 4]) {
        self.fill_rect(x0, y0, x1, y0 + t, c);
        self.fill_rect(x0, y1 - t, x1, y1, c);
        self.fill_rect(x0, y0, x0 + t, y1, c);
        self.fill_rect(x1 - t, y0, x1, y1, c);
    }

    fn fill_circle(&mut self, cx: i32, cy: i32, r: i32, c: [u8; 4]) {
        let r2 = r * r;
        for y in (cy - r)..(cy + r) {
            for x in (cx - r)..(cx + r) {
                let dx = x - cx;
                let dy = y - cy;
                if dx * dx + dy * dy <= r2 {
                    self.put(x, y, c);
                }
            }
        }
    }

    fn page(&mut self) {
        let x0 = 22;
        let y0 = 14;
        let x1 = 74;
        let y1 = 82;
        self.stroke_rect(x0, y0, x1, y1, 4, INK);
        // folded corner
        self.fill_rect(56, 14, 74, 32, [0, 0, 0, 0]);
        for i in 0..18 {
            self.put(56 + i, 14 + i, INK);
            self.put(57 + i, 14 + i, INK);
            self.put(56 + i, 15 + i, INK);
        }
        self.fill_rect(56, 14, 74, 18, INK);
        self.fill_rect(70, 14, 74, 32, INK);
    }
}

fn draw(cat: Category) -> Vec<u8> {
    let mut c = Canvas::new();
    match cat {
        Category::Generic => c.page(),
        Category::Document => {
            c.page();
            c.fill_rect(32, 40, 64, 44, INK);
            c.fill_rect(32, 50, 60, 54, INK);
            c.fill_rect(32, 60, 56, 64, INK);
        }
        Category::Pdf => {
            c.page();
            c.fill_rect(30, 46, 66, 62, INK);
        }
        Category::Executable => {
            c.fill_rect(18, 18, 78, 78, INK);
            c.clear_rect(26, 26, 70, 70);
            for i in 0..18 {
                c.fill_rect(38, 39 + i, 38 + i, 40 + i, INK);
                c.fill_rect(38, 57 - i, 38 + i, 58 - i, INK);
            }
        }
        Category::Archive => {
            c.fill_rect(20, 28, 76, 76, INK);
            c.clear_rect(28, 40, 68, 68);
            c.fill_rect(40, 16, 56, 36, INK);
        }
        Category::Video => {
            c.stroke_rect(14, 24, 82, 72, 4, INK);
            for i in 0..16 {
                c.fill_rect(40, 34 + i, 40 + i, 35 + i, INK);
                c.fill_rect(40, 66 - i, 40 + i, 67 - i, INK);
            }
        }
        Category::Audio => {
            c.fill_rect(22, 40, 36, 56, INK);
            c.fill_rect(36, 32, 44, 64, INK);
            c.stroke_rect(52, 34, 70, 62, 4, INK);
            c.stroke_rect(60, 26, 78, 70, 4, INK);
        }
        Category::Image => {
            c.stroke_rect(14, 22, 82, 74, 4, INK);
            c.fill_circle(32, 38, 6, INK);
            c.fill_rect(24, 58, 46, 64, INK);
            c.fill_rect(42, 48, 70, 64, INK);
        }
        Category::Installer => {
            c.stroke_rect(22, 16, 74, 68, 4, INK);
            c.fill_rect(44, 28, 52, 50, INK);
            c.fill_rect(34, 44, 62, 52, INK);
            c.fill_rect(28, 72, 68, 80, INK);
        }
        Category::DiskImage => {
            c.fill_circle(48, 48, 30, INK);
            c.clear_circle(48, 48, 12);
        }
    }
    raster::encode(ICON_PX, ICON_PX, &c.px).unwrap_or_else(generic_solid)
}

fn generic_solid() -> Vec<u8> {
    let mut px = vec![0u8; (ICON_PX as usize) * (ICON_PX as usize) * 4];
    for y in 8..(ICON_PX - 8) {
        for x in 8..(ICON_PX - 8) {
            let edge = x < 12 || y < 12 || x >= ICON_PX - 12 || y >= ICON_PX - 12;
            if edge {
                let i = ((y * ICON_PX + x) * 4) as usize;
                px[i] = 150;
                px[i + 1] = 150;
                px[i + 2] = 150;
                px[i + 3] = 255;
            }
        }
    }
    raster::encode(ICON_PX, ICON_PX, &px).unwrap_or_else(|| vec![0x89, b'P', b'N', b'G'])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_known_and_unknown_extensions() {
        assert_eq!(category("setup.EXE", ""), Category::Executable);
        assert_eq!(category("a.zip", ""), Category::Archive);
        assert_eq!(category("a.mp4", ""), Category::Video);
        assert_eq!(category("a.mp3", ""), Category::Audio);
        assert_eq!(category("a.png", ""), Category::Image);
        assert_eq!(category("a.pdf", ""), Category::Pdf);
        assert_eq!(category("notes.txt", ""), Category::Document);
        assert_eq!(category("disk.dmg", ""), Category::DiskImage);
        assert_eq!(category("pkg.deb", ""), Category::Installer);
        assert_eq!(category("file.abcxyz", ""), Category::Generic);
        assert_eq!(category("noext", "video/mp4"), Category::Video);
    }

    #[test]
    fn unknown_extension_is_a_png() {
        let png = png_for("file.abcxyz", "");
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert!(super::super::raster::is_png(&png));
    }
}
