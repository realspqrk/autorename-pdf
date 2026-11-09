//! AutoRename-PDF core: deterministic business logic shared by CLI and GUI.
//!
//! Functional style: plain functions over data structs/enums. No I/O beyond
//! what each module documents (config/harmonize/journal touch the filesystem).

pub mod config;
pub mod config_edit;
pub mod dates;
pub mod enrich;
pub mod error;
pub mod extract;
pub mod fsx;
pub mod harmonize;
pub mod journal;
pub mod naming;
pub mod ocr;
pub mod pipeline;
pub mod profiles;
pub mod providers;
mod textutil;

pub use error::{Error, ExitCode, Result};
