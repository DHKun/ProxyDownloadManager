//! L1 memory cache and L2 `icon-cache-v1/{type,file}/<sha256>.png`.
//! The directory is not scanned at startup. Eviction runs when a write crosses the cap.

use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use super::raster;

const MEM_CAP: usize = 512;
pub const DISK_LIMIT: u64 = 64 * 1024 * 1024;

struct MemEntry {
    at: SystemTime,
    bytes: Arc<Vec<u8>>,
}

struct WaitSlot {
    lock: Mutex<Option<Arc<Vec<u8>>>>,
    cv: Condvar,
}

struct Counter {
    loaded: bool,
    bytes: u64,
    writes: u32,
}

pub struct IconStore {
    root: PathBuf,
    limit: u64,
    mem: Mutex<HashMap<String, MemEntry>>,
    inflight: Mutex<HashMap<String, Arc<WaitSlot>>>,
    counter: Mutex<Counter>,
}

impl IconStore {
    pub fn new(root: PathBuf, limit: u64) -> Self {
        Self {
            root,
            limit,
            mem: Mutex::new(HashMap::new()),
            inflight: Mutex::new(HashMap::new()),
            counter: Mutex::new(Counter {
                loaded: false,
                bytes: 0,
                writes: 0,
            }),
        }
    }

    pub fn load(&self, key: &str, produce: impl FnOnce() -> Vec<u8>) -> Arc<Vec<u8>> {
        if let Some(hit) = self.mem_get(key) {
            return hit;
        }

        let slot = {
            let mut inflight = lock(&self.inflight);
            if let Some(existing) = inflight.get(key) {
                let slot = existing.clone();
                drop(inflight);
                return self.wait(slot);
            }
            let slot = Arc::new(WaitSlot {
                lock: Mutex::new(None),
                cv: Condvar::new(),
            });
            inflight.insert(key.to_string(), slot.clone());
            slot
        };

        let bytes = if let Some(disk) = self.read_disk(key) {
            disk
        } else {
            let kind = if key.starts_with("file:") {
                "file"
            } else {
                "type"
            };
            log::debug!("icon cache miss ({kind})");
            let produced = std::panic::catch_unwind(std::panic::AssertUnwindSafe(produce));
            let bytes = match produced {
                Ok(bytes) if raster::is_png(&bytes) => bytes,
                Ok(_) => {
                    log::warn!("file icon: provider returned an unreadable image, using fallback");
                    super::fallback::generic_png()
                }
                Err(_) => {
                    log::warn!("file icon: provider panicked, using fallback");
                    super::fallback::generic_png()
                }
            };
            self.write_disk(key, &bytes);
            Arc::new(bytes)
        };

        self.remember(key, bytes.clone());
        {
            let mut guard = lock(&slot.lock);
            *guard = Some(bytes.clone());
            slot.cv.notify_all();
        }
        lock(&self.inflight).remove(key);
        bytes
    }

    fn wait(&self, slot: Arc<WaitSlot>) -> Arc<Vec<u8>> {
        let mut guard = lock(&slot.lock);
        while guard.is_none() {
            guard = slot.cv.wait(guard).unwrap_or_else(|e| e.into_inner());
        }
        guard.clone().unwrap_or_else(empty_png)
    }

    fn mem_get(&self, key: &str) -> Option<Arc<Vec<u8>>> {
        let mut mem = lock(&self.mem);
        let entry = mem.get_mut(key)?;
        entry.at = SystemTime::now();
        Some(entry.bytes.clone())
    }

    fn remember(&self, key: &str, bytes: Arc<Vec<u8>>) {
        let mut mem = lock(&self.mem);
        if mem.len() >= MEM_CAP && !mem.contains_key(key) {
            if let Some(old) = mem.iter().min_by_key(|(_, e)| e.at).map(|(k, _)| k.clone()) {
                mem.remove(&old);
            }
        }
        mem.insert(
            key.to_string(),
            MemEntry {
                at: SystemTime::now(),
                bytes,
            },
        );
    }

    fn path_for(&self, key: &str) -> PathBuf {
        let kind = if key.starts_with("file:") {
            "file"
        } else {
            "type"
        };
        let mut hasher = Sha256::new();
        hasher.update(key.as_bytes());
        let name = format!("{:x}.png", hasher.finalize());
        self.root.join(kind).join(name)
    }

