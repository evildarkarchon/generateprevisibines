//! Prepared workflow runs.
//!
//! This module turns user intent into a validated, executable workflow run while
//! leaving prompts and presentation to adapters such as `main`.

use std::path::{Path, PathBuf};

use crate::config::{ArchiveTool, BuildMode, PluginIdentity, ProjectConfig, WorkflowStep};
use crate::error::{Error, Result};
use crate::files::{FileSpace, SystemFileSpace};
use crate::logging;
use crate::toolchain::{ToolchainDiagnostic, WorkflowToolchainProbe};
use crate::tools::CreationKitPaths;
use crate::tools::process::SystemProcessRunner;
use crate::tools::wait::SystemWait;
use crate::validation;
use crate::workflow::WorkflowPlan;
use crate::workflow::operations::{
    InteractivePrompts, OperationPorts, ProductionWorkflowPreparation, execute_registered_workflow,
    prepare_production_workflow,
};

/// User intent before tool paths, CKPE configuration, logs, or runnable steps are resolved.
#[derive(Debug, Clone)]
pub struct WorkflowRequest {
    pub build_mode: BuildMode,
    pub archive_tool: ArchiveTool,
    pub plugin: PluginIdentity,
    pub non_interactive: bool,
    pub resume_from: Option<WorkflowStep>,
    pub fallout4_override: Option<PathBuf>,
}

impl WorkflowRequest {
    /// Create a request from already-parsed user choices.
    #[must_use]
    pub const fn new(
        build_mode: BuildMode,
        archive_tool: ArchiveTool,
        plugin: PluginIdentity,
        non_interactive: bool,
        resume_from: Option<WorkflowStep>,
        fallout4_override: Option<PathBuf>,
    ) -> Self {
        Self {
            build_mode,
            archive_tool,
            plugin,
            non_interactive,
            resume_from,
            fallout4_override,
        }
    }

    /// Build the resolved project config once data-root facts have been prepared.
    pub fn to_project_config(&self, probe: &WorkflowToolchainProbe) -> Result<ProjectConfig> {
        validation::validate_plugin(&self.plugin, self.build_mode)?;

        Ok(ProjectConfig {
            build_mode: self.build_mode,
            archive_tool: self.archive_tool,
            fallout4_dir: probe.fallout4_dir().to_path_buf(),
            data_dir: probe.data_dir().to_path_buf(),
            plugin: self.plugin.clone(),
            non_interactive: self.non_interactive,
            resume_from: self.resume_from,
        })
    }
}

/// Structured information discovered while preparing a workflow run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunDiagnostic {
    Toolchain(ToolchainDiagnostic),
    LaterStepsNotImplemented { skipped: usize, planned: usize },
}

/// A prepared build attempt ready to execute through Workflow Operations.
#[derive(Debug, Clone)]
pub struct WorkflowRun {
    config: ProjectConfig,
    /// Resolved Creation Kit paths, or `None` when no runnable step required Creation Kit.
    creation_kit: Option<CreationKitPaths>,
    plan: WorkflowPlan,
    diagnostics: Vec<RunDiagnostic>,
    log_path: PathBuf,
}

impl WorkflowRun {
    /// Prepare a workflow run from the production-backed Workflow Plan.
    ///
    /// `files` is the space the session log is created in; it is borrowed only for the
    /// duration of preparation, because nothing the prepared run does later touches it.
    ///
    /// Returns the fully validated run, or propagates request validation, unimplemented resume,
    /// toolchain readiness, log initialization, and Creation Kit context errors.
    pub(crate) fn prepare(
        request: &WorkflowRequest,
        exe_dir: &Path,
        probe: &WorkflowToolchainProbe,
        files: &dyn FileSpace,
    ) -> Result<Self> {
        let config = request.to_project_config(probe)?;
        let preparation = prepare_production_workflow(config.build_mode, config.resume_from)?;
        Self::prepare_with_registration(config, exe_dir, probe, preparation, files)
    }

