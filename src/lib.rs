//! Core library for the `GeneratePrevisibines` Rust port.
//!
//! External tool automation is intentionally behind wrapper traits so workflow logic
//! can be unit-tested without Creation Kit / `FO4Edit` installed.

pub mod cli;
pub mod config;
pub mod discovery;
pub mod error;
pub mod intake;
pub mod interactive;
pub mod logging;
pub mod run;
pub mod timing;
pub mod toolchain;
pub mod tools;
pub mod validation;
pub mod workflow;

mod text;

pub use cli::Cli;
pub use config::{ArchiveTool, BuildMode, ProjectConfig, WorkflowStep};
pub use error::{Error, Result};
pub use intake::{
    InteractiveWorkflowIntakePrompts, WorkflowIntakeOutcome, WorkflowIntakePrompts,
    WorkflowRequestIntake,
};
pub use run::{RunDiagnostic, WorkflowRequest, WorkflowRun};
pub use toolchain::{
    PluginReadiness, ToolchainDiagnostic, ToolchainRequirements, WorkflowToolchain,
    WorkflowToolchainProbe,
};
pub use workflow::operations::{
    OperationAdapters, ProductionOperationAdapters, WorkflowOperationExecutor,
};
pub use workflow::{OperationCapability, WorkflowPlan};
