//! System file icons for the download list and the details window.
//! One 96×96 PNG is cached per type or per real file. The webview scales it.

mod cache;
mod fallback;
mod key;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
mod raster;
#[cfg(target_os = "windows")]
mod windows;

use std::path::Path;
use std::sync::OnceLock;
use std::time::UNIX_EPOCH;

use base64::Engine;
use serde::{Deserialize, Serialize};

use cache::IconStore;
use key::{effective_mime, file_cache_key, normalize_extension, type_cache_key};

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IconRequest {
    pub id: u64,
    #[serde(default)]
    pub file_name: String,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub mime_type: String,
    #[serde(default)]
    pub completed: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct IconPayload {
    pub key: String,
    pub mime_type: String,
    pub data: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct IconMatch {
    pub id: u64,
    pub key: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct IconBatch {
    pub icons: Vec<IconPayload>,
    pub matches: Vec<IconMatch>,
}

pub struct FileIconService {
    store: IconStore,
    theme: OnceLock<String>,
}

impl FileIconService {
    pub fn new(cache_dir: std::path::PathBuf) -> Self {
        Self {
            store: IconStore::new(cache_dir, cache::DISK_LIMIT),
            theme: OnceLock::new(),
        }
    }

    pub fn get_one(&self, request: IconRequest) -> IconPayload {
        let batch = self.get_many(vec![request]);
        batch
            .icons
            .into_iter()
            .next()
            .unwrap_or_else(|| IconPayload {
                key: String::new(),
                mime_type: "image/png".to_string(),
                data: base64::engine::general_purpose::STANDARD.encode(fallback::generic_png()),
            })
    }

    pub fn get_many(&self, requests: Vec<IconRequest>) -> IconBatch {
        let theme = self.theme.get_or_init(theme_name);
        let mut order = Vec::with_capacity(requests.len());
        let mut unique: Vec<String> = Vec::new();
        for req in &requests {
            let key = request_key(req, theme);
            if !unique.iter().any(|k| k == &key) {
                unique.push(key.clone());
            }
            order.push((req.id, key));
        }

        let mut icons = Vec::with_capacity(unique.len());
        for key in &unique {
            let sample = requests.iter().find(|req| request_key(req, theme) == *key);
            let bytes = self.store.load(key, || match sample {
                Some(req) => render(req),
                None => fallback::generic_png(),
            });
            icons.push(IconPayload {
                key: key.clone(),
                mime_type: "image/png".to_string(),
                data: base64::engine::general_purpose::STANDARD.encode(bytes.as_ref()),
            });
        }

        IconBatch {
            icons,
            matches: order
                .into_iter()
                .map(|(id, key)| IconMatch { id, key })
                .collect(),
        }
    }
}

fn request_key(req: &IconRequest, theme: &str) -> String {
    if req.completed && !req.path.is_empty() && distinct_file_icon(&req.path) {
        let path = Path::new(&req.path);
        if path.exists() {
            if let Ok(meta) = path.metadata() {
                let mtime = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                return file_cache_key(&req.path, mtime, meta.len());
            }
        }
    }
    type_cache_key(&req.file_name, theme)
}

/// Windows and macOS can return a per-file icon (an EXE's embedded icon, a
/// bundle, a disk image). Linux theme icons are per type, except `.desktop`.
fn distinct_file_icon(path: &str) -> bool {
    #[cfg(target_os = "linux")]
    {
        return Path::new(path)
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("desktop"));
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = path;
        true
    }
}

fn render(req: &IconRequest) -> Vec<u8> {
    let ext = normalize_extension(&req.file_name);
    let mime = effective_mime(&req.mime_type, &ext);
    if req.completed && !req.path.is_empty() {
        let path = Path::new(&req.path);
        if path.exists() {
            if let Some(png) = system_file(path) {
                return png;
            }
            log::debug!("file icon: real file icon unavailable");
        }
    }
    if let Some(png) = system_type(&ext, &mime) {
        return png;
    }
    log::warn!("file icon: system provider failed, using fallback");
    fallback::png_for(&req.file_name, &mime)
}

fn theme_name() -> String {
    #[cfg(target_os = "linux")]
    {
        return linux::theme_name();
    }
    #[cfg(not(target_os = "linux"))]
    {
        "default".to_string()
    }
}

fn system_file(path: &Path) -> Option<Vec<u8>> {
    #[cfg(target_os = "windows")]
    {
        return windows::file_png(path);
    }
    #[cfg(target_os = "macos")]
    {
        return macos::file_png(path);
    }
    #[cfg(target_os = "linux")]
    {
        return linux::file_png(path);
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        let _ = path;
        None
    }
}

fn system_type(ext: &str, mime: &str) -> Option<Vec<u8>> {
    #[cfg(target_os = "windows")]
    {
        return windows::type_png(ext, mime);
    }
    #[cfg(target_os = "macos")]
    {
        return macos::type_png(ext, mime);
    }
    #[cfg(target_os = "linux")]
    {
        return linux::type_png(ext, mime);
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        let _ = (ext, mime);
        None
    }
}
