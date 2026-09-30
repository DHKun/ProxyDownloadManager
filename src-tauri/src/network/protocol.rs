//! Protocol and startup/perf instrumentation.
//!
//! These are DEBUG-level only: nothing here is persisted to the DB and nothing
//! is emitted in release unless the logger is configured for debug. The point
//! is to answer "did HTTP/2 actually get negotiated?" and "where did the first
//! bytes spend their time?" without logging per packet.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// Human label for the protocol a response was actually served over.
pub fn protocol_label(version: reqwest::Version) -> &'static str {
    match version {
        reqwest::Version::HTTP_09 => "http/0.9",
        reqwest::Version::HTTP_10 => "http/1.0",
        reqwest::Version::HTTP_11 => "http/1.1",
        reqwest::Version::HTTP_2 => "h2",
        reqwest::Version::HTTP_3 => "h3",
        _ => "unknown",
    }
}

/// Per-download startup + retry/stall counters.
///
/// Times are measured from engine start, so the difference between
/// `first-header`, `first-body` and `first-progress` shows where the latency
/// sits: TCP/TLS+headers, first body byte, or limiter/buffer/write.
pub struct PerfStats {
    id: u64,
    started: Instant,
    header_logged: AtomicBool,
    body_logged: AtomicBool,
    progress_logged: AtomicBool,
    protocol: OnceLock<String>,
    pub retries: AtomicU64,
    pub stalls: AtomicU64,
}

impl PerfStats {
    pub fn new(id: u64) -> Self {
        Self {
            id,
            started: Instant::now(),
            header_logged: AtomicBool::new(false),
            body_logged: AtomicBool::new(false),
            progress_logged: AtomicBool::new(false),
            protocol: OnceLock::new(),
            retries: AtomicU64::new(0),
            stalls: AtomicU64::new(0),
        }
    }

    fn elapsed_ms(&self) -> u128 {
        self.started.elapsed().as_millis()
    }

    pub fn note_header(&self, version: reqwest::Version) {
        let _ = self.protocol.set(protocol_label(version).to_string());
        if !self.header_logged.swap(true, Ordering::Relaxed) {
            log::debug!(
                "[startup] first-header id={} at={}ms protocol={}",
                self.id,
                self.elapsed_ms(),
                protocol_label(version)
            );
        }
    }

    pub fn note_body(&self) {
        if !self.body_logged.swap(true, Ordering::Relaxed) {
            log::debug!(
                "[startup] first-body id={} at={}ms",
                self.id,
                self.elapsed_ms()
            );
        }
    }

    pub fn note_progress(&self) {
        if !self.progress_logged.swap(true, Ordering::Relaxed) {
            log::debug!(
                "[startup] first-progress id={} at={}ms",
                self.id,
                self.elapsed_ms()
            );
        }
    }

    pub fn note_retry(&self) {
        self.retries.fetch_add(1, Ordering::Relaxed);
    }

    pub fn note_stall(&self) {
        self.stalls.fetch_add(1, Ordering::Relaxed);
    }

    pub fn protocol(&self) -> String {
        self.protocol
            .get()
            .cloned()
            .unwrap_or_else(|| "unknown".to_string())
    }

    pub fn errors(&self) -> u64 {
        self.retries.load(Ordering::Relaxed) + self.stalls.load(Ordering::Relaxed)
    }

    pub fn log_summary(&self, connections: u32, bytes_written: u64) {
        let secs = self.started.elapsed().as_secs_f64().max(0.001);
        log::debug!(
            "[perf] id={} protocol={} connections={} speed={:.1}MB/s retries={} stalls={}",
            self.id,
            self.protocol(),
            connections,
            bytes_written as f64 / secs / (1024.0 * 1024.0),
            self.retries.load(Ordering::Relaxed),
            self.stalls.load(Ordering::Relaxed)
        );
    }

    pub fn summary_interval() -> Duration {
        Duration::from_secs(5)
    }
}
