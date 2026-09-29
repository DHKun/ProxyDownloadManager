//! Freedesktop icon theme lookup. No GTK or Qt.

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use super::key::effective_mime;
use super::raster;

pub fn theme_name() -> String {
    static THEME: OnceLock<String> = OnceLock::new();
    THEME.get_or_init(detect_theme).clone()
}

pub fn file_png(path: &Path) -> Option<Vec<u8>> {
    if !path.exists() {
        return None;
    }
    if path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("desktop"))
    {
        if let Some(icon) = desktop_icon(path) {
            if let Some(png) = load_icon_name(&icon, &theme_name()) {
                return Some(png);
            }
        }
    }
    let ext = super::key::normalize_extension(&path.to_string_lossy());
    type_png(&ext, &effective_mime("", &ext))
}

pub fn type_png(ext: &str, mime: &str) -> Option<Vec<u8>> {
    let mime = if mime.is_empty() {
        effective_mime("", ext)
    } else {
        mime.to_string()
    };
    let theme = theme_name();
    for name in icon_names(&mime, ext) {
        if let Some(png) = load_icon_name(&name, &theme) {
            return Some(png);
        }
    }
    None
}

fn icon_names(mime: &str, ext: &str) -> Vec<String> {
    let mut names = Vec::new();
    if let Some(mapped) = shared_mime_icon(mime) {
        names.push(mapped);
    }
    if !mime.is_empty() {
        names.push(mime.replace('/', "-"));
    }
    if let Some(name) = generic_name(mime, ext) {
        names.push(name.to_string());
    }
    names
}

fn generic_name(mime: &str, ext: &str) -> Option<&'static str> {
    let ext = ext.trim_start_matches('.');
    if matches!(
        ext,
        "zip" | "7z" | "rar" | "tar" | "gz" | "tgz" | "bz2" | "xz"
    ) {
        return Some("package-x-generic");
    }
    if mime.starts_with("video/") {
        Some("video-x-generic")
    } else if mime.starts_with("audio/") {
        Some("audio-x-generic")
    } else if mime.starts_with("image/") {
        Some("image-x-generic")
    } else if mime.starts_with("text/") {
        Some("text-x-generic")
    } else if mime.contains("executable") || matches!(ext, "exe" | "msi" | "appimage" | "bin") {
        Some("application-x-executable")
    } else if mime.contains("pdf") || ext == "pdf" {
        Some("application-pdf")
    } else {
        None
    }
}

fn shared_mime_icon(mime: &str) -> Option<String> {
    if mime.is_empty() {
        return None;
    }
    static MAP: OnceLock<HashMap<String, String>> = OnceLock::new();
    let map = MAP.get_or_init(load_shared_mime);
    map.get(mime).cloned()
}

fn load_shared_mime() -> HashMap<String, String> {
    let mut map = HashMap::new();
    for path in [
        "/usr/share/mime/generic-icons",
        "/usr/share/mime/icons",
        "/usr/local/share/mime/generic-icons",
    ] {
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        for line in text.lines() {
            let Some((mime, icon)) = line.split_once(':') else {
                continue;
            };
            if !mime.is_empty() && !icon.is_empty() {
                map.entry(mime.to_string())
                    .or_insert_with(|| icon.to_string());
            }
        }
    }
    map
}

fn load_icon_name(name: &str, theme: &str) -> Option<Vec<u8>> {
    if let Some(path) = Path::new(name)
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
    {
        if Path::new(name).is_file() {
            return load_path(Path::new(name));
        }
        let _ = path;
    }
    let mut seen = HashSet::new();
    lookup(name, theme, &mut seen).and_then(|p| load_path(&p))
}

fn lookup(name: &str, theme: &str, seen: &mut HashSet<String>) -> Option<PathBuf> {
    if theme.is_empty() || !seen.insert(theme.to_string()) {
        return None;
    }
    const SIZES: [&str; 8] = [
        "128x128", "96x96", "64x64", "48x48", "scalable", "256x256", "32x32", "22x22",
    ];
    const CONTEXTS: [&str; 6] = [
        "mimetypes",
        "apps",
        "places",
        "devices",
        "categories",
        "status",
    ];
    for base in theme_bases(theme) {
        for size in SIZES {
            for ctx in CONTEXTS {
                let dir = base.join(size).join(ctx);
                let png = dir.join(format!("{name}.png"));
                if png.is_file() {
                    return Some(png);
                }
                let svg = dir.join(format!("{name}.svg"));
                if svg.is_file() {
                    return Some(svg);
                }
            }
        }
    }
    for parent in inherits(theme) {
        if let Some(found) = lookup(name, &parent, seen) {
            return Some(found);
        }
    }
    if theme != "hicolor" {
        if let Some(found) = lookup(name, "hicolor", seen) {
            return Some(found);
        }
    }
    for dir in data_dirs() {
        let png = dir.join("pixmaps").join(format!("{name}.png"));
        if png.is_file() {
            return Some(png);
        }
        let svg = dir.join("pixmaps").join(format!("{name}.svg"));
        if svg.is_file() {
            return Some(svg);
        }
    }
    None
}

fn load_path(path: &Path) -> Option<Vec<u8>> {
    let bytes = std::fs::read(path).ok()?;
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    if ext.eq_ignore_ascii_case("svg") || bytes.starts_with(b"<svg") || bytes.starts_with(b"<?xml")
    {
        return render_svg(&bytes);
    }
    raster::standardize_png(&bytes)
}

