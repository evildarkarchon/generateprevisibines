//! Wrappers for external tools.
//!
//! See `docs/workarounds.md` — do not remove MO2 delays, DLL renaming, FO4Edit
//! keystroke automation, or Archive2 extract-repack behavior when implementing these.

mod archive;
mod creation_kit;
mod dll;
mod fo4edit;
mod precomb;

pub use archive::ArchiveOps;
pub use creation_kit::{CkOperation, CreationKitOps};
pub use dll::DllGuard;
pub use fo4edit::Fo4EditOps;

use crate::config::{ProjectConfig, WorkflowStep};
use crate::error::{Error, Result};

/// Shared context passed to each tool invocation.
#[derive(Debug, Clone)]
pub struct ToolContext {
    pub session_log: Option<std::path::PathBuf>,
    pub unattended_log: Option<std::path::PathBuf>,
    pub fallout4_dir: std::path::PathBuf,
    pub creation_kit: std::path::PathBuf,
    pub ck_log_path: Option<std::path::PathBuf>,
}

impl Default for ToolContext {
    fn default() -> Self {
        Self {
            session_log: None,
            unattended_log: None,
            fallout4_dir: std::path::PathBuf::new(),
            creation_kit: std::path::PathBuf::new(),
            ck_log_path: None,
        }
    }
}

/// Abstraction over external tool execution for workflow orchestration.
pub trait ToolRunner {
    fn run_step(&self, step: WorkflowStep, config: &ProjectConfig, ctx: &ToolContext)
        -> Result<()>;
}

/// No-op runner used in tests and `--dry-run`.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoopRunner;

impl ToolRunner for NoopRunner {
    fn run_step(
        &self,
        step: WorkflowStep,
        _config: &ProjectConfig,
        _ctx: &ToolContext,
    ) -> Result<()> {
        tracing::debug!(step = step.number(), "noop tool runner");
        Ok(())
    }
}

/// Scaffold runner that records step intent without launching processes.
#[derive(Debug, Clone, Copy, Default)]
pub struct ScaffoldRunner;

impl ToolRunner for ScaffoldRunner {
    fn run_step(
        &self,
        step: WorkflowStep,
        config: &ProjectConfig,
        ctx: &ToolContext,
    ) -> Result<()> {
        tracing::info!(
            step = step.number(),
            mode = config.build_mode.as_str(),
            archiver = config.archive_tool.program_name(),
            plugin = %config.plugin.file_name,
            log = ?ctx.session_log,
            "scaffold: step not yet implemented"
        );
        Ok(())
    }
}

/// Production runner — Step 1 implemented; later steps fail fast.
#[derive(Debug, Default)]
pub struct ProductionRunner {
    ck: CreationKitOps,
}

impl ProductionRunner {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            ck: CreationKitOps,
        }
    }
}

impl ToolRunner for ProductionRunner {
    fn run_step(
        &self,
        step: WorkflowStep,
        config: &ProjectConfig,
        ctx: &ToolContext,
    ) -> Result<()> {
        match step {
            WorkflowStep::GeneratePrecombines => precomb::run_generate_precombines(config, ctx, &self.ck),
            _ => Err(Error::StepNotImplemented(step.number())),
        }
    }
}

/// Steps this runner can execute (Step 1 only until later slices land).
#[must_use]
pub fn filter_runnable_steps(steps: &[WorkflowStep]) -> Vec<WorkflowStep> {
    steps
        .iter()
        .copied()
        .filter(|s| *s == WorkflowStep::GeneratePrecombines)
        .collect()
}

/// Error when resume targets a step that is not implemented yet.
pub fn assert_resume_step_implemented(config: &ProjectConfig) -> Result<()> {
    if let Some(resume) = config.resume_from {
        if resume != WorkflowStep::GeneratePrecombines {
            return Err(Error::StepNotImplemented(resume.number()));
        }
    }
    Ok(())
}

#[cfg(test)]
mod runner_tests {
    use super::*;
    use crate::config::BuildMode;

    #[test]
    fn filters_to_step_one_only() {
        let all = WorkflowStep::steps_for_mode(BuildMode::Clean).to_vec();
        let runnable = filter_runnable_steps(&all);
        assert_eq!(runnable, vec![WorkflowStep::GeneratePrecombines]);
    }
}