    fn read_disk(&self, key: &str) -> Option<Arc<Vec<u8>>> {
        let path = self.path_for(key);
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(_) => return None,
        };
        if !raster::is_png(&bytes) {
            log::warn!("file icon: dropped unreadable disk cache");
            let _ = fs::remove_file(&path);
            return None;
        }
        if let Ok(file) = File::options().write(true).open(&path) {
            let _ = file.set_modified(SystemTime::now());
        }
        Some(Arc::new(bytes))
    }

    fn write_disk(&self, key: &str, bytes: &[u8]) {
        let path = self.path_for(key);
        let Some(parent) = path.parent() else { return };
        if let Err(e) = fs::create_dir_all(parent) {
            log::warn!("file icon: disk cache directory unavailable: {e}");
            return;
        }
        if let Err(e) = fs::write(&path, bytes) {
            log::warn!("file icon: disk cache write failed: {e}");
            return;
        }
        let mut counter = lock(&self.counter);
        if !counter.loaded {
            counter.bytes = read_counter(&self.root.join("bytes"));
            counter.loaded = true;
        }
        counter.bytes = counter.bytes.saturating_add(bytes.len() as u64);
        counter.writes = counter.writes.saturating_add(1);
        let over = counter.bytes > self.limit;
        let periodic = counter.writes % 40 == 0;
        let total = counter.bytes;
        drop(counter);
        let _ = fs::write(self.root.join("bytes"), total.to_string());
        if over || periodic {
            self.sweep();
        }
    }

    fn sweep(&self) {
        let mut files = Vec::new();
        let mut total = 0u64;
        for kind in ["type", "file"] {
            let dir = self.root.join(kind);
            let Ok(rd) = fs::read_dir(&dir) else { continue };
            for ent in rd.flatten() {
                let Ok(meta) = ent.metadata() else { continue };
                if !meta.is_file() {
                    continue;
                }
                let len = meta.len();
                total = total.saturating_add(len);
                let modified = meta.modified().unwrap_or(UNIX_EPOCH);
                files.push((modified, len, ent.path()));
            }
        }
        if total > self.limit {
            files.sort_by_key(|(modified, _, _)| *modified);
            let target = self.limit.saturating_mul(3) / 4;
            for (_, len, path) in &files {
                if total <= target {
                    break;
                }
                if fs::remove_file(path).is_ok() {
                    total = total.saturating_sub(*len);
                }
            }
        }
        if let Ok(mut counter) = self.counter.lock() {
            counter.bytes = total;
            counter.loaded = true;
        }
        let _ = fs::write(self.root.join("bytes"), total.to_string());
    }
}

fn read_counter(path: &Path) -> u64 {
    fs::read_to_string(path)
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

fn empty_png() -> Arc<Vec<u8>> {
    Arc::new(super::fallback::generic_png())
}

#[cfg(test)]
fn set_age(path: &Path, secs_ago: u64) {
    let when = SystemTime::now()
        .checked_sub(std::time::Duration::from_secs(secs_ago))
        .unwrap_or(UNIX_EPOCH);
    if let Ok(file) = File::options().write(true).open(path) {
        let _ = file.set_modified(when);
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread;
    use std::time::Duration;
    fn temp_dir(name: &str) -> PathBuf {
        static N: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "pdm_icon_{}_{}_{}",
            std::process::id(),
            name,
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn corrupted_disk_cache_is_a_miss() {
        let dir = temp_dir("corrupt");
        let store = IconStore::new(dir, DISK_LIMIT);
        let key = "type:test:.pdf";
        let path = store.path_for(key);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"").unwrap();
        let calls = AtomicUsize::new(0);
        let bytes = store.load(key, || {
            calls.fetch_add(1, Ordering::SeqCst);
            super::super::fallback::generic_png()
        });
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(raster::is_png(&bytes));
        assert!(raster::is_png(&fs::read(store.path_for(key)).unwrap()));
        let _ = fs::remove_dir_all(&store.root);
    }

    #[test]
    fn garbage_png_is_replaced() {
        let dir = temp_dir("garbage");
        let store = IconStore::new(dir, DISK_LIMIT);
        let key = "type:test:.zip";
        let path = store.path_for(key);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"not a png at all").unwrap();
        let bytes = store.load(key, super::super::fallback::generic_png);
        assert!(raster::is_png(&bytes));
        let _ = fs::remove_dir_all(&store.root);
    }

    #[test]
    fn inflight_dedup_calls_producer_once() {
        let dir = temp_dir("inflight");
        let store = Arc::new(IconStore::new(dir, DISK_LIMIT));
        let calls = Arc::new(AtomicUsize::new(0));
        let mut handles = Vec::new();
        for _ in 0..8 {
            let store = store.clone();
            let calls = calls.clone();
            handles.push(thread::spawn(move || {
                store.load("type:test:.exe", || {
                    calls.fetch_add(1, Ordering::SeqCst);
                    thread::sleep(Duration::from_millis(40));
                    super::super::fallback::generic_png()
                })
            }));
        }
        let first = handles.pop().unwrap().join().unwrap();
        for handle in handles {
            let bytes = handle.join().unwrap();
            assert_eq!(bytes.as_slice(), first.as_slice());
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let _ = fs::remove_dir_all(&store.root);
    }

    #[test]
    fn second_load_does_not_call_producer() {
        let dir = temp_dir("disk");
        let store = IconStore::new(dir, DISK_LIMIT);
        let calls = AtomicUsize::new(0);
        let produce = || {
            calls.fetch_add(1, Ordering::SeqCst);
            super::super::fallback::generic_png()
        };
        let _ = store.load("type:test:.txt", produce);
        // New store, same directory: memory is empty, disk should hit.
        let store2 = IconStore::new(store.root.clone(), DISK_LIMIT);
        let _ = store2.load("type:test:.txt", produce);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let _ = fs::remove_dir_all(&store.root);
    }

    #[test]
    fn eviction_drops_oldest_files() {
        let dir = temp_dir("evict");
        let png = super::super::fallback::generic_png();
        // Two icons fit; the third crosses the cap and the oldest is removed.
        let store = IconStore::new(dir, (png.len() as u64).saturating_mul(5) / 2);
        store.write_disk("type:test:a", &png);
        set_age(&store.path_for("type:test:a"), 30);
        store.write_disk("type:test:b", &png);
        set_age(&store.path_for("type:test:b"), 20);
        store.write_disk("type:test:c", &png);
        let left = fs::read_dir(store.root.join("type")).unwrap().count();
        assert!(left < 3, "expected eviction, still {left} files");
        assert!(fs::metadata(store.path_for("type:test:c")).is_ok());
        let _ = fs::remove_dir_all(&store.root);
    }
}
