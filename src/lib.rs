//! Core library for the `GeneratePrevisibines` Rust port.
//!
//! External tool automation is intentionally behind wrapper traits so workflow logic
//! can be unit-tested without Creation Kit / `FO4Edit` installed.

pub mod cli;
pub mod config;
pub mod discovery;
pub mod error;
pub mod installation_lock;
pub mod intake;
pub mod interactive;
pub mod run;
pub mod toolchain;
pub mod tools;
pub mod validation;
pub mod workflow;

mod files;
mod text;

// Crate-private because every entry point now takes a `FileSpace`, and that seam is
// crate-private by design; the session log is internal machinery, not part of the API.
pub(crate) mod logging;

// Crate-private like `logging`: a Build Warning is raised by a Workflow Operation and surfaced
// by the collector as it is raised, so nothing outside the crate ever holds one.
pub(crate) mod warning;

pub use cli::Cli;
pub use config::{ArchiveTool, BuildMode, ProjectConfig, WorkflowStep};
pub use error::{Error, Result};
pub use intake::{InteractiveWorkflowIntakePrompts, WorkflowIntakeOutcome, WorkflowRequestIntake};
pub use run::{RunDiagnostic, RunStopped, WorkflowRequest, WorkflowRun};
pub use toolchain::{
    PluginReadiness, ToolchainDiagnostic, ToolchainRequirements, WorkflowToolchain,
    WorkflowToolchainProbe,
};
pub use workflow::WorkflowPlan;