fn render_svg(bytes: &[u8]) -> Option<Vec<u8>> {
    let tree = resvg::usvg::Tree::from_data(bytes, &resvg::usvg::Options::default()).ok()?;
    let size = tree.size();
    let w = size.width().max(1.0);
    let h = size.height().max(1.0);
    let scale = (raster::ICON_PX as f32 / w).min(raster::ICON_PX as f32 / h);
    let mut pixmap = resvg::tiny_skia::Pixmap::new(raster::ICON_PX, raster::ICON_PX)?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    raster::standardize_png(&pixmap.encode_png().ok()?).or_else(|| Some(pixmap.encode_png().ok()?))
}

fn theme_bases(theme: &str) -> Vec<PathBuf> {
    let mut bases = Vec::new();
    if let Some(home) = dirs::home_dir() {
        bases.push(home.join(".icons").join(theme));
        bases.push(home.join(".local/share/icons").join(theme));
    }
    if let Ok(data_home) = std::env::var("XDG_DATA_HOME") {
        if !data_home.is_empty() {
            bases.push(PathBuf::from(data_home).join("icons").join(theme));
        }
    }
    for dir in data_dirs() {
        bases.push(dir.join("icons").join(theme));
    }
    bases
}

fn data_dirs() -> Vec<PathBuf> {
    let raw = std::env::var("XDG_DATA_DIRS")
        .unwrap_or_else(|_| "/usr/local/share:/usr/share".to_string());
    raw.split(':')
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .collect()
}

fn inherits(theme: &str) -> Vec<String> {
    static CACHE: OnceLock<Mutex<HashMap<String, Vec<String>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = cache.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(hit) = guard.get(theme) {
        return hit.clone();
    }
    let parsed = read_inherits(theme);
    guard.insert(theme.to_string(), parsed.clone());
    parsed
}

fn read_inherits(theme: &str) -> Vec<String> {
    for base in theme_bases(theme) {
        let path = base.join("index.theme");
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        for line in text.lines() {
            let line = line.trim();
            let Some(rest) = line.strip_prefix("Inherits") else {
                continue;
            };
            let rest = rest.trim().trim_start_matches('=').trim();
            if rest.is_empty() {
                continue;
            }
            return rest
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
        }
    }
    Vec::new()
}

fn detect_theme() -> String {
    if let Some(home) = dirs::home_dir() {
        for rel in ["gtk-3.0/settings.ini", "gtk-4.0/settings.ini"] {
            if let Some(name) = ini_value(&home.join(".config").join(rel), "gtk-icon-theme-name") {
                return name;
            }
        }
        if let Some(name) = kde_icon_theme(&home.join(".config/kdeglobals")) {
            return name;
        }
    }
    if let Some(name) = gsettings_theme() {
        return name;
    }
    let desktop = std::env::var("XDG_CURRENT_DESKTOP")
        .unwrap_or_default()
        .to_ascii_lowercase();
    if desktop.contains("kde") {
        "breeze".to_string()
    } else if desktop.contains("xfce") {
        "Adwaita".to_string()
    } else if desktop.contains("gnome") || desktop.contains("unity") {
        "Adwaita".to_string()
    } else {
        "hicolor".to_string()
    }
}

fn ini_value(path: &Path, key: &str) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    for line in text.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix(key) else {
            continue;
        };
        let rest = rest.trim();
        if !rest.starts_with('=') {
            continue;
        }
        let value = rest
            .trim_start_matches('=')
            .trim()
            .trim_matches('"')
            .trim()
            .to_string();
        if !value.is_empty() {
            return Some(value);
        }
    }
    None
}

fn kde_icon_theme(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut in_icons = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_icons = line.eq_ignore_ascii_case("[Icons]");
            continue;
        }
        if in_icons {
            if let Some(name) = line.strip_prefix("Theme=") {
                let name = name.trim();
                if !name.is_empty() {
                    return Some(name.to_string());
                }
            }
        }
    }
    None
}

fn gsettings_theme() -> Option<String> {
    let mut child = Command::new("timeout")
        .args([
            "0.4",
            "gsettings",
            "get",
            "org.gnome.desktop.interface",
            "icon-theme",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let start = std::time::Instant::now();
    loop {
        if let Some(status) = child.try_wait().ok()? {
            if !status.success() {
                return None;
            }
            let mut out = String::new();
            let _ = child.stdout.as_mut()?.read_to_string(&mut out);
            let value = out.trim().trim_matches('\'').trim_matches('"').trim();
            if value.is_empty() {
                return None;
            }
            return Some(value.to_string());
        }
        if start.elapsed() > Duration::from_millis(500) {
            let _ = child.kill();
            return None;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_extension_has_no_theme_icon() {
        assert!(type_png(".abcxyz", "").is_none());
    }

    #[test]
    fn pdf_theme_icon_is_png_when_present() {
        let Some(png) = type_png(".pdf", "application/pdf") else {
            return;
        };
        assert!(super::raster::is_png(&png));
    }
}

fn desktop_icon(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    for line in text.lines() {
        let line = line.trim();
        if let Some(icon) = line.strip_prefix("Icon=") {
            let icon = icon.trim();
            if !icon.is_empty() {
                return Some(icon.to_string());
            }
        }
    }
    None
}
