//! Mirror of `src-tauri/src/types.rs`. The real file is a single-file module
//! that declares children, which `#[path]` cannot relocate correctly, so the
//! child files are pointed at explicitly here.

#[path = "../../../../src-tauri/src/types/config.rs"]
pub mod config;

#[path = "../../../../src-tauri/src/types/download.rs"]
pub mod download;

#[path = "../../../../src-tauri/src/types/engine_config.rs"]
pub mod engine_config;

#[path = "../../../../src-tauri/src/types/error.rs"]
pub mod error;

#[path = "../../../../src-tauri/src/types/event.rs"]
pub mod event;

pub use config::*;
pub use download::*;
pub use engine_config::{EngineConfig, ResumePlan};
pub use error::*;
pub use event::*;
