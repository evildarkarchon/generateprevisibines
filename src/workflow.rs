//! Workflow state machine for the 8-step batch process.

use crate::config::{BuildMode, ProjectConfig, WorkflowStep};
use crate::error::{Error, Result};
use crate::tools::{ToolContext, ToolRunner};

/// Plain description of which workflow steps a runner adapter can execute.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunnerCapability {
    runnable_steps: &'static [WorkflowStep],
}

impl RunnerCapability {
    /// Create a runner capability from the fixed steps supported by an adapter.
    ///
    /// The slice is static because runner capability is adapter-level behavior, not
    /// per-run state. Use [`WorkflowPlan`] to filter a specific Workflow Run.
    #[must_use]
    pub const fn new(runnable_steps: &'static [WorkflowStep]) -> Self {
        Self { runnable_steps }
    }

    /// Steps the runner adapter can execute.
    #[must_use]
    pub const fn runnable_steps(self) -> &'static [WorkflowStep] {
        self.runnable_steps
    }

    /// Return the subset of planned steps this capability can execute.
    #[must_use]
    pub fn filter_steps(self, steps: &[WorkflowStep]) -> Vec<WorkflowStep> {
        steps
            .iter()
            .copied()
            .filter(|step| self.can_run(*step))
            .collect()
    }

    fn can_run(self, step: WorkflowStep) -> bool {
        self.runnable_steps.contains(&step)
    }
}

/// Ordered steps for a Workflow Run after build mode, resume, and capability are applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowPlan {
    planned_steps: Vec<WorkflowStep>,
    runnable_steps: Vec<WorkflowStep>,
}

impl WorkflowPlan {
    /// Build the Workflow Plan for a resolved config and runner capability.
    ///
    /// Returns [`Error::StepNotImplemented`] when the requested resume step is not
    /// runnable by the capability, or when none of the planned steps can run.
    pub fn new(config: &ProjectConfig, capability: RunnerCapability) -> Result<Self> {
        let planned_steps = Self::planned_steps_for_config(config);
        let runnable_steps = capability.filter_steps(&planned_steps);

        if let Some(resume) = config.resume_from {
            if !capability.can_run(resume) {
                return Err(Error::StepNotImplemented(resume.number()));
            }
        }

        if runnable_steps.is_empty() {
            return Err(Error::StepNotImplemented(
                planned_steps.first().map_or(1, |step| step.number()),
            ));
        }

        Ok(Self {
            planned_steps,
            runnable_steps,
        })
    }

    /// Steps this Workflow Run will attempt, honoring build mode and resume.
    #[must_use]
    pub fn planned_steps_for_config(config: &ProjectConfig) -> Vec<WorkflowStep> {
        let all = WorkflowStep::steps_for_mode(config.build_mode);
        let Some(resume) = config.resume_from else {
            return all.to_vec();
        };

        all.iter().copied().filter(|step| *step >= resume).collect()
    }

    /// Steps belonging to this Workflow Run, including currently unrunnable steps.
    #[must_use]
    pub fn planned_steps(&self) -> &[WorkflowStep] {
        &self.planned_steps
    }

    /// Steps this Workflow Run can execute with the selected runner capability.
    #[must_use]
    pub fn runnable_steps(&self) -> &[WorkflowStep] {
        &self.runnable_steps
    }

    /// Number of planned steps skipped because the runner cannot execute them yet.
    #[must_use]
    pub fn skipped_unrunnable_count(&self) -> usize {
        self.planned_steps.len() - self.runnable_steps.len()
    }

    /// Whether this plan is partial because the runner lacks later-step capability.
    #[must_use]
    pub fn is_partial_due_to_capability(&self) -> bool {
        self.skipped_unrunnable_count() > 0
    }
}

/// Orchestrates workflow steps; external tools are invoked through [`ToolRunner`].
pub struct WorkflowEngine<R: ToolRunner> {
    runner: R,
}

impl<R: ToolRunner> WorkflowEngine<R> {
    #[must_use]
    pub const fn new(runner: R) -> Self {
        Self { runner }
    }

    /// Steps that will run for this configuration, honoring resume and build mode.
    #[must_use]
    pub fn planned_steps(config: &ProjectConfig) -> Vec<WorkflowStep> {
        WorkflowPlan::planned_steps_for_config(config)
    }

    /// Print the resume menu labels (batch `:GetStep`).
    pub fn print_resume_menu(build_mode: BuildMode) {
        println!();
        for step in WorkflowStep::steps_for_mode(build_mode) {
            println!("[{}] {}", step.number(), step.label());
        }
    }

