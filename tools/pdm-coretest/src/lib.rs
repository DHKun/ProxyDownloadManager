//! Headless harness for ProxyDownloadManager's download core.
//!
//! The Tauri GUI stack cannot be built in this environment (no GTK/WebKit dev
//! libraries, no pkg-config), but every module that actually moves bytes —
//! engine, network pool, probe, worker pool — is Tauri-free. This crate
//! compiles those *same source files* so their unit tests run against the real
//! code, together with the local benchmark server.
//!
//! Production code is not modified by this crate; it is a test-only consumer.
//! `<name>/mod.rs` shims exist only where a real single-file module (types.rs,
//! state/) declares children, because `#[path]`-including such a file makes its
//! children resolve to the wrong directory.

pub mod types;
pub mod state;

#[path = "../../../src-tauri/src/network/mod.rs"]
pub mod network;

#[path = "../../../src-tauri/src/engine/mod.rs"]
pub mod engine;

#[path = "../../../src-tauri/src/retry.rs"]
pub mod retry;

#[path = "../../../src-tauri/src/headers.rs"]
pub mod headers;

#[path = "../../../src-tauri/src/filename.rs"]
pub mod filename;

#[path = "../../../src-tauri/src/probe.rs"]
pub mod probe;

#[path = "../../../src-tauri/src/worker.rs"]
pub mod worker;

#[path = "../../../src-tauri/src/config.rs"]
pub mod config;

/// Local HTTP server used by reuse/throughput/adaptive tests.
pub mod testserver;

#[cfg(test)]
mod caplog;

#[cfg(test)]
mod perf_tests;

#[cfg(test)]
mod real_bench;
