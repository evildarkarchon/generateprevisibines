//! Workflow Request Intake.
//!
//! This module owns the pre-run decision flow from parsed command-line choices
//! to either a prepared Workflow Run or a deliberate user exit.

use std::path::Path;

use crate::cli::Cli;
use crate::config::{BuildMode, PluginIdentity, WorkflowStep};
use crate::error::{Error, Result};
use crate::interactive::{self, ExistingPluginAction};
use crate::run::{WorkflowRequest, WorkflowRun};
use crate::toolchain::{PluginReadiness, WorkflowToolchainProbe};

/// Result of Workflow Request Intake.
#[derive(Debug, Clone)]
pub enum WorkflowIntakeOutcome {
    /// A Workflow Run is prepared and ready to execute.
    Ready(Box<WorkflowRun>),
    /// The user deliberately exited during intake prompts.
    Exited,
}

/// Prompt adapter used by Workflow Request Intake.
pub trait WorkflowIntakePrompts {
    /// Prompt for the plugin identity, or return `None` when the user exits.
    fn prompt_plugin_name(&self, build_mode: BuildMode) -> Result<Option<PluginIdentity>>;

    /// Ensure the plugin is ready before full Workflow Run preparation.
    fn ensure_plugin_ready(&self, readiness: &PluginReadiness) -> Result<ExistingPluginAction>;

    /// Prompt for the Workflow Step to resume from, or return `None` to re-prompt plugin intake.
    fn prompt_resume_step(&self, build_mode: BuildMode) -> Result<Option<WorkflowStep>>;
}

/// Terminal-backed prompt adapter for production intake.
#[derive(Debug, Default, Clone, Copy)]
pub struct InteractiveWorkflowIntakePrompts;

impl WorkflowIntakePrompts for InteractiveWorkflowIntakePrompts {
    fn prompt_plugin_name(&self, build_mode: BuildMode) -> Result<Option<PluginIdentity>> {
        interactive::prompt_plugin_name(build_mode)
    }

    fn ensure_plugin_ready(&self, readiness: &PluginReadiness) -> Result<ExistingPluginAction> {
        interactive::ensure_plugin_ready(readiness)
    }

    fn prompt_resume_step(&self, build_mode: BuildMode) -> Result<Option<WorkflowStep>> {
        interactive::prompt_resume_step(build_mode)
    }
}

/// Turns Workflow Requests into ready Workflow Runs.
#[derive(Debug, Clone)]
pub struct WorkflowRequestIntake<P> {
    prompts: P,
}

impl WorkflowRequestIntake<InteractiveWorkflowIntakePrompts> {
    /// Create Workflow Request Intake backed by terminal prompts.
    #[must_use]
    pub const fn interactive() -> Self {
        Self::new(InteractiveWorkflowIntakePrompts)
    }
}

impl<P: WorkflowIntakePrompts> WorkflowRequestIntake<P> {
    /// Create Workflow Request Intake backed by the provided prompt adapter.
    #[must_use]
    pub const fn new(prompts: P) -> Self {
        Self { prompts }
    }

    /// Resolve parsed choices and probed toolchain facts into a Workflow Run or a deliberate exit.
    pub fn resolve(
        &self,
        cli: &Cli,
        exe_dir: &Path,
        probe: &WorkflowToolchainProbe,
    ) -> Result<WorkflowIntakeOutcome> {
        let Some(request) = self.resolve_request(cli, probe)? else {
            return Ok(WorkflowIntakeOutcome::Exited);
        };

        let run = WorkflowRun::prepare(&request, exe_dir, probe)?;
        Ok(WorkflowIntakeOutcome::Ready(Box::new(run)))
    }

    fn resolve_request(
        &self,
        cli: &Cli,
        probe: &WorkflowToolchainProbe,
    ) -> Result<Option<WorkflowRequest>> {
        let build_mode = cli.build_mode();
        let archive_tool = cli.archive_tool();

        if let Some(plugin_name) = &cli.plugin {
            let request = WorkflowRequest::new(
                build_mode,
                archive_tool,
                PluginIdentity::parse(plugin_name),
                true,
                cli.resume_from,
                cli.fo4_dir.clone(),
            );
            let readiness = Self::plugin_readiness(&request, probe)?;

            match self.prompts.ensure_plugin_ready(&readiness)? {
                ExistingPluginAction::Exit => return Ok(None),
                ExistingPluginAction::ChooseResumeStep => {
                    return Err(Error::Other(
                        "resume step selection requires interactive mode".into(),
                    ));
                }
                ExistingPluginAction::Continue => {}
            }

            return Ok(Some(request));
        }

        let mut resume_from = cli.resume_from;

        loop {
            let Some(plugin) = self.prompts.prompt_plugin_name(build_mode)? else {
                return Ok(None);
            };

            let mut request = WorkflowRequest::new(
                build_mode,
                archive_tool,
                plugin,
                false,
                resume_from,
                cli.fo4_dir.clone(),
            );
            let readiness = Self::plugin_readiness(&request, probe)?;

            match self.prompts.ensure_plugin_ready(&readiness)? {
                ExistingPluginAction::Exit => return Ok(None),
                ExistingPluginAction::ChooseResumeStep => {
                    if let Some(step) = self.prompts.prompt_resume_step(build_mode)? {
                        request.resume_from = Some(step);
                    } else {
                        resume_from = None;
                        continue;
                    }
                }
                ExistingPluginAction::Continue => {}
            }

            return Ok(Some(request));
        }
    }

