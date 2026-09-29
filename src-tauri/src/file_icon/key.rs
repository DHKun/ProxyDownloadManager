//! Cache keys. Type icons share an extension; real files include mtime and size.

/// ".PDF" and "Report.PDF" both become ".pdf". No extension becomes "".
pub fn normalize_extension(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name).trim();
    let Some((_, ext)) = base.rsplit_once('.') else {
        return String::new();
    };
    if ext.is_empty() || (base.starts_with('.') && !base[1..].contains('.')) {
        // ".pdf" and ".gitignore" keep the whole name, lowercased.
        if base.starts_with('.') && !base[1..].contains('.') {
            return base.to_ascii_lowercase();
        }
        return String::new();
    }
    format!(".{}", ext.to_ascii_lowercase())
}

pub fn file_cache_key(path: &str, mtime_secs: u64, size: u64) -> String {
    format!("file:{path}:{mtime_secs}:{size}")
}

pub fn type_cache_key(file_name: &str, theme: &str) -> String {
    let ext = normalize_extension(file_name);
    let token = if ext.is_empty() { "none" } else { ext.as_str() };
    #[cfg(target_os = "linux")]
    {
        return format!("type:linux:{token}:{theme}");
    }
    #[cfg(target_os = "macos")]
    {
        let _ = theme;
        return format!("type:macos:{token}");
    }
    #[cfg(target_os = "windows")]
    {
        let _ = theme;
        return format!("type:windows:{token}");
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        let _ = theme;
        format!("type:other:{token}")
    }
}

/// MIME hint used when asking the system. `application/octet-stream` is ignored.
pub fn effective_mime(content_type: &str, ext: &str) -> String {
    let ct = content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if ct.contains('/') && ct != "application/octet-stream" {
        return ct;
    }
    mime_from_extension(ext).unwrap_or("").to_string()
}

pub fn mime_from_extension(ext: &str) -> Option<&'static str> {
    Some(
        match ext.trim_start_matches('.').to_ascii_lowercase().as_str() {
            "pdf" => "application/pdf",
            "zip" => "application/zip",
            "7z" => "application/x-7z-compressed",
            "rar" => "application/vnd.rar",
            "gz" | "tgz" => "application/gzip",
            "tar" => "application/x-tar",
            "exe" | "msi" | "appimage" => "application/x-executable",
            "deb" => "application/vnd.debian.binary-package",
            "rpm" => "application/x-rpm",
            "dmg" | "iso" | "img" => "application/x-cd-image",
            "pkg" => "application/vnd.apple.installer+xml",
            "mp4" | "m4v" => "video/mp4",
            "mkv" => "video/x-matroska",
            "webm" => "video/webm",
            "avi" => "video/x-msvideo",
            "mov" => "video/quicktime",
            "mp3" => "audio/mpeg",
            "flac" => "audio/flac",
            "wav" => "audio/wav",
            "ogg" => "audio/ogg",
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "gif" => "image/gif",
            "webp" => "image/webp",
            "svg" => "image/svg+xml",
            "txt" | "log" | "md" => "text/plain",
            "doc" | "docx" | "odt" | "rtf" => "application/vnd.oasis.opendocument.text",
            "xls" | "xlsx" | "csv" => "application/vnd.ms-excel",
            "ppt" | "pptx" => "application/vnd.ms-powerpoint",
            "json" | "xml" | "html" | "css" | "js" | "ts" => "text/plain",
            _ => return None,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_extension() {
        assert_eq!(normalize_extension("Report.PDF"), ".pdf");
        assert_eq!(normalize_extension(".PDF"), ".pdf");
        assert_eq!(normalize_extension("archive.tar.GZ"), ".gz");
        assert_eq!(normalize_extension("noext"), "");
        assert_eq!(normalize_extension("dir/file.abcxyz"), ".abcxyz");
        assert_eq!(normalize_extension(r"C:\Downloads\setup.EXE"), ".exe");
    }

    #[test]
    fn same_extension_shares_type_key() {
        let theme = "breeze-dark";
        assert_eq!(
            type_cache_key("a.pdf", theme),
            type_cache_key("b.PDF", theme)
        );
        assert_ne!(
            type_cache_key("a.pdf", theme),
            type_cache_key("a.zip", theme)
        );
    }

    #[test]
    fn file_key_changes_with_mtime_or_size() {
        let a = file_cache_key(r"C:\Downloads\setup.exe", 1759123456, 4532142);
        let b = file_cache_key(r"C:\Downloads\setup.exe", 1759123999, 4532142);
        let c = file_cache_key(r"C:\Downloads\setup.exe", 1759123456, 100);
        assert_ne!(a, b);
        assert_ne!(a, c);
        assert_eq!(
            a,
            file_cache_key(r"C:\Downloads\setup.exe", 1759123456, 4532142)
        );
    }

    #[test]
    fn hundred_downloads_collapse_to_four_type_keys() {
        let mut names = Vec::new();
        names.extend((0..60).map(|i| format!("app{i}.exe")));
        names.extend((0..20).map(|i| format!("pack{i}.zip")));
        names.extend((0..10).map(|i| format!("doc{i}.pdf")));
        names.extend((0..10).map(|i| format!("note{i}.txt")));
        let mut keys: Vec<_> = names.iter().map(|n| type_cache_key(n, "default")).collect();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), 4);
    }
}
