use std::path::{Path, PathBuf};

/// Partial file for one download: `{home}/temp/{id}.pdm`.
/// Callers must not invent a `.pdm` suffix themselves.
pub fn temp_path(download_id: u64) -> String {
    crate::state::gob::temp_dir()
        .join(format!("{download_id}.pdm"))
        .to_string_lossy()
        .to_string()
}

pub fn hls_part_dir(download_id: u64) -> PathBuf {
    crate::state::gob::temp_dir().join(format!("hls-{download_id}"))
}

/// Older builds stored `{save_path}.pdm`. Move that file to the id path once
/// so a resume after upgrade still finds the bytes.
pub fn migrate_legacy_temp(download_id: u64, save_path: &str) {
    let path = temp_path(download_id);
    if Path::new(&path).exists() {
        return;
    }
    let legacy = format!("{save_path}.pdm");
    if !Path::new(&legacy).exists() {
        return;
    }
    if let Some(parent) = Path::new(&path).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::rename(&legacy, &path);
}

/// Drop the temp file and any HLS part directory. Does not touch the final file.
pub fn remove_temp(download_id: u64) {
    let _ = std::fs::remove_file(temp_path(download_id));
    let _ = std::fs::remove_dir_all(hls_part_dir(download_id));
}

/// Delete temp data and the finished file.
pub fn remove_download_files(download_id: u64, save_path: &str) {
    remove_temp(download_id);
    let _ = std::fs::remove_file(save_path);
}

pub async fn create_output_file(
    download_id: u64,
    save_path: &str,
    total_size: u64,
) -> Result<std::fs::File, String> {
    migrate_legacy_temp(download_id, save_path);
    let path = temp_path(download_id);
    if let Some(parent) = Path::new(&path).parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .read(true)
        .open(&path)
        .map_err(|e| format!("Failed to create output file: {e}"))?;
    if total_size > 0 {
        let _ = file.set_len(total_size);
    }
    Ok(file)
}

/// Rename the id temp file onto `save_path`. Caller must have dropped handles
/// and synced. Failure leaves the temp file in place.
pub async fn finalize_file(download_id: u64, save_path: &str) -> Result<(), String> {
    migrate_legacy_temp(download_id, save_path);
    let pdm_path = temp_path(download_id);
    if !Path::new(&pdm_path).exists() {
        return Err(format!("partial file missing: {pdm_path}"));
    }
    if let Some(parent) = Path::new(save_path).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
    }
    // Windows: antivirus/indexers briefly hold freshly written files, making
    // the rename fail with a sharing violation — retry with a short backoff.
    let attempts = if cfg!(windows) { 10 } else { 1 };
    let mut last_err = String::new();
    for i in 0..attempts {
        match tokio::fs::rename(&pdm_path, save_path).await {
            Ok(()) => return Ok(()),
            Err(e) => {
                last_err = e.to_string();
                if i + 1 < attempts {
                    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                }
            }
        }
    }
    Err(format!("Failed to rename file: {last_err}"))
}

/// `written` must match a known length before a download may complete.
/// `expected == 0` means the length was never known.
pub fn length_shortfall(written: u64, expected: u64) -> Option<u64> {
    if expected > 0 && written < expected {
        Some(expected - written)
    } else {
        None
    }
}

/// Cross-platform write_at: write to a specific offset without seeking.
#[cfg(unix)]
pub fn write_at(file: &std::fs::File, buf: &[u8], offset: u64) -> std::io::Result<()> {
    use std::os::unix::fs::FileExt;
    FileExt::write_all_at(file, buf, offset)
}

#[cfg(windows)]
pub fn write_at(file: &std::fs::File, buf: &[u8], offset: u64) -> std::io::Result<()> {
    use std::os::windows::fs::FileExt;
    let mut written = 0;
    while written < buf.len() {
        let n = FileExt::seek_write(file, &buf[written..], offset + written as u64)?;
        written += n;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn temp_paths_differ_by_id() {
        let a = temp_path(11);
        let b = temp_path(12);
        assert_ne!(a, b);
        assert!(a.ends_with("11.pdm"));
        assert!(b.ends_with("12.pdm"));
        assert!(a.contains("temp"));
        assert!(!a.contains("foo.zip"));
    }

    #[test]
    fn shortfall_when_body_ends_early() {
        assert_eq!(length_shortfall(60, 100), Some(40));
        assert_eq!(length_shortfall(100, 100), None);
        assert_eq!(length_shortfall(60, 0), None);
    }
}