    /// Complete preparation from a validated config and registration-resolved workflow state.
    ///
    /// Returns the ready-to-execute run, or propagates toolchain readiness, log initialization,
    /// and Creation Kit context errors. The plan and requirements must come from the same
    /// Workflow Operation registration before this helper is called.
    fn prepare_with_registration(
        config: ProjectConfig,
        exe_dir: &Path,
        probe: &WorkflowToolchainProbe,
        preparation: ProductionWorkflowPreparation,
        files: &dyn FileSpace,
    ) -> Result<Self> {
        let (plan, requirements) = preparation.into_parts();
        let toolchain = probe.prepare(exe_dir, config.archive_tool, requirements)?;
        let mut diagnostics = toolchain
            .diagnostics()
            .iter()
            .cloned()
            .map(RunDiagnostic::Toolchain)
            .collect::<Vec<_>>();
        if plan.skipped_unrunnable_count() > 0 {
            diagnostics.push(RunDiagnostic::LaterStepsNotImplemented {
                skipped: plan.skipped_unrunnable_count(),
                planned: plan.planned_steps().len(),
            });
        }

        let log_path = logging::session_log_path(&config.plugin, files);
        logging::init_session_log(
            &log_path,
            config.build_mode.as_str(),
            &config.plugin.file_name,
            files,
        )?;

        // Absent rather than empty when Creation Kit was never required: there is no usable
        // Creation Kit path set for a run that did not ask for one, and saying so with `None`
        // is what keeps unresolved paths out of an episode that would then spawn them.
        let creation_kit = if requirements.needs_creation_kit() {
            Some(toolchain.creation_kit_paths(log_path.clone())?)
        } else {
            None
        };

        Ok(Self {
            config,
            creation_kit,
            plan,
            diagnostics,
            log_path,
        })
    }

    /// Execute the runnable subset of the prepared workflow through the production ports.
    ///
    /// Returns [`Error::CreationKitNotPrepared`] when the prepared run carries no Creation Kit
    /// paths, and otherwise propagates whatever the dispatched Workflow Operations report.
    pub fn execute(&self) -> Result<()> {
        let files = SystemFileSpace;
        let process = SystemProcessRunner;
        let wait = SystemWait;
        let prompts = InteractivePrompts;

        // Every registered Workflow Operation requires Creation Kit today, so a prepared run
        // that reaches here without its paths is a preparation bug rather than a user state.
        // The check is not removable as dead code, though: the moment an xEdit-backed operation
        // is registered, a plan can be runnable without Creation Kit ever being resolved.
        let ck = self
            .creation_kit()
            .ok_or(Error::CreationKitNotPrepared)?
            .bind(&process, &wait, &files);

        self.execute_with_ports(&OperationPorts {
            ck: &ck,
            prompts: &prompts,
            files: &files,
        })
    }

    /// Execute this prepared run through production registration with crate-private ports.
    ///
    /// The supplied ports replace only external programs, prompts and the filesystem; the
    /// prepared Workflow Plan and registered Workflow Operations continue to own sequencing
    /// and domain behavior.
    pub(crate) fn execute_with_ports(&self, ports: &OperationPorts<'_>) -> Result<()> {
        execute_registered_workflow(self, ports)
    }

    /// Diagnostics collected while preparing the run.
    #[must_use]
    pub fn diagnostics(&self) -> &[RunDiagnostic] {
        &self.diagnostics
    }

    /// Workflow Plan resolved for this run.
    #[must_use]
    pub const fn plan(&self) -> &WorkflowPlan {
        &self.plan
    }

    /// Full planned workflow, including steps that may not be runnable in the scaffold.
    #[must_use]
    pub fn planned_steps(&self) -> &[WorkflowStep] {
        self.plan.planned_steps()
    }

    /// Steps that will be executed by registered Workflow Operations.
    #[must_use]
    pub fn runnable_steps(&self) -> &[WorkflowStep] {
        self.plan.runnable_steps()
    }

    /// Session log path initialized for this run.
    #[must_use]
    pub fn log_path(&self) -> &Path {
        &self.log_path
    }

    /// Resolved project configuration for this run.
    #[must_use]
    pub const fn config(&self) -> &ProjectConfig {
        &self.config
    }