    /// Execute the given steps (subset of [`Self::planned_steps`]).
    pub fn run_steps(
        &self,
        steps: &[WorkflowStep],
        config: &ProjectConfig,
        ctx: &ToolContext,
    ) -> Result<()> {
        for step in steps {
            tracing::info!(step = step.number(), "{}", step.label());
            self.runner.run_step(*step, config, ctx)?;
        }
        Ok(())
    }

    /// Execute all planned steps for this configuration.
    pub fn run(&self, config: &ProjectConfig, ctx: &ToolContext) -> Result<()> {
        let steps = Self::planned_steps(config);
        self.run_steps(&steps, config, ctx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ArchiveTool, PluginIdentity};
    use std::path::PathBuf;

    fn all_steps_capability() -> RunnerCapability {
        RunnerCapability::new(WorkflowStep::steps_for_mode(BuildMode::Clean))
    }

    fn sample_config(mode: BuildMode, resume: Option<WorkflowStep>) -> ProjectConfig {
        ProjectConfig {
            build_mode: mode,
            archive_tool: ArchiveTool::Archive2,
            fallout4_dir: PathBuf::from("C:\\Fallout4"),
            plugin: PluginIdentity::parse("TestMod"),
            non_interactive: true,
            resume_from: resume,
            fo4edit_path: None,
            xedit_data_dir: None,
            ck_log_path: None,
        }
    }

    #[test]
    fn clean_mode_includes_eight_steps() {
        let config = sample_config(BuildMode::Clean, None);
        let plan = WorkflowPlan::new(&config, all_steps_capability()).unwrap();
        assert_eq!(plan.planned_steps().len(), 8);
    }

    #[test]
    fn filtered_mode_skips_psg_and_cdx() {
        let config = sample_config(BuildMode::Filtered, None);
        let plan = WorkflowPlan::new(&config, all_steps_capability()).unwrap();
        let steps = plan.planned_steps();

        assert_eq!(steps.len(), 6);
        assert!(!steps.contains(&WorkflowStep::CompressPsg));
        assert!(!steps.contains(&WorkflowStep::BuildCdx));
    }

    #[test]
    fn resume_from_step_filters_earlier_steps() {
        let config = sample_config(BuildMode::Clean, Some(WorkflowStep::GeneratePrevis));
        let plan = WorkflowPlan::new(&config, all_steps_capability()).unwrap();
        let steps = plan.planned_steps();

        assert_eq!(steps.first(), Some(&WorkflowStep::GeneratePrevis));
        assert!(!steps.contains(&WorkflowStep::GeneratePrecombines));
    }

    #[test]
    fn capability_filters_runnable_steps() {
        let config = sample_config(BuildMode::Clean, None);
        let capability = RunnerCapability::new(&[WorkflowStep::GeneratePrecombines]);
        let plan = WorkflowPlan::new(&config, capability).unwrap();

        assert_eq!(plan.runnable_steps(), &[WorkflowStep::GeneratePrecombines]);
        assert_eq!(plan.skipped_unrunnable_count(), 7);
        assert!(plan.is_partial_due_to_capability());
    }

    #[test]
    fn resume_to_unrunnable_step_keeps_current_error() {
        let config = sample_config(BuildMode::Clean, Some(WorkflowStep::GeneratePrevis));
        let capability = RunnerCapability::new(&[WorkflowStep::GeneratePrecombines]);
        let err = WorkflowPlan::new(&config, capability).unwrap_err();

        assert!(matches!(err, Error::StepNotImplemented(6)));
    }

    #[test]
    fn resume_to_mode_skipped_step_reports_requested_step() {
        let config = sample_config(BuildMode::Filtered, Some(WorkflowStep::CompressPsg));
        let capability = RunnerCapability::new(&[WorkflowStep::GeneratePrecombines]);
        let err = WorkflowPlan::new(&config, capability).unwrap_err();

        assert!(matches!(err, Error::StepNotImplemented(4)));
    }

    #[test]
    fn no_runnable_steps_errors_on_first_planned_step() {
        let config = sample_config(BuildMode::Clean, Some(WorkflowStep::GeneratePrevis));
        let capability = RunnerCapability::new(&[]);
        let err = WorkflowPlan::new(&config, capability).unwrap_err();

        assert!(matches!(err, Error::StepNotImplemented(6)));
    }
}
