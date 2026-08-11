//! Prepared workflow runs.
//!
//! This module turns user intent into a validated, executable workflow run while
//! leaving prompts and presentation to adapters such as `main`.

use std::path::{Path, PathBuf};

use crate::config::{ArchiveTool, BuildMode, PluginIdentity, ProjectConfig, WorkflowStep};
use crate::error::Result;
use crate::files::FileSpace;
use crate::logging;
use crate::toolchain::{ToolchainDiagnostic, WorkflowToolchainProbe};
use crate::tools::ToolContext;
use crate::validation;
use crate::workflow::WorkflowPlan;
use crate::workflow::operations::{
    OperationAdapters, ProductionOperationAdapters, ProductionWorkflowPreparation,
    execute_registered_workflow, prepare_production_workflow,
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
    ctx: ToolContext,
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

        let ctx = if requirements.needs_creation_kit() {
            toolchain.creation_kit_context(log_path.clone(), files)?
        } else {
            ToolContext {
                session_log: Some(log_path.clone()),
                unattended_log: Some(logging::unattended_log_path(files)),
                fallout4_dir: config.fallout4_dir.clone(),
                ..ToolContext::default()
            }
        };

        Ok(Self {
            config,
            ctx,
            plan,
            diagnostics,
            log_path,
        })
    }

    /// Execute the runnable subset of the prepared workflow through production operations.
    pub fn execute(&self) -> Result<()> {
        self.execute_with_adapters(&ProductionOperationAdapters::new())
    }

    /// Execute this prepared run through production registration with crate-private adapters.
    ///
    /// The supplied adapters replace only external programs and prompts; the prepared Workflow
    /// Plan and registered Workflow Operations continue to own sequencing and domain behavior.
    pub(crate) fn execute_with_adapters(&self, adapters: &dyn OperationAdapters) -> Result<()> {
        execute_registered_workflow(self, adapters)
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

    /// Tool invocation context for Workflow Operation adapters.
    pub(crate) const fn tool_context(&self) -> &ToolContext {
        &self.ctx
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::PluginIdentity;
    use crate::discovery::ToolPaths;
    use crate::error::Error;
    use crate::files::InMemoryFileSpace;
    use crate::workflow::operations::recording_adapters::{
        RecordedCreationKitCall, RecordingOperationAdapters,
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
            run.tool_context().ck_log_path,
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
    /// The subject here is the Workflow Run's own dispatch, so checking for files the fake
    /// wrote moments earlier — through the same accessors the assertions used — would only
    /// report confidence this test has not earned.
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
        let adapters = RecordingOperationAdapters::new();

        assert_eq!(run.runnable_steps(), &[WorkflowStep::GeneratePrecombines]);
        run.execute_with_adapters(&adapters).unwrap();

        assert_eq!(
            adapters.creation_kit_calls(),
            vec![RecordedCreationKitCall {
                plugin_file: "MyMod.esp".to_owned(),
                build_mode: BuildMode::Clean,
            }]
        );
        // A fresh fixture has nothing to clear, so the resume prompt must never fire. This
        // is a dispatch fact about the run, not an artifact check: the fake established no
        // meshes, so nothing here asserts something the test itself put in place.
        assert_eq!(adapters.clear_prompt_count(), 0);
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

        let error = WorkflowRun::prepare(
            &request,
            fixture.directory.path(),
            &probe,
            &fixture.files,
        )
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

        assert_eq!(run.tool_context().ck_log_path, fo4.join("CK.log"));
    }
}
