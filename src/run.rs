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
use crate::tools::clock::SystemClock;
use crate::tools::desktop::SystemDesktopWindows;
use crate::tools::process::SystemProcessRunner;
use crate::tools::wait::SystemWait;
use crate::tools::{CkPorts, CreationKitPaths, Fo4EditPaths, Fo4EditPorts};
use crate::validation;
use crate::warning::BuildWarnings;
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
    ) -> Self {
        Self {
            build_mode,
            archive_tool,
            plugin,
            non_interactive,
            resume_from,
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
    /// The plan has steps with no registered Workflow Operation yet, so the run stops short.
    ///
    /// Carries the `runnable` steps the run will execute, because since Step 4 was registered
    /// a resume can run something other than Step 1, and the message has to say which.
    LaterStepsNotImplemented {
        skipped: usize,
        planned: usize,
        runnable: Vec<WorkflowStep>,
    },
}

/// A Workflow Run that stopped, already reported on the console and in the session log.
///
/// Returned instead of the bare [`Error`] so `main` can tell an error the run has reported —
/// `ERROR - …`, `Build of Patch … failed.` and `See Log at …` — from one raised before any run
/// existed, which still needs its `ERROR - …` printed. Printing it twice is what this prevents.
#[derive(Debug)]
pub struct RunStopped {
    error: Error,
}

impl RunStopped {
    /// The error that stopped the run.
    #[must_use]
    pub const fn error(&self) -> &Error {
        &self.error
    }
}

/// A prepared build attempt ready to execute through Workflow Operations.
#[derive(Debug, Clone)]
pub struct WorkflowRun {
    config: ProjectConfig,
    /// Resolved Creation Kit paths, or `None` when no runnable step required Creation Kit.
    creation_kit: Option<CreationKitPaths>,
    /// Resolved FO4Edit paths, or `None` when no runnable step required FO4Edit.
    fo4edit: Option<Fo4EditPaths>,
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
    /// and Creation Kit or FO4Edit context errors. The plan and requirements must come from the same
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
                runnable: plan.runnable_steps().to_vec(),
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
        // The same for FO4Edit: a run that resumes past both merge steps never resolves it.
        let fo4edit = if requirements.needs_fo4edit() {
            Some(toolchain.fo4edit_paths(log_path.clone())?)
        } else {
            None
        };

        Ok(Self {
            config,
            creation_kit,
            fo4edit,
            plan,
            diagnostics,
            log_path,
        })
    }

    /// Execute the runnable subset of the prepared workflow through the production ports.
    ///
    /// Every exit is reported before this returns; see [`Self::execute_with_ports`]. Only the tool
    /// episodes this run prepared are bound; an operation that reaches one that was not stops
    /// the run with [`Error::CreationKitNotPrepared`] or [`Error::Fo4EditNotPrepared`], reported
    /// the same way.
    ///
    /// # Errors
    ///
    /// Returns [`RunStopped`] when the run stops, carrying the error that stopped it.
    pub fn execute(&self) -> std::result::Result<(), RunStopped> {
        let files = SystemFileSpace;
        let process = SystemProcessRunner;
        let wait = SystemWait;
        let clock = SystemClock;
        let desktop = SystemDesktopWindows;
        let prompts = InteractivePrompts;
        let warnings = BuildWarnings::new(self.log_path.clone(), &files);

        // Bind each episode whose paths were prepared, and leave the other absent rather than
        // failing up front: a resume at Step 2 or Step 7 needs FO4Edit and no Creation Kit. An
        // operation that reaches an absent episode stops through `ports.ck()?` or
        // `ports.fo4edit()?`, and `execute_with_ports` reports that stop like any other, since
        // the session log exists by now.
        let ck = self.creation_kit().map(|creation_kit| {
            creation_kit.bind(CkPorts {
                process: &process,
                wait: &wait,
                clock: &clock,
                files: &files,
            })
        });
        let fo4edit = self.fo4edit().map(|fo4edit| {
            fo4edit.bind(Fo4EditPorts {
                process: &process,
                wait: &wait,
                desktop: &desktop,
                files: &files,
            })
        });

        self.execute_with_ports(&OperationPorts {
            ck: ck.as_ref(),
            fo4edit: fo4edit.as_ref(),
            prompts: &prompts,
            files: &files,
            warnings: &warnings,
        })
    }

    /// Execute this prepared run through production registration with crate-private ports.
    ///
    /// The supplied ports replace only external programs, prompts and the filesystem; the
    /// prepared Workflow Plan and registered Workflow Operations continue to own sequencing
    /// and domain behavior.
    ///
    /// Owns how the run ends, on the console and in the session log. A completed run prints
    /// `Build step(s) complete.` then `See Log at …` as its last line; a stopped one is reported
    /// by [`Self::report_stop`]. Both happen here rather than in `main` so that Finish, once
    /// ported, can run ahead of `See Log at` (batch 368 follows `:Fin`).
    ///
    /// # Errors
    ///
    /// Returns [`RunStopped`] when a dispatched Workflow Operation stops the run.
    pub(crate) fn execute_with_ports(
        &self,
        ports: &OperationPorts<'_>,
    ) -> std::result::Result<(), RunStopped> {
        if let Err(error) = execute_registered_workflow(self, ports) {
            return Err(self.report_stop(error, ports.files));
        }

        println!("Build step(s) complete.");
        println!("{}", logging::see_log_line(&self.log_path));
        Ok(())
    }

    /// Report a stopped run: `ERROR - <error>`, `Build of Patch <name> failed.`, then
    /// `See Log at <path>`, each once and in that order (batch `:Failed` -> `:Done`, 376 -> 368).
    ///
    /// The first two are also appended to the session log — an additive divergence (the batch
    /// only echoes them), so a log read later explains why the run ended. A failed append is
    /// noted and otherwise ignored: the error being reported is what the operator needs, and an
    /// unwritable session log must not replace it.
    fn report_stop(&self, error: Error, files: &dyn FileSpace) -> RunStopped {
        let error_line = logging::error_line(&error);
        let failed_line = logging::build_failed_line(&self.config.plugin.base_name);

        // Stderr for the error, where `main` prints the errors raised before a run exists.
        eprintln!("{error_line}");
        println!("{failed_line}");

        for line in [&error_line, &failed_line] {
            if let Err(append_error) = logging::append_log_line(&self.log_path, line, files) {
                tracing::warn!("Could not record the failure in the session log: {append_error}");
            }
        }

        println!("{}", logging::see_log_line(&self.log_path));
        RunStopped { error }
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

    /// Resolved paths this run's FO4Edit episodes run against.
    ///
    /// `None` when no runnable Workflow Operation required FO4Edit readiness, so nothing was
    /// resolved. Narrow for the same reason as [`Self::creation_kit`].
    pub(crate) const fn fo4edit(&self) -> Option<&Fo4EditPaths> {
        self.fo4edit.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::PluginIdentity;
    use crate::discovery::ToolPaths;
    use crate::error::Error;
    use crate::files::InMemoryFileSpace;
    use crate::tools::clock::ScriptedClock;
    use crate::tools::process::{RecordedProcessCall, RecordingProcessRunner};
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
                    runnable: vec![WorkflowStep::GeneratePrecombines],
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
            "Starting clean Build V2.99 of MyMod.esp\n"
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
        // The subject here is the run's dispatch, not the session log, so a clock that never
        // moves is all this needs.
        let clock = ScriptedClock::fixed();
        let prompts = RecordingPrompts::new();
        let ck = run.creation_kit().unwrap().bind(CkPorts {
            process: &process,
            wait: &wait,
            clock: &clock,
            files: &fixture.files,
        });
        let warnings = BuildWarnings::new(run.log_path().to_path_buf(), &fixture.files);

        assert_eq!(run.runnable_steps(), &[WorkflowStep::GeneratePrecombines]);
        run.execute_with_ports(&OperationPorts {
            ck: Some(&ck),
            fo4edit: None,
            prompts: &prompts,
            files: &fixture.files,
            warnings: &warnings,
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
        assert_eq!(prompts.asked(), []);
    }

    /// Execute a prepared run's Step 1 over recording ports, with `process` standing in for
    /// Creation Kit, and hand back how the run ended.
    ///
    /// The ports are assembled here rather than in each test because `CreationKitOps` and the
    /// warnings collector borrow the fixture's space, so the chain has to live in one frame.
    fn execute_over_recording_ports(
        run: &WorkflowRun,
        files: &InMemoryFileSpace,
        process: &RecordingProcessRunner<'_>,
    ) -> std::result::Result<(), RunStopped> {
        let wait = RecordingWait::new();
        let clock = ScriptedClock::fixed();
        let prompts = RecordingPrompts::new();
        let ck = run.creation_kit().unwrap().bind(CkPorts {
            process,
            wait: &wait,
            clock: &clock,
            files,
        });
        let warnings = BuildWarnings::new(run.log_path().to_path_buf(), files);

        run.execute_with_ports(&OperationPorts {
            ck: Some(&ck),
            fo4edit: None,
            prompts: &prompts,
            files,
            warnings: &warnings,
        })
    }

    /// Prepare the fixture's Clean Step 1 request into a ready Workflow Run.
    fn prepared_run(fixture: &ReadyWorkflowFixture) -> WorkflowRun {
        WorkflowRun::prepare(
            &fixture.request,
            fixture.directory.path(),
            &fixture.probe,
            &fixture.files,
        )
        .unwrap()
    }

    /// A stopped run records why it ended: the error, then the batch's failure line, in that
    /// order, as the last two lines of the session log.
    #[test]
    fn a_stopped_run_appends_its_error_and_failure_line_to_the_session_log() {
        let fixture = ready_workflow_fixture();
        let run = prepared_run(&fixture);
        // Creation Kit ran and wrote nothing, so Step 1 stops on its first postcondition.
        let process = RecordingProcessRunner::new();

        let stopped = execute_over_recording_ports(&run, &fixture.files, &process).unwrap_err();

        assert!(matches!(stopped.error(), Error::MissingCombinedObjects));
        let session = fixture.files.read_lossy(run.log_path()).unwrap();
        assert!(
            session.ends_with(
                "ERROR - CombinedObjects.esp was not created by Creation Kit\n\
                 Build of Patch MyMod failed.\n"
            ),
            "session log: {session}"
        );
        // Console-only, as in the batch: the log has no use for its own path.
        assert!(!session.contains("See Log at"), "session log: {session}");
    }

    /// An unwritable session log must not replace the error being reported.
    #[test]
    fn a_failed_session_log_append_never_masks_the_stopping_error() {
        let fixture = ready_workflow_fixture();
        let run = prepared_run(&fixture);
        // A precondition stop, so nothing before the report needs the session log either.
        fixture.files.add_file(run.config().plugin_archive_path());
        fixture.files.refuse_appends_to(run.log_path());
        let process = RecordingProcessRunner::new();

        let stopped = execute_over_recording_ports(&run, &fixture.files, &process).unwrap_err();

        assert!(matches!(stopped.error(), Error::PluginAlreadyHasArchive));
        assert_eq!(process.calls(), Vec::<RecordedProcessCall>::new());
    }

    /// A completed run leaves its session log free of failure lines.
    #[test]
    fn a_completed_run_records_no_failure() {
        let fixture = ready_workflow_fixture();
        let run = prepared_run(&fixture);
        let config = run.config().clone();
        let ck_log = run.creation_kit().unwrap().ck_log_path.clone();
        let process = RecordingProcessRunner::new().with_effects(&fixture.files, move |space| {
            record_successful_precombine_outputs(space, &config, &ck_log);
        });

        execute_over_recording_ports(&run, &fixture.files, &process).unwrap();

        let session = fixture.files.read_lossy(run.log_path()).unwrap();
        assert!(!session.contains("ERROR - "), "session log: {session}");
        assert!(!session.contains("failed."), "session log: {session}");
    }

    /// Step 1 needs no FO4Edit, so none is resolved even where one could be found.
    #[test]
    fn a_run_without_fo4edit_steps_prepares_no_fo4edit() {
        let fixture = ready_workflow_fixture();

        let run = prepared_run(&fixture);

        assert!(run.fo4edit().is_none());
    }

    /// A run whose runnable operations require FO4Edit gets its paths, logging into the run's
    /// own session log, and needs no Creation Kit for it.
    #[test]
    fn a_run_requiring_fo4edit_prepares_its_paths() {
        let fixture = ready_workflow_fixture();
        let fo4edit = fixture.directory.path().join("FO4Edit.exe");
        fs::write(&fo4edit, b"").unwrap();
        let scripts = fixture.directory.path().join("Edit Scripts");
        fs::create_dir_all(&scripts).unwrap();
        fs::write(
            scripts.join("Batch_FO4MergeCombinedObjectsAndCheck.pas"),
            "V1.5",
        )
        .unwrap();
        fs::write(scripts.join("Batch_FO4MergePrevisandCleanRefr.pas"), "V2.3").unwrap();
        let probe = WorkflowToolchainProbe::from_tool_paths(ToolPaths {
            fo4edit: Some(fo4edit.clone()),
            fallout4_dir: Some(fixture.fallout4_directory.clone()),
            ..ToolPaths::default()
        })
        .unwrap();
        let config = fixture.request.to_project_config(&probe).unwrap();
        let plan =
            crate::workflow::operations::production_workflow_plan(BuildMode::Clean, None).unwrap();
        let preparation = ProductionWorkflowPreparation::from_parts(
            plan,
            crate::toolchain::ToolchainRequirements::fo4edit(),
        );

        let run = WorkflowRun::prepare_with_registration(
            config,
            fixture.directory.path(),
            &probe,
            preparation,
            &fixture.files,
        )
        .unwrap();

        let paths = run.fo4edit().unwrap();
        assert_eq!(paths.exe, fo4edit);
        assert_eq!(paths.data_dir_override, None);
        assert_eq!(paths.session_log, run.log_path());
        assert!(run.creation_kit().is_none());
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
        let cases = [(BuildMode::Filtered, 5, 6), (BuildMode::Xbox, 7, 8)];

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
                    .contains(&RunDiagnostic::LaterStepsNotImplemented {
                        skipped,
                        planned,
                        runnable: vec![WorkflowStep::GeneratePrecombines],
                    }),
                "build mode: {build_mode:?}, diagnostics: {:?}",
                run.diagnostics()
            );
        }
    }

    /// A Clean resume at 4 runs Steps 4 to 6, so the diagnostic must say that rather than
    /// claim Step 1 is what runs.
    #[test]
    fn a_resumed_partial_run_names_the_steps_it_will_execute() {
        let fixture = ready_workflow_fixture();
        let mut request = fixture.request.clone();
        request.resume_from = Some(WorkflowStep::CompressPsg);

        let run = WorkflowRun::prepare(
            &request,
            fixture.directory.path(),
            &fixture.probe,
            &fixture.files,
        )
        .unwrap();

        let runnable = vec![
            WorkflowStep::CompressPsg,
            WorkflowStep::BuildCdx,
            WorkflowStep::GeneratePrevis,
        ];
        assert_eq!(run.runnable_steps(), runnable.as_slice());
        assert!(
            run.diagnostics()
                .contains(&RunDiagnostic::LaterStepsNotImplemented {
                    skipped: 2,
                    planned: 5,
                    runnable,
                }),
            "diagnostics: {:?}",
            run.diagnostics()
        );
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
        );

        let run =
            WorkflowRun::prepare(&request, dir.path(), &probe, &InMemoryFileSpace::new()).unwrap();

        assert_eq!(run.creation_kit().unwrap().ck_log_path, fo4.join("CK.log"));
    }
}
