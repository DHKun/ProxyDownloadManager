//! Minimal in-memory logger so benchmark tests can read the `[startup]`,
//! `[net]` and `[perf]` debug lines the engine emits.
//!
//! Only installed by benchmark tests; normal tests do not install a logger.

use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

pub struct Capture {
    start: Mutex<Instant>,
    lines: Mutex<Vec<(u128, String)>>,
}

static CAPTURE: OnceLock<Arc<Capture>> = OnceLock::new();

struct Bridge(Arc<Capture>);

impl log::Log for Bridge {
    fn enabled(&self, _: &log::Metadata<'_>) -> bool {
        true
    }
    fn log(&self, record: &log::Record<'_>) {
        let at = self
            .0
            .start
            .lock()
            .map(|s| s.elapsed().as_millis())
            .unwrap_or(0);
        if let Ok(mut lines) = self.0.lines.lock() {
            lines.push((at, format!("{}", record.args())));
        }
    }
    fn flush(&self) {}
}

/// Install (once) and return the shared capture.
pub fn init() -> Arc<Capture> {
    CAPTURE
        .get_or_init(|| {
            let capture = Arc::new(Capture {
                start: Mutex::new(Instant::now()),
                lines: Mutex::new(Vec::new()),
            });
            let _ = log::set_boxed_logger(Box::new(Bridge(capture.clone())));
            log::set_max_level(log::LevelFilter::Debug);
            capture
        })
        .clone()
}

impl Capture {
    /// Clear buffered lines and restart the relative clock.
    pub fn reset(&self) {
        if let Ok(mut lines) = self.lines.lock() {
            lines.clear();
        }
        if let Ok(mut start) = self.start.lock() {
            *start = Instant::now();
        }
    }

    pub fn lines(&self) -> Vec<(u128, String)> {
        self.lines.lock().map(|l| l.clone()).unwrap_or_default()
    }

    /// Milliseconds (relative to the last `reset`) at which a line containing
    /// `needle` was logged.
    pub fn at_ms(&self, needle: &str) -> Option<u128> {
        self.lines()
            .into_iter()
            .find(|(_, m)| m.contains(needle))
            .map(|(t, _)| t)
    }

    /// First line containing `needle`.
    pub fn line(&self, needle: &str) -> Option<String> {
        self.lines()
            .into_iter()
            .find(|(_, m)| m.contains(needle))
            .map(|(_, m)| m)
    }
}
