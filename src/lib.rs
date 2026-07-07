//! Core library for the GeneratePrevisibines Rust port.
//!
//! External tool automation is intentionally behind wrapper traits so workflow logic
//! can be unit-tested without Creation Kit / FO4Edit installed.

pub mod checks;
pub mod cli;
pub mod config;
pub mod discovery;
pub mod error;
pub mod interactive;
pub mod logging;
pub mod run;
pub mod timing;
pub mod tools;
pub mod validation;
pub mod workflow;

pub use cli::Cli;
pub use config::{ArchiveTool, BuildMode, ProjectConfig, WorkflowStep};
pub use error::{Error, Result};
pub use run::{RunDiagnostic, WorkflowRequest, WorkflowRun};
pub use workflow::{RunnerCapability, WorkflowPlan};