    fn plugin_readiness(
        request: &WorkflowRequest,
        probe: &WorkflowToolchainProbe,
    ) -> Result<PluginReadiness> {
        crate::validation::validate_plugin(&request.plugin, request.build_mode)?;
        Ok(probe.plugin_readiness(&request.plugin, request.non_interactive))
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::path::PathBuf;

    use clap::Parser;

    use super::*;
    use crate::config::{ArchiveTool, WorkflowStep};

    #[derive(Debug, Default)]
    struct RecordingPrompts {
        plugin_names: RefCell<Vec<Option<PluginIdentity>>>,
        ready_actions: RefCell<Vec<ExistingPluginAction>>,
        resume_steps: RefCell<Vec<Option<WorkflowStep>>>,
    }

    impl RecordingPrompts {
        fn with_plugin_names(self, names: Vec<Option<PluginIdentity>>) -> Self {
            self.plugin_names.replace(names);
            self
        }

        fn with_ready_actions(self, actions: Vec<ExistingPluginAction>) -> Self {
            self.ready_actions.replace(actions);
            self
        }

        fn with_resume_steps(self, steps: Vec<Option<WorkflowStep>>) -> Self {
            self.resume_steps.replace(steps);
            self
        }
    }

    impl WorkflowIntakePrompts for RecordingPrompts {
        fn prompt_plugin_name(&self, _build_mode: BuildMode) -> Result<Option<PluginIdentity>> {
            Ok(self.plugin_names.borrow_mut().remove(0))
        }

        fn ensure_plugin_ready(
            &self,
            _readiness: &PluginReadiness,
        ) -> Result<ExistingPluginAction> {
            Ok(self.ready_actions.borrow_mut().remove(0))
        }

        fn prompt_resume_step(&self, _build_mode: BuildMode) -> Result<Option<WorkflowStep>> {
            Ok(self.resume_steps.borrow_mut().remove(0))
        }
    }

    #[test]
    fn cli_plugin_continue_yields_non_interactive_request() {
        let cli = Cli::try_parse_from(["generateprevisibines", "--filtered", "MyMod"]).unwrap();
        let intake = WorkflowRequestIntake::new(
            RecordingPrompts::default().with_ready_actions(vec![ExistingPluginAction::Continue]),
        );
        let probe = WorkflowToolchainProbe::from_tool_paths(crate::discovery::ToolPaths {
            fallout4_dir: Some(PathBuf::from(r"C:\Fallout4")),
            ..crate::discovery::ToolPaths::default()
        })
        .unwrap();

        let request = intake.resolve_request(&cli, &probe).unwrap().unwrap();

        assert_eq!(request.build_mode, BuildMode::Filtered);
        assert_eq!(request.archive_tool, ArchiveTool::Archive2);
        assert_eq!(request.plugin.file_name, "MyMod.esp");
        assert!(request.non_interactive);
    }

    #[test]
    fn non_interactive_resume_selection_is_rejected() {
        let cli = Cli::try_parse_from(["generateprevisibines", "MyMod"]).unwrap();
        let intake = WorkflowRequestIntake::new(
            RecordingPrompts::default()
                .with_ready_actions(vec![ExistingPluginAction::ChooseResumeStep]),
        );
        let probe = WorkflowToolchainProbe::from_tool_paths(crate::discovery::ToolPaths {
            fallout4_dir: Some(PathBuf::from(r"C:\Fallout4")),
            ..crate::discovery::ToolPaths::default()
        })
        .unwrap();

        let err = intake.resolve_request(&cli, &probe).unwrap_err();

        assert!(matches!(err, Error::Other(message) if message.contains("interactive mode")));
    }

    #[test]
    fn interactive_resume_reprompt_clears_initial_resume_choice() {
        let cli = Cli::try_parse_from(["generateprevisibines", "--resume-from", "6", "--filtered"])
            .unwrap();
        let intake = WorkflowRequestIntake::new(
            RecordingPrompts::default()
                .with_plugin_names(vec![
                    Some(PluginIdentity::parse("FirstMod")),
                    Some(PluginIdentity::parse("SecondMod")),
                ])
                .with_ready_actions(vec![
                    ExistingPluginAction::ChooseResumeStep,
                    ExistingPluginAction::Continue,
                ])
                .with_resume_steps(vec![None]),
        );
        let probe = WorkflowToolchainProbe::from_tool_paths(crate::discovery::ToolPaths {
            fallout4_dir: Some(PathBuf::from(r"C:\Fallout4")),
            ..crate::discovery::ToolPaths::default()
        })
        .unwrap();

        let request = intake.resolve_request(&cli, &probe).unwrap().unwrap();

        assert_eq!(request.plugin.file_name, "SecondMod.esp");
        assert_eq!(request.resume_from, None);
        assert!(!request.non_interactive);
    }

    #[test]
    fn interactive_resume_choice_updates_request() {
        let cli = Cli::try_parse_from(["generateprevisibines"]).unwrap();
        let intake = WorkflowRequestIntake::new(
            RecordingPrompts::default()
                .with_plugin_names(vec![Some(PluginIdentity::parse("MyMod"))])
                .with_ready_actions(vec![ExistingPluginAction::ChooseResumeStep])
                .with_resume_steps(vec![Some(WorkflowStep::GeneratePrevis)]),
        );
        let probe = WorkflowToolchainProbe::from_tool_paths(crate::discovery::ToolPaths {
            fallout4_dir: Some(PathBuf::from(r"C:\Fallout4")),
            ..crate::discovery::ToolPaths::default()
        })
        .unwrap();

        let request = intake.resolve_request(&cli, &probe).unwrap().unwrap();

        assert_eq!(request.plugin.file_name, "MyMod.esp");
        assert_eq!(request.resume_from, Some(WorkflowStep::GeneratePrevis));
        assert!(!request.non_interactive);
    }
}