    /// Resolved paths this run's Creation Kit episodes run against.
    ///
    /// `None` when no runnable Workflow Operation required Creation Kit readiness, so nothing
    /// was resolved. Deliberately narrow: the ports an episode runs through are chosen at
    /// execution time, so a Workflow Operation is never handed the run back to fish them out.
    pub(crate) const fn creation_kit(&self) -> Option<&CreationKitPaths> {
        self.creation_kit.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::PluginIdentity;
    use crate::discovery::ToolPaths;
    use crate::error::Error;
    use crate::files::InMemoryFileSpace;
    use crate::tools::process::RecordingProcessRunner;
    use crate::tools::wait::{MO2_DELAY_AFTER_CK_SECS, RecordingWait};
    use crate::workflow::operations::recording_adapters::{
        RecordingPrompts, record_successful_precombine_outputs,
    };
    use std::fs;
    use tempfile::{TempDir, tempdir};

    struct ReadyWorkflowFixture {
        directory: TempDir,
        fallout4_directory: PathBuf,
        probe: WorkflowToolchainProbe,
        request: WorkflowRequest,
        /// The space the prepared run's session log lands in.
        ///
        /// Owned per fixture, which is what keeps the several test modules that all prepare a
        /// run for the plugin `MyMod` off one shared `%TEMP%\MyMod.log`.
        files: InMemoryFileSpace,
    }

    /// Build a real filesystem fixture with the production Step 1 toolchain ready.
    fn ready_workflow_fixture() -> ReadyWorkflowFixture {
        let directory = tempdir().unwrap();
        let fallout4_directory = directory.path().join("Fallout4");
        fs::create_dir_all(&fallout4_directory).unwrap();
        fs::write(fallout4_directory.join("CreationKit.exe"), b"").unwrap();
        fs::write(
            fallout4_directory.join("fallout4_test.ini"),
            "[CreationKit]\nBSHandleRefObjectPatch=true\n[CreationKit_Log]\nOutputFile=CK.log\n",
        )
        .unwrap();

        let probe = WorkflowToolchainProbe::from_tool_paths(ToolPaths {
            fallout4_dir: Some(fallout4_directory.clone()),
            creation_kit: Some(fallout4_directory.join("CreationKit.exe")),
            ..ToolPaths::default()
        })
        .unwrap();
        let request = WorkflowRequest::new(
            BuildMode::Clean,
            ArchiveTool::Archive2,
            PluginIdentity::parse("MyMod"),
            true,
            None,
            None,
        );

        ReadyWorkflowFixture {
            directory,
            fallout4_directory,
            probe,
            request,
            files: InMemoryFileSpace::new(),
        }
    }

    #[test]
    fn request_to_project_config_uses_probe_data_dir() {
        let request = WorkflowRequest::new(
            BuildMode::Filtered,
            ArchiveTool::BSArch,
            PluginIdentity::parse("MyMod"),
            true,
            None,
            Some(PathBuf::from(r"D:\Games\Fallout4")),
        );
        let probe = WorkflowToolchainProbe::from_tool_paths(ToolPaths {
            fallout4_dir: Some(PathBuf::from(r"D:\Games\Fallout4")),
            ..ToolPaths::default()
        })
        .unwrap();

        let config = request.to_project_config(&probe).unwrap();

        assert_eq!(config.build_mode, BuildMode::Filtered);
        assert_eq!(config.archive_tool, ArchiveTool::BSArch);
        assert_eq!(config.data_dir, PathBuf::from(r"D:\Games\Fallout4\Data"));
    }

    #[test]
    fn prepare_creates_runnable_step_one_run() {
        let fixture = ready_workflow_fixture();

        let run = WorkflowRun::prepare(
            &fixture.request,
            fixture.directory.path(),
            &fixture.probe,
            &fixture.files,
        )
        .unwrap();

        assert_eq!(run.runnable_steps(), &[WorkflowStep::GeneratePrecombines]);
        assert_eq!(
            run.plan().runnable_steps(),
            &[WorkflowStep::GeneratePrecombines]
        );
        assert!(run.planned_steps().len() > run.runnable_steps().len());
        assert!(
            run.diagnostics()
                .contains(&RunDiagnostic::LaterStepsNotImplemented {
                    skipped: 7,
                    planned: 8,
                })
        );
        assert_eq!(
            run.creation_kit().unwrap().ck_log_path,
            fixture.fallout4_directory.join("CK.log")
        );
        // The session log is initialized in the fixture's own space, not the machine's
        // `%TEMP%`, so concurrent test modules preparing a `MyMod` run cannot collide.
        assert_eq!(run.log_path(), fixture.files.temp_dir().join("MyMod.log"));
        assert_eq!(
            fixture.files.read_lossy(run.log_path()).unwrap(),
            "Starting clean Build V2.95 of MyMod.esp\n"
        );
    }

    /// The prepared run dispatches Step 1 and hands Creation Kit the run's Clean build mode.
    ///
    /// Artifact assertions belong to the Workflow Operation and Precombine Workspace tests.
    /// The subject here is the Workflow Run's own dispatch, so checking for files the simulated
    /// Creation Kit wrote moments earlier — through the same accessors the assertions used —
    /// would only report confidence this test has not earned.
    #[test]
    fn prepared_run_dispatches_step_one_with_the_runs_clean_build_mode() {
        let fixture = ready_workflow_fixture();
        let run = WorkflowRun::prepare(
            &fixture.request,
            fixture.directory.path(),
            &fixture.probe,
            &fixture.files,
        )
        .unwrap();
        let config = run.config().clone();
        let ck_log = run.creation_kit().unwrap().ck_log_path.clone();
        let process = RecordingProcessRunner::new().with_effects(&fixture.files, move |space| {
            record_successful_precombine_outputs(space, &config, &ck_log);
        });
        let wait = RecordingWait::new();
        let prompts = RecordingPrompts::new();
        let ck = run
            .creation_kit()
            .unwrap()
            .bind(&process, &wait, &fixture.files);

        assert_eq!(run.runnable_steps(), &[WorkflowStep::GeneratePrecombines]);
        run.execute_with_ports(&OperationPorts {
            ck: &ck,
            prompts: &prompts,
            files: &fixture.files,
        })
        .unwrap();

        // One spawn, of the run's own Creation Kit, naming the run's own plugin. What the
        // Clean build mode turns into on that command line is asserted where the mapping
        // lives, in `tools::creation_kit`; the subject here is the run's dispatch.
        let calls = process.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].exe, run.creation_kit().unwrap().exe);
        assert!(
            calls[0]
                .args
                .iter()
                .any(|arg| arg.to_string_lossy().contains("MyMod.esp")),
            "args: {:?}",
            calls[0].args
        );
        // Dispatching Step 1 dispatches the whole episode, mandated delay included.
        assert_eq!(wait.delays(), vec![MO2_DELAY_AFTER_CK_SECS]);
        // A fresh fixture has nothing to clear, so the resume prompt must never fire. This is
        // a dispatch fact about the run, not an artifact check: the simulated Creation Kit
        // established no prior meshes, so nothing here asserts what the test put in place.
        assert_eq!(prompts.clear_prompt_count(), 0);
    }

    #[test]
    fn prepare_requires_creation_kit_for_registered_generate_precombines() {
        let directory = tempdir().unwrap();
        let fallout4_directory = directory.path().join("Fallout4");
        fs::create_dir_all(&fallout4_directory).unwrap();
        let probe = WorkflowToolchainProbe::from_tool_paths(ToolPaths {
            fallout4_dir: Some(fallout4_directory.clone()),
            ..ToolPaths::default()
        })
        .unwrap();
        let request = WorkflowRequest::new(
            BuildMode::Clean,
            ArchiveTool::Archive2,
            PluginIdentity::parse("MyMod"),
            true,
            None,
            None,
        );

        let error = WorkflowRun::prepare(
            &request,
            directory.path(),
            &probe,
            &InMemoryFileSpace::new(),
        )
        .unwrap_err();

        assert!(matches!(
            error,
            Error::Other(message)
                if message
                    == format!(
                        "CreationKit.exe not found in {}",
                        fallout4_directory.display()
                    )
        ));
    }

    #[test]
    fn prepare_preserves_filtered_and_xbox_partial_diagnostic_counts() {
        let cases = [(BuildMode::Filtered, 5, 6), (BuildMode::Xbox, 5, 6)];

        for (build_mode, skipped, planned) in cases {
            let fixture = ready_workflow_fixture();
            let mut request = fixture.request.clone();
            request.build_mode = build_mode;

            let run = WorkflowRun::prepare(
                &request,
                fixture.directory.path(),
                &fixture.probe,
                &fixture.files,
            )
            .unwrap();

            assert!(
                run.diagnostics()
                    .contains(&RunDiagnostic::LaterStepsNotImplemented { skipped, planned }),
                "build mode: {build_mode:?}, diagnostics: {:?}",
                run.diagnostics()
            );
        }
    }

    #[test]
    fn prepare_rejects_unavailable_explicit_resume_before_toolchain_readiness() {
        let fixture = ready_workflow_fixture();
        let mut request = fixture.request.clone();
        request.resume_from = Some(WorkflowStep::MergePrecombineObjects);
        let probe = WorkflowToolchainProbe::from_tool_paths(ToolPaths {
            fallout4_dir: Some(fixture.fallout4_directory.clone()),
            ..ToolPaths::default()
        })
        .unwrap();

        let error =
            WorkflowRun::prepare(&request, fixture.directory.path(), &probe, &fixture.files)
                .unwrap_err();

        assert!(matches!(error, Error::StepNotImplemented(2)));
    }

    #[test]
    fn prepare_tolerates_non_utf8_ckpe_config_bytes() {
        let dir = tempdir().unwrap();
        let fo4 = dir.path().join("Fallout4");
        fs::create_dir_all(&fo4).unwrap();
        fs::write(fo4.join("CreationKit.exe"), b"").unwrap();
        fs::write(
            fo4.join("fallout4_test.ini"),
            b"[CreationKit]\nBSHandleRefObjectPatch=true\n[CreationKit_Log]\nOutputFile=CK.log\n;\xFF\n",
        )
        .unwrap();

        let probe = WorkflowToolchainProbe::from_tool_paths(ToolPaths {
            fallout4_dir: Some(fo4.clone()),
            creation_kit: Some(fo4.join("CreationKit.exe")),
            ..ToolPaths::default()
        })
        .unwrap();
        let request = WorkflowRequest::new(
            BuildMode::Clean,
            ArchiveTool::Archive2,
            PluginIdentity::parse("MyMod"),
            true,
            None,
            None,
        );

        let run =
            WorkflowRun::prepare(&request, dir.path(), &probe, &InMemoryFileSpace::new()).unwrap();

        assert_eq!(run.creation_kit().unwrap().ck_log_path, fo4.join("CK.log"));
    }
}
