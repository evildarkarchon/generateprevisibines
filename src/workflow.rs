//! Workflow state machine for the 8-step batch process.

use crate::config::{BuildMode, ProjectConfig, WorkflowStep};
use crate::error::Result;
use crate::tools::{ToolContext, ToolRunner};

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
        let all = WorkflowStep::steps_for_mode(config.build_mode);
        let Some(resume) = config.resume_from else {
            return all.to_vec();
        };

        all.iter().copied().filter(|step| *step >= resume).collect()
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
        let steps = WorkflowEngine::<crate::tools::NoopRunner>::planned_steps(&sample_config(
            BuildMode::Clean,
            None,
        ));
        assert_eq!(steps.len(), 8);
    }

    #[test]
    fn filtered_mode_skips_psg_and_cdx() {
        let steps = WorkflowEngine::<crate::tools::NoopRunner>::planned_steps(&sample_config(
            BuildMode::Filtered,
            None,
        ));
        assert_eq!(steps.len(), 6);
        assert!(!steps.contains(&WorkflowStep::CompressPsg));
        assert!(!steps.contains(&WorkflowStep::BuildCdx));
    }

    #[test]
    fn resume_from_step_filters_earlier_steps() {
        let steps = WorkflowEngine::<crate::tools::NoopRunner>::planned_steps(&sample_config(
            BuildMode::Clean,
            Some(WorkflowStep::GeneratePrevis),
        ));
        assert_eq!(steps.first(), Some(&WorkflowStep::GeneratePrevis));
        assert!(!steps.contains(&WorkflowStep::GeneratePrecombines));
    }
}
