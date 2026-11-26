//! `pptdif`: reading and content-aligned comparison of PowerPoint
//! presentations.
//!
//! The library owns every presentation rule and has no process I/O; the
//! `pptdif-cli` crate adapts it to the process contract. See
//! `ARCHITECTURE.md` for the module map and `PLAN.md` for the milestones.

pub mod align;
mod chart;
pub mod diff;
mod error;
pub mod model;
mod package;
pub mod reader;
pub mod views;
mod xml;

pub use error::{Error, ErrorCode};
pub use package::Limits;
pub use reader::{read_bytes, read_bytes_with, read_path, read_path_with};
