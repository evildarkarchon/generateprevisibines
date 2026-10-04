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
use crate::tools::{
    ArchivePaths, ArchivePorts, CkPorts, CreationKitPaths, Fo4EditPaths, Fo4EditPorts,
    restore_archive_work_folders,
};
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
    /// Resolved archive-tool paths, or `None` when no runnable step required the archive tool.
    archive: Option<ArchivePaths>,
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
    /// and Creation Kit, FO4Edit or archive-tool context errors. The plan and requirements must come from the same
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
        // And for the archive tool, which a resume at Step 8 needs and nothing else. Every
        // production plan now ends at Step 8, so it is always resolved there; the condition
        // stays because readiness follows the plan's requirements, not a list of steps.
        let archive = if requirements.needs_archive() {
            Some(toolchain.archive_paths(log_path.clone())?)
        } else {
            None
        };

        Ok(Self {
            config,
            creation_kit,
            fo4edit,
            archive,
            plan,
            diagnostics,
            log_path,
        })
    }

    /// Execute the runnable subset of the prepared workflow through the production ports.
    ///
    /// Every exit is reported before this returns; see [`Self::execute_with_ports`]. Only the tool
    /// episodes this run prepared are bound; an operation that reaches one that was not stops
    /// the run with [`Error::CreationKitNotPrepared`], [`Error::Fo4EditNotPrepared`] or
    /// [`Error::ArchiveNotPrepared`], reported the same way.
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

        // Bind each episode whose paths were prepared, and leave the others absent rather than
        // failing up front: a resume at Step 7 needs FO4Edit and the archive tool but no
        // Creation Kit, and a resume at Step 8 needs only the archive tool. An operation that
        // reaches an absent episode stops through `ports.ck()?`, `ports.fo4edit()?` or
        // `ports.archive()?`, and `execute_with_ports` reports that stop like any other, since
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
        let archive = self.archive().map(|archive| {
            archive.bind(ArchivePorts {
                process: &process,
                wait: &wait,
                files: &files,
                warnings: &warnings,
            })
        });

        self.execute_with_ports(&OperationPorts {
            ck: ck.as_ref(),
            fo4edit: fo4edit.as_ref(),
            archive: archive.as_ref(),
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
    /// Before the first dispatch, every leftover archive work folder is restored and cleared;
    /// see [`restore_archive_work_folders`].
    ///
    /// # Errors
    ///
    /// Returns [`RunStopped`] when a dispatched Workflow Operation stops the run, or when a
    /// Build Warning the run-start restore raises cannot be appended to the session log.
    pub(crate) fn execute_with_ports(
        &self,
        ports: &OperationPorts<'_>,
    ) -> std::result::Result<(), RunStopped> {
        // On every run, whatever the resume point and whether or not the plan reaches an archive
        // step, and even when no archive tool was prepared: a crashed run may have left the only
        // copy of the user's precombines, `vis` or new archive in a work folder, and this run may
        // not be the one that would have rebuilt it. It needs only the Fallout 4 directory, since
        // each restore-list target is a full path. The installation lock `main` holds for the
        // whole run (ADR-0005) is what makes every folder it finds a dead run's.
        let outcome =
            restore_archive_work_folders(&self.config.fallout4_dir, ports.files, ports.warnings)
                .and_then(|()| execute_registered_workflow(self, ports));
        if let Err(error) = outcome {
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

    /// Resolved paths this run's Archive episodes run against.
    ///
    /// `None` when no runnable Workflow Operation required archive readiness, so nothing was
    /// resolved. Narrow for the same reason as [`Self::creation_kit`].
    pub(crate) const fn archive(&self) -> Option<&ArchivePaths> {
        self.archive.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::PluginIdentity;
    use crate::discovery::ToolPaths;
    use crate::error::Error;
    use crate::files::InMemoryFileSpace;
    use crate::toolchain::{archive2_exe, write_fo4edit_install};
    use crate::tools::clock::ScriptedClock;
    use crate::tools::leftovers;
    use crate::tools::process::{
        ExitFlag, ProcessCallKind, RecordedProcessCall, RecordingProcessRunner, ScriptedCall,
    };
    use crate::tools::wait::{
        FO4EDIT_CLOSE_DELAYS_SECS, FO4EDIT_STARTUP_DELAY_SECS,
        MO2_DELAY_AFTER_ARCHIVE2_EXTRACT_SECS, MO2_DELAY_AFTER_CK_SECS,
        MO2_DELAY_BEFORE_FO4EDIT_SECS, RecordingWait,
    };
    use crate::warning::{BuildWarning, LeftoverItems};
    use crate::workflow::operations::prepare_workflow_without;
    use crate::workflow::operations::recording_adapters::{
        FaultyFileSpace, ONE_POLL_MERGE_DELAYS, PACKED_ARCHIVE, RecordingPrompts,
        archive2_extract_writing_precombines, archive2_pack_writing_archive,
        behaving_fo4edit_windows, fo4edit_exiting_when, record_successful_cdx_outputs,
        record_successful_combined_objects_merge, record_successful_compress_outputs,
        record_successful_precombine_outputs, record_successful_previs_merge,
        record_successful_previs_outputs,
    };
    use std::cell::RefCell;
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

    /// Build a real filesystem fixture with the production toolchain for a fresh run ready:
    /// Creation Kit for Steps 1 and 4 to 6, FO4Edit for Steps 2 and 7, and Archive2 for Steps 3
    /// and 8.
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
            fo4edit: Some(write_fo4edit_install(&directory.path().join("FO4Edit"))),
            archive2: Some(archive2_exe(&fallout4_directory)),
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

    /// The steps a fresh Clean run executes: every one, since Step 8 was registered.
    const FRESH_RUN_STEPS: [WorkflowStep; 8] = [
        WorkflowStep::GeneratePrecombines,
        WorkflowStep::MergePrecombineObjects,
        WorkflowStep::CreateBa2FromPrecombines,
        WorkflowStep::CompressPsg,
        WorkflowStep::BuildCdx,
        WorkflowStep::GeneratePrevis,
        WorkflowStep::MergePrevis,
        WorkflowStep::AddPrevisToArchive,
    ];

    /// Whether `run` was prepared with the diagnostic saying it stops short of its plan.
    fn stops_short(run: &WorkflowRun) -> bool {
        run.diagnostics()
            .iter()
            .any(|diagnostic| matches!(diagnostic, RunDiagnostic::LaterStepsNotImplemented { .. }))
    }

    #[test]
    fn prepare_creates_a_run_of_every_planned_step() {
        let fixture = ready_workflow_fixture();

        let run = WorkflowRun::prepare(
            &fixture.request,
            fixture.directory.path(),
            &fixture.probe,
            &fixture.files,
        )
        .unwrap();

        assert_eq!(run.runnable_steps(), &FRESH_RUN_STEPS);
        assert_eq!(run.plan().runnable_steps(), &FRESH_RUN_STEPS);
        assert_eq!(run.planned_steps(), run.runnable_steps());
        assert!(!stops_short(&run), "diagnostics: {:?}", run.diagnostics());
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

    /// What one execution over recording ports did, for the dispatch assertions.
    struct RecordedExecution {
        result: std::result::Result<(), RunStopped>,
        delays: Vec<u64>,
        prompts_asked: Vec<crate::workflow::operations::Confirmation>,
        /// Every Build Warning the run raised, including the run-start restore's.
        warnings: Vec<BuildWarning>,
    }

    /// Execute a prepared run over recording ports, with `process` standing in for Creation
    /// Kit and FO4Edit, and hand back how the run ended.
    ///
    /// Each episode the run prepared is bound, as `execute` binds them, and the other is left
    /// absent. FO4Edit behaves: its Module Selection is dismissed by its `OK`, its script
    /// leaves a clean log, and it exits when asked to close — provided `process` spawns it with
    /// [`fo4edit_exit`] as its exit flag. The ports are assembled here rather than in each test
    /// because the episodes and the warnings collector borrow the fixture's space, so the chain
    /// has to live in one frame.
    fn execute_over_recording_ports(
        run: &WorkflowRun,
        files: &InMemoryFileSpace,
        process: &RecordingProcessRunner<'_>,
        fo4edit_exit: &ExitFlag,
    ) -> RecordedExecution {
        execute_through(run, files, files, process, fo4edit_exit)
    }

    /// [`execute_over_recording_ports`], with every port seeing `files` through `ports_files`.
    ///
    /// `ports_files` is a [`FaultyFileSpace`] over `files` when a test needs an operation
    /// refused; the simulated tools still write into `files` directly.
    fn execute_through(
        run: &WorkflowRun,
        files: &InMemoryFileSpace,
        ports_files: &dyn FileSpace,
        process: &RecordingProcessRunner<'_>,
        fo4edit_exit: &ExitFlag,
    ) -> RecordedExecution {
        let wait = RecordingWait::new();
        // The subject here is the run's dispatch, not the session log, so a clock that never
        // moves is all this needs.
        let clock = ScriptedClock::fixed();
        let prompts = RecordingPrompts::new();
        let desktop = behaving_fo4edit_windows(fo4edit_exit, || {
            record_successful_combined_objects_merge(files);
        });
        // Shadowed only after the desktop above, whose simulated merge writes straight into the
        // underlying space as the real FO4Edit would.
        let files = ports_files;
        let ck = run.creation_kit().map(|paths| {
            paths.bind(CkPorts {
                process,
                wait: &wait,
                clock: &clock,
                files,
            })
        });
        let fo4edit = run.fo4edit().map(|paths| {
            paths.bind(Fo4EditPorts {
                process,
                wait: &wait,
                desktop: &desktop,
                files,
            })
        });
        let warnings = BuildWarnings::new(run.log_path().to_path_buf(), files);
        let archive = run.archive().map(|paths| {
            paths.bind(ArchivePorts {
                process,
                wait: &wait,
                files,
                warnings: &warnings,
            })
        });

        let result = run.execute_with_ports(&OperationPorts {
            ck: ck.as_ref(),
            fo4edit: fo4edit.as_ref(),
            archive: archive.as_ref(),
            prompts: &prompts,
            files,
            warnings: &warnings,
        });
        RecordedExecution {
            result,
            delays: wait.delays(),
            prompts_asked: prompts.asked(),
            warnings: warnings.raised(),
        }
    }

    /// A process runner under which every runnable step of `run` completes, and whose FO4Edit
    /// spawns exit once `fo4edit_exit` is set.
    ///
    /// The calls each step makes when it succeeds, scripted in plan order: one for every step
    /// but Step 8, whose Archive2 rebuild extracts and then repacks. Each leaves behind what its
    /// step then checks for. Nothing is runner-wide, so an unscripted call leaves nothing.
    ///
    /// Step 2's merge log is written when the desktop's Module Selection is dismissed (see
    /// [`execute_through`]). Step 7's is written at its spawn instead, because the recording
    /// desktop shows Module Selection only once. In a run that also merged at Step 2, that
    /// first FO4Edit's close has already set `fo4edit_exit`, so the second has exited by its
    /// first poll — which is harmless, because the poll finds the log before it asks.
    fn completing_process<'a>(
        run: &WorkflowRun,
        files: &'a InMemoryFileSpace,
        fo4edit_exit: &ExitFlag,
    ) -> RecordingProcessRunner<'a> {
        let config = run.config().clone();
        // Read only when a Creation Kit step is runnable, because a resume past Step 6 prepares
        // no Creation Kit.
        let ck_log = run
            .creation_kit()
            .map(|creation_kit| creation_kit.ck_log_path.clone());
        let ck_call = |record: fn(&InMemoryFileSpace, &ProjectConfig, &Path)| {
            let config = config.clone();
            let ck_log = ck_log
                .clone()
                .expect("a Creation Kit step is runnable only when Creation Kit was prepared");
            ScriptedCall::new().with_effect(files, move |space, _exe, _args| {
                record(space, &config, &ck_log);
            })
        };

        let mut process = fo4edit_exiting_when(fo4edit_exit);
        let mut index = 0;
        for step in run.runnable_steps() {
            let calls = match step {
                WorkflowStep::GeneratePrecombines => {
                    vec![ck_call(record_successful_precombine_outputs)]
                }
                // Its log comes from the desktop, so the spawn itself leaves nothing.
                WorkflowStep::MergePrecombineObjects => vec![ScriptedCall::new()],
                WorkflowStep::CreateBa2FromPrecombines => {
                    vec![archive2_pack_writing_archive(files)]
                }
                WorkflowStep::CompressPsg => vec![ck_call(record_successful_compress_outputs)],
                WorkflowStep::BuildCdx => vec![ck_call(record_successful_cdx_outputs)],
                WorkflowStep::GeneratePrevis => vec![ck_call(record_successful_previs_outputs)],
                WorkflowStep::MergePrevis => {
                    vec![
                        ScriptedCall::new().with_effect(files, |space, _exe, _args| {
                            record_successful_previs_merge(space);
                        }),
                    ]
                }
                WorkflowStep::AddPrevisToArchive => vec![
                    archive2_extract_writing_precombines(
                        files,
                        run.config().fo4edit_data_dir(),
                        false,
                    ),
                    archive2_pack_writing_archive(files),
                ],
            };
            for call in calls {
                process = process.scripting_call(index, call);
                index += 1;
            }
        }
        process
    }

    /// A full Clean run dispatches Steps 1 to 8 in plan order, each through its whole episode:
    /// Creation Kit, FO4Edit, the archive tool, Creation Kit three times, FO4Edit again, then
    /// the archive tool's extract and repack.
    ///
    /// Artifact assertions belong to the Workflow Operation, Precombine Workspace and Archive
    /// episode tests. The subject
    /// here is the Workflow Run's own dispatch, so checking for files the simulated tools wrote
    /// moments earlier — through the same accessors the assertions used — would only report
    /// confidence this test has not earned.
    #[test]
    fn a_full_clean_run_dispatches_steps_one_to_eight_in_order() {
        let fixture = ready_workflow_fixture();
        let run = prepared_run(&fixture);
        let exit = ExitFlag::new();
        let process = completing_process(&run, &fixture.files, &exit);

        assert_eq!(run.runnable_steps(), &FRESH_RUN_STEPS);
        let execution = execute_over_recording_ports(&run, &fixture.files, &process, &exit);
        execution.result.unwrap();

        // Each tool the run's own, and each call naming the run's own plugin or its archive.
        // What the build mode, the merge scripts and the archive verbs turn into on those
        // command lines is asserted where the mappings live, in `tools`.
        let ck = run.creation_kit().unwrap().exe.clone();
        let fo4edit = run.fo4edit().unwrap().exe.clone();
        let archive = run.archive().unwrap().exe.clone();
        let calls = process.calls();
        assert_eq!(
            calls
                .iter()
                .map(|call| (call.exe.clone(), call.kind))
                .collect::<Vec<_>>(),
            vec![
                (ck.clone(), ProcessCallKind::Run),
                (fo4edit.clone(), ProcessCallKind::Spawn),
                (archive.clone(), ProcessCallKind::RunCapturing),
                (ck.clone(), ProcessCallKind::Run),
                (ck.clone(), ProcessCallKind::Run),
                (ck, ProcessCallKind::Run),
                (fo4edit, ProcessCallKind::Spawn),
                (archive.clone(), ProcessCallKind::RunCapturing),
                (archive, ProcessCallKind::RunCapturing),
            ]
        );
        for call in &calls {
            // The archive tool names the run's archive; Creation Kit and FO4Edit its plugin.
            let expected = if call.kind == ProcessCallKind::RunCapturing {
                "MyMod - Main.ba2"
            } else {
                "MyMod.esp"
            };
            assert!(
                call.args
                    .iter()
                    .any(|arg| arg.to_string_lossy().contains(expected)),
                "args: {:?}",
                call.args
            );
        }
        // Dispatching each step dispatches its whole episode, mandated delays included.
        let mut expected_delays = vec![MO2_DELAY_AFTER_CK_SECS];
        expected_delays.extend(ONE_POLL_MERGE_DELAYS);
        // Step 3 with Archive2 waits for nothing; Steps 4 to 6 each wait after Creation Kit.
        expected_delays.extend([MO2_DELAY_AFTER_CK_SECS; 3]);
        // Step 7's log is there by its first poll, so it has no poll wait.
        expected_delays.extend([MO2_DELAY_BEFORE_FO4EDIT_SECS, FO4EDIT_STARTUP_DELAY_SECS]);
        expected_delays.extend(FO4EDIT_CLOSE_DELAYS_SECS);
        // Step 8 with Archive2 waits once, after its extract.
        expected_delays.push(MO2_DELAY_AFTER_ARCHIVE2_EXTRACT_SECS);
        assert_eq!(execution.delays, expected_delays);
        // A fresh fixture has nothing to clear, so neither resume prompt may fire. This is a
        // dispatch fact about the run, not an artifact check: the simulated tools established
        // no prior meshes or previs, so nothing here asserts what the test put in place.
        assert_eq!(execution.prompts_asked, []);
        assert_eq!(execution.warnings, []);
    }

    /// `--resume-from 3` prepares the archive tool and dispatches Step 3 through it first, then
    /// runs on through Step 8.
    ///
    /// Creation Kit and FO4Edit are prepared too, because Steps 4 to 8 are registered and a
    /// resume plans every step after the one it names. That Step 3 itself runs with the archive
    /// tool alone is pinned at the operation, with the other episodes absent.
    #[test]
    fn a_resume_at_step_three_dispatches_step_three_through_the_archive_tool_first() {
        let fixture = ready_workflow_fixture();
        let mut request = fixture.request.clone();
        request.resume_from = Some(WorkflowStep::CreateBa2FromPrecombines);
        let run = WorkflowRun::prepare(
            &request,
            fixture.directory.path(),
            &fixture.probe,
            &fixture.files,
        )
        .unwrap();
        // What Steps 1 and 2 left behind: the loose meshes for Step 3, and Step 4's geometry.
        let data = fixture.fallout4_directory.join("Data");
        fixture.files.add_file(
            data.join("meshes")
                .join("precombined")
                .join("cell")
                .join("mesh.nif"),
        );
        fixture.files.add_file(data.join("MyMod - Geometry.psg"));
        let exit = ExitFlag::new();
        let process = completing_process(&run, &fixture.files, &exit);

        let execution = execute_over_recording_ports(&run, &fixture.files, &process, &exit);

        execution.result.unwrap();
        assert_eq!(run.runnable_steps(), &FRESH_RUN_STEPS[2..]);
        let archive = run.archive().unwrap();
        assert_eq!(archive.tool, ArchiveTool::Archive2);
        assert_eq!(archive.exe, archive2_exe(&fixture.fallout4_directory));
        assert_eq!(archive.session_log, run.log_path());
        let calls = process.calls();
        assert_eq!(calls.len(), 7);
        assert_eq!(calls[0].exe, archive.exe);
        assert_eq!(calls[0].kind, ProcessCallKind::RunCapturing);
        assert_eq!(execution.warnings, []);
    }

    /// [`WorkflowRun::prepare`] under production's registration with `unregistered` taken out;
    /// see [`prepare_workflow_without`].
    ///
    /// For the plans production no longer makes now every step is registered: a run that stops
    /// short of its plan, a resume at a step with no operation, and a plan with no archive step.
    fn prepare_without(
        unregistered: &[WorkflowStep],
        request: &WorkflowRequest,
        exe_dir: &Path,
        probe: &WorkflowToolchainProbe,
        files: &dyn FileSpace,
    ) -> Result<WorkflowRun> {
        let config = request.to_project_config(probe)?;
        let preparation =
            prepare_workflow_without(unregistered, config.build_mode, config.resume_from)?;
        WorkflowRun::prepare_with_registration(config, exe_dir, probe, preparation, files)
    }

    /// A Clean run resumed at Step 7 over an install with FO4Edit and no Creation Kit at all,
    /// prepared and checked to have resolved FO4Edit alone.
    ///
    /// No production plan resolves FO4Edit alone any more: since Step 8 was registered, every
    /// resume runs on through it and needs the archive tool. So this run is prepared under a
    /// registration without Step 8, standing in for a plan with no archive step.
    struct Fo4EditOnlyRun {
        /// The real directory the toolchain probe validated; kept alive for the run's lifetime.
        _directory: TempDir,
        /// `Fallout4\Data`, where the merge steps' inputs are seeded.
        data: PathBuf,
        /// The FO4Edit executable the install holds.
        fo4edit: PathBuf,
        files: InMemoryFileSpace,
        run: WorkflowRun,
    }

    /// Prepare [`Fo4EditOnlyRun`] for a resume at `resume_from`, asserting the run plans
    /// `resume_from` alone and prepared FO4Edit, logging into its own session log, and neither
    /// Creation Kit nor the archive tool.
    ///
    /// The install has no Creation Kit, so a run that still resolved it could not even be
    /// prepared, and one that bound it unconditionally could not get as far as the merge.
    fn prepare_fo4edit_only_run(resume_from: WorkflowStep) -> Fo4EditOnlyRun {
        let directory = tempdir().unwrap();
        let fallout4_directory = directory.path().join("Fallout4");
        fs::create_dir_all(&fallout4_directory).unwrap();
        let fo4edit = write_fo4edit_install(&directory.path().join("FO4Edit"));
        let probe = WorkflowToolchainProbe::from_tool_paths(ToolPaths {
            fallout4_dir: Some(fallout4_directory.clone()),
            fo4edit: Some(fo4edit.clone()),
            ..ToolPaths::default()
        })
        .unwrap();
        let request = WorkflowRequest::new(
            BuildMode::Clean,
            ArchiveTool::Archive2,
            PluginIdentity::parse("MyMod"),
            true,
            Some(resume_from),
        );
        let files = InMemoryFileSpace::new();

        let run = prepare_without(
            &[WorkflowStep::AddPrevisToArchive],
            &request,
            directory.path(),
            &probe,
            &files,
        )
        .unwrap();

        assert_eq!(run.runnable_steps(), &[resume_from]);
        assert!(run.creation_kit().is_none());
        assert!(run.archive().is_none());
        let paths = run.fo4edit().unwrap();
        assert_eq!(paths.exe, fo4edit);
        assert_eq!(paths.data_dir_override, None);
        assert_eq!(paths.session_log, run.log_path());

        Fo4EditOnlyRun {
            _directory: directory,
            data: fallout4_directory.join("Data"),
            fo4edit,
            files,
            run,
        }
    }

    /// Execute a [`Fo4EditOnlyRun`] over a behaving FO4Edit and assert it completed after
    /// exactly one spawn of that FO4Edit, for `script`.
    fn assert_executes_one_fo4edit_merge(fixture: &Fo4EditOnlyRun, script: &str) {
        let exit = ExitFlag::new();
        let process = fo4edit_exiting_when(&exit);

        let execution = execute_over_recording_ports(&fixture.run, &fixture.files, &process, &exit);

        execution.result.unwrap();
        let calls = process.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].exe, fixture.fo4edit);
        assert_eq!(calls[0].kind, ProcessCallKind::Spawn);
        let script_arg = std::ffi::OsString::from(format!("-Script:{script}"));
        assert!(
            calls[0].args.contains(&script_arg),
            "args: {:?}",
            calls[0].args
        );
    }

    /// Seed what Step 6 leaves behind for Step 7 to merge: a `.uvd` and `Previs.esp`.
    fn add_previs_outputs(fixture: &Fo4EditOnlyRun) {
        fixture
            .files
            .add_file(fixture.data.join("vis").join("cell").join("cluster.uvd"));
        fixture.files.add_file(fixture.data.join("Previs.esp"));
    }

    /// A Clean run resumed at `resume_from` over an install with Archive2 and no Creation Kit
    /// at all, and with FO4Edit only when asked for.
    struct NoCreationKitRun {
        /// The real directory the toolchain probe validated; kept alive for the run's lifetime.
        _directory: TempDir,
        /// `Fallout4\Data`, where the later steps' inputs are seeded.
        data: PathBuf,
        /// Where discovery found Archive2.
        archive2: PathBuf,
        files: InMemoryFileSpace,
        run: WorkflowRun,
    }

    /// Prepare [`NoCreationKitRun`] for a resume at `resume_from`, with FO4Edit installed when
    /// `with_fo4edit` is set, and assert it prepared no Creation Kit.
    ///
    /// The install has no Creation Kit, so a run that still resolved it could not even be
    /// prepared.
    fn prepare_no_creation_kit_run(
        resume_from: WorkflowStep,
        with_fo4edit: bool,
    ) -> NoCreationKitRun {
        let directory = tempdir().unwrap();
        let fallout4_directory = directory.path().join("Fallout4");
        fs::create_dir_all(&fallout4_directory).unwrap();
        let archive2 = archive2_exe(&fallout4_directory);
        let probe = WorkflowToolchainProbe::from_tool_paths(ToolPaths {
            fallout4_dir: Some(fallout4_directory.clone()),
            fo4edit: with_fo4edit.then(|| write_fo4edit_install(&directory.path().join("FO4Edit"))),
            archive2: Some(archive2.clone()),
            ..ToolPaths::default()
        })
        .unwrap();
        let request = WorkflowRequest::new(
            BuildMode::Clean,
            ArchiveTool::Archive2,
            PluginIdentity::parse("MyMod"),
            true,
            Some(resume_from),
        );
        let files = InMemoryFileSpace::new();

        let run = WorkflowRun::prepare(&request, directory.path(), &probe, &files).unwrap();

        assert!(run.creation_kit().is_none());
        NoCreationKitRun {
            _directory: directory,
            data: fallout4_directory.join("Data"),
            archive2,
            files,
            run,
        }
    }

    /// Seed what Steps 3 and 6 leave behind for Steps 7 and 8: the Plugin Archive, a `.uvd`
    /// and `Previs.esp`.
    fn add_archive_and_previs_outputs(fixture: &NoCreationKitRun) {
        fixture
            .files
            .add_file_with_contents(fixture.data.join(ARCHIVE_NAME), "old archive");
        fixture
            .files
            .add_file(fixture.data.join("vis").join("cell").join("cluster.uvd"));
        fixture.files.add_file(fixture.data.join("Previs.esp"));
    }

    /// `--resume-from 7` prepares FO4Edit and the archive tool but no Creation Kit, merges
    /// `Previs.esp`, then adds the previs to the archive.
    #[test]
    fn a_resume_at_step_seven_prepares_and_executes_without_creation_kit() {
        let fixture = prepare_no_creation_kit_run(WorkflowStep::MergePrevis, true);
        add_archive_and_previs_outputs(&fixture);
        let exit = ExitFlag::new();
        let process = completing_process(&fixture.run, &fixture.files, &exit);

        let execution = execute_over_recording_ports(&fixture.run, &fixture.files, &process, &exit);

        execution.result.unwrap();
        assert_eq!(
            fixture.run.runnable_steps(),
            &[WorkflowStep::MergePrevis, WorkflowStep::AddPrevisToArchive]
        );
        let fo4edit = fixture.run.fo4edit().unwrap().exe.clone();
        assert_eq!(
            process
                .calls()
                .iter()
                .map(|call| (call.exe.clone(), call.kind))
                .collect::<Vec<_>>(),
            vec![
                (fo4edit, ProcessCallKind::Spawn),
                (fixture.archive2.clone(), ProcessCallKind::RunCapturing),
                (fixture.archive2.clone(), ProcessCallKind::RunCapturing),
            ]
        );
        assert_eq!(execution.warnings, []);
    }

    /// `--resume-from 8` prepares and executes with the archive tool alone: the install has
    /// neither Creation Kit nor FO4Edit, and Step 8 rebuilds the archive with the new previs.
    #[test]
    fn a_resume_at_step_eight_prepares_and_executes_with_the_archive_tool_alone() {
        let fixture = prepare_no_creation_kit_run(WorkflowStep::AddPrevisToArchive, false);
        add_archive_and_previs_outputs(&fixture);
        let exit = ExitFlag::new();
        let process = completing_process(&fixture.run, &fixture.files, &exit);

        let execution = execute_over_recording_ports(&fixture.run, &fixture.files, &process, &exit);

        execution.result.unwrap();
        assert_eq!(
            fixture.run.runnable_steps(),
            &[WorkflowStep::AddPrevisToArchive]
        );
        assert!(fixture.run.fo4edit().is_none());
        let archive = fixture.run.archive().unwrap();
        assert_eq!(archive.tool, ArchiveTool::Archive2);
        assert_eq!(archive.exe, fixture.archive2);
        assert_eq!(archive.session_log, fixture.run.log_path());
        // The extract, then the repack.
        let calls = process.calls();
        assert_eq!(calls.len(), 2);
        for call in &calls {
            assert_eq!(call.exe, fixture.archive2);
            assert_eq!(call.kind, ProcessCallKind::RunCapturing);
        }
        assert_eq!(execution.delays, [MO2_DELAY_AFTER_ARCHIVE2_EXTRACT_SECS]);
        assert_eq!(execution.warnings, []);
        assert_eq!(
            fixture
                .files
                .read_lossy(&fixture.data.join(ARCHIVE_NAME))
                .unwrap(),
            PACKED_ARCHIVE
        );
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

        let stopped =
            execute_over_recording_ports(&run, &fixture.files, &process, &ExitFlag::new())
                .result
                .unwrap_err();

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

        let stopped =
            execute_over_recording_ports(&run, &fixture.files, &process, &ExitFlag::new())
                .result
                .unwrap_err();

        assert!(matches!(stopped.error(), Error::PluginAlreadyHasArchive));
        assert_eq!(process.calls(), Vec::<RecordedProcessCall>::new());
    }

    /// A completed run leaves its session log free of failure lines.
    #[test]
    fn a_completed_run_records_no_failure() {
        let fixture = ready_workflow_fixture();
        let run = prepared_run(&fixture);
        let exit = ExitFlag::new();
        let process = completing_process(&run, &fixture.files, &exit);

        execute_over_recording_ports(&run, &fixture.files, &process, &exit)
            .result
            .unwrap();

        let session = fixture.files.read_lossy(run.log_path()).unwrap();
        assert!(!session.contains("ERROR - "), "session log: {session}");
        assert!(!session.contains("failed."), "session log: {session}");
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

    /// Every Build Mode runs its whole plan in production, so none is prepared with the
    /// diagnostic saying it stops short.
    #[test]
    fn prepare_runs_the_whole_plan_in_every_build_mode() {
        for build_mode in [BuildMode::Clean, BuildMode::Filtered, BuildMode::Xbox] {
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

            assert_eq!(
                run.runnable_steps(),
                run.planned_steps(),
                "build mode: {build_mode:?}"
            );
            assert!(
                !stops_short(&run),
                "build mode: {build_mode:?}, diagnostics: {:?}",
                run.diagnostics()
            );
        }
    }

    /// A run that stops short of its plan says by how much, in each Build Mode. Production
    /// runs every planned step, so Step 8 is taken out of the registration to have a run that
    /// stops short.
    #[test]
    fn prepare_preserves_filtered_and_xbox_partial_diagnostic_counts() {
        // Filtered has no Steps 4 and 5; Xbox plans every step, as Clean does.
        let filtered_runnable = vec![
            WorkflowStep::GeneratePrecombines,
            WorkflowStep::MergePrecombineObjects,
            WorkflowStep::CreateBa2FromPrecombines,
            WorkflowStep::GeneratePrevis,
            WorkflowStep::MergePrevis,
        ];
        let cases = [
            (BuildMode::Filtered, 1, 6, filtered_runnable),
            (BuildMode::Xbox, 1, 8, FRESH_RUN_STEPS[..7].to_vec()),
        ];

        for (build_mode, skipped, planned, runnable) in cases {
            let fixture = ready_workflow_fixture();
            let mut request = fixture.request.clone();
            request.build_mode = build_mode;

            let run = prepare_without(
                &[WorkflowStep::AddPrevisToArchive],
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
                        runnable,
                    }),
                "build mode: {build_mode:?}, diagnostics: {:?}",
                run.diagnostics()
            );
        }
    }

    /// A Clean resume at 4 under a registration without Step 8 runs Steps 4 to 7, so the
    /// diagnostic must say that rather than claim Step 1 is what runs.
    #[test]
    fn a_resumed_partial_run_names_the_steps_it_will_execute() {
        let fixture = ready_workflow_fixture();
        let mut request = fixture.request.clone();
        request.resume_from = Some(WorkflowStep::CompressPsg);

        let run = prepare_without(
            &[WorkflowStep::AddPrevisToArchive],
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
            WorkflowStep::MergePrevis,
        ];
        assert_eq!(run.runnable_steps(), runnable.as_slice());
        assert!(
            run.diagnostics()
                .contains(&RunDiagnostic::LaterStepsNotImplemented {
                    skipped: 1,
                    planned: 5,
                    runnable,
                }),
            "diagnostics: {:?}",
            run.diagnostics()
        );
    }

    /// A resume at a step with no operation is refused before any tool is looked for. Every
    /// step is registered in production, so Step 8 is taken out to have one that is not.
    #[test]
    fn prepare_rejects_unavailable_explicit_resume_before_toolchain_readiness() {
        let fixture = ready_workflow_fixture();
        let mut request = fixture.request.clone();
        request.resume_from = Some(WorkflowStep::AddPrevisToArchive);
        let probe = WorkflowToolchainProbe::from_tool_paths(ToolPaths {
            fallout4_dir: Some(fixture.fallout4_directory.clone()),
            ..ToolPaths::default()
        })
        .unwrap();

        let error = prepare_without(
            &[WorkflowStep::AddPrevisToArchive],
            &request,
            fixture.directory.path(),
            &probe,
            &fixture.files,
        )
        .unwrap_err();

        assert!(matches!(error, Error::StepNotImplemented(8)));
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
            fo4edit: Some(write_fo4edit_install(&dir.path().join("FO4Edit"))),
            archive2: Some(archive2_exe(&fo4)),
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

    // --- the run-start restore of leftover archive work folders ------------------------------

    const ARCHIVE_NAME: &str = "MyMod - Main.ba2";

    /// A prepared fresh Clean run whose Fallout 4 directory a test seeds with leftover work
    /// folders before executing it.
    struct LeftoverRun {
        fixture: ReadyWorkflowFixture,
        run: WorkflowRun,
    }

    /// What starting a run over leftover work folders did.
    struct LeftoverExecution {
        result: std::result::Result<(), RunStopped>,
        warnings: Vec<BuildWarning>,
        /// The Fallout 4 directory's subfolders when Step 1 launched Creation Kit, or `None`
        /// when Step 1 stopped before launching it.
        folders_at_launch: Option<Vec<PathBuf>>,
    }

    impl LeftoverRun {
        /// A fresh Clean run, whose Step 1 stops before any archive step is reached.
        ///
        /// That the restore also runs when no archive tool was prepared at all is pinned by
        /// [`the_restore_runs_on_a_plan_with_no_archive_step`].
        fn new() -> Self {
            let fixture = ready_workflow_fixture();
            let run = prepared_run(&fixture);
            Self { fixture, run }
        }

        fn files(&self) -> &InMemoryFileSpace {
            &self.fixture.files
        }

        fn fo4(&self) -> PathBuf {
            self.fixture.fallout4_directory.clone()
        }

        fn data(&self) -> PathBuf {
            self.fo4().join("Data")
        }

        fn folder(&self, name: &str) -> PathBuf {
            self.fo4().join(name)
        }

        /// Write `lines` as the restore list of the work folder `name`.
        fn list(&self, name: &str, lines: &[String]) {
            self.files().add_file_with_contents(
                leftovers::restore_list(&self.folder(name)),
                lines.concat(),
            );
        }

        /// Execute over the fixture's space.
        fn execute(&self) -> LeftoverExecution {
            self.execute_through(self.files())
        }

        /// Execute with every port seeing the fixture's space through `ports_files`, and a
        /// Creation Kit that records the Fallout 4 directory's subfolders when it is launched.
        fn execute_through(&self, ports_files: &dyn FileSpace) -> LeftoverExecution {
            let launched = RefCell::new(None);
            let fo4 = self.fo4();
            let process = RecordingProcessRunner::new().with_effects(self.files(), |space| {
                let mut folders = space.child_dirs(&fo4);
                folders.sort();
                *launched.borrow_mut() = Some(folders);
            });

            let execution = execute_through(
                &self.run,
                self.files(),
                ports_files,
                &process,
                &ExitFlag::new(),
            );
            let folders_at_launch = launched.borrow().clone();
            LeftoverExecution {
                result: execution.result,
                warnings: execution.warnings,
                folders_at_launch,
            }
        }
    }

    /// Nothing to restore: a list-less `ArchiveWork` (a stale archive and an unlisted unpacked
    /// mesh) and an `ArchiveWork.3` with an empty list (a gap) are both gone before the first
    /// step dispatches, and nothing is warned about.
    #[test]
    fn leftovers_with_nothing_to_restore_are_removed_before_the_first_step() {
        let leftover = LeftoverRun::new();
        let files = leftover.files();
        files.add_file(leftover.folder("ArchiveWork").join(ARCHIVE_NAME));
        files.add_file(
            leftover
                .folder("ArchiveWork")
                .join("staging")
                .join("meshes")
                .join("precombined")
                .join("unpacked.nif"),
        );
        leftover.list("ArchiveWork.3", &[]);

        let execution = leftover.execute();

        // Creation Kit ran, so Step 1 was dispatched, and by then both were gone.
        assert!(matches!(
            execution.result.unwrap_err().error(),
            Error::MissingCombinedObjects
        ));
        let at_launch = execution.folders_at_launch.unwrap();
        assert!(!at_launch.contains(&leftover.folder("ArchiveWork")));
        assert!(!at_launch.contains(&leftover.folder("ArchiveWork.3")));
        assert_eq!(execution.warnings, []);
    }

    /// A crash while BSArch held the staged precombines: they go back into `Data` with their
    /// bytes, one warning names them, and the folder is removed — before Step 1, which then
    /// finds them.
    #[test]
    fn staged_precombines_from_a_crashed_step_three_are_moved_back() {
        let leftover = LeftoverRun::new();
        let staged = leftover
            .folder("ArchiveWork")
            .join("staging")
            .join("meshes")
            .join("precombined");
        let loose = leftover.data().join("meshes").join("precombined");
        leftover
            .files()
            .add_file_with_bytes(staged.join("cell").join("mesh.nif"), b"\x00mesh".to_vec());
        leftover.list(
            "ArchiveWork",
            &[leftovers::source_line(
                &PathBuf::from("staging").join("meshes").join("precombined"),
                &loose,
            )],
        );

        let execution = leftover.execute();

        assert_eq!(
            leftover
                .files()
                .contents(&loose.join("cell").join("mesh.nif")),
            Some(b"\x00mesh".to_vec())
        );
        assert_eq!(
            execution.warnings,
            [BuildWarning::ArchiveWorkRestored {
                item: staged,
                target: loose,
            }]
        );
        assert!(!leftover.files().is_dir(&leftover.folder("ArchiveWork")));
        // Step 1 ran after the restore, and found the restored meshes.
        assert!(matches!(
            execution.result.unwrap_err().error(),
            Error::PrecombinedMeshesExist
        ));
    }

    /// A crash while BSArch held `vis`: it goes back, while the unpacked precombines, unlisted
    /// copies, are discarded with the folder.
    #[test]
    fn staged_previs_from_a_crashed_step_eight_is_moved_back() {
        let leftover = LeftoverRun::new();
        let work = leftover.folder("ArchiveWork");
        let loose_vis = leftover.data().join("vis");
        leftover
            .files()
            .add_file_with_contents(work.join("staging").join("vis").join("c.uvd"), "previs");
        leftover.files().add_file(
            work.join("staging")
                .join("meshes")
                .join("precombined")
                .join("unpacked.nif"),
        );
        leftover.list(
            "ArchiveWork",
            &[leftovers::source_line(
                &PathBuf::from("staging").join("vis"),
                &loose_vis,
            )],
        );

        let execution = leftover.execute();

        assert_eq!(
            leftover
                .files()
                .read_lossy(&loose_vis.join("c.uvd"))
                .unwrap(),
            "previs"
        );
        assert!(
            !leftover
                .files()
                .is_dir(&leftover.data().join("meshes").join("precombined"))
        );
        assert!(!leftover.files().is_dir(&work));
        assert_eq!(execution.warnings.len(), 1);
    }

    /// A crash between the swap's delete and its rename: with no archive in `Data` the listed
    /// one is moved in; with one there, the entry is skipped silently and the old one kept.
    #[test]
    fn a_listed_archive_goes_back_only_when_data_has_none() {
        for archive_in_data in [false, true] {
            let leftover = LeftoverRun::new();
            let work = leftover.folder("ArchiveWork");
            let target = leftover.data().join(ARCHIVE_NAME);
            leftover
                .files()
                .add_file_with_contents(work.join(ARCHIVE_NAME), "new");
            if archive_in_data {
                leftover.files().add_file_with_contents(&target, "old");
            }
            leftover.list(
                "ArchiveWork",
                &[leftovers::archive_line(
                    &PathBuf::from(ARCHIVE_NAME),
                    &target,
                )],
            );

            let execution = leftover.execute();

            let expected = if archive_in_data { "old" } else { "new" };
            assert_eq!(leftover.files().read_lossy(&target).unwrap(), expected);
            let expected_warnings = if archive_in_data {
                vec![]
            } else {
                vec![BuildWarning::ArchiveWorkRestored {
                    item: work.join(ARCHIVE_NAME),
                    target: target.clone(),
                }]
            };
            assert_eq!(execution.warnings, expected_warnings);
            assert!(!leftover.files().is_dir(&work));
        }
    }

    /// A listed item that never reached the folder (the crash came before the move) is skipped
    /// silently, and the folder removed.
    #[test]
    fn an_entry_whose_item_never_moved_is_skipped() {
        let leftover = LeftoverRun::new();
        leftover.list(
            "ArchiveWork",
            &[leftovers::source_line(
                &PathBuf::from("staging").join("vis"),
                &leftover.data().join("vis"),
            )],
        );

        let execution = leftover.execute();

        assert_eq!(execution.warnings, []);
        assert!(!leftover.files().is_dir(&leftover.folder("ArchiveWork")));
        assert!(!leftover.files().is_dir(&leftover.data().join("vis")));
    }

    /// A `source` whose place in `Data` is occupied sets the folder aside, intact and with its
    /// list, under the first free `ArchiveWork.orphaned.<n>`; one already there is left alone.
    #[test]
    fn a_folder_whose_source_is_blocked_is_set_aside() {
        for orphan_present in [false, true] {
            let leftover = LeftoverRun::new();
            let staged_uvd = PathBuf::from("staging").join("vis").join("c.uvd");
            leftover
                .files()
                .add_file_with_contents(leftover.folder("ArchiveWork").join(&staged_uvd), "staged");
            leftover
                .files()
                .add_file_with_contents(leftover.data().join("vis").join("other.uvd"), "other");
            let lines = [leftovers::source_line(
                &PathBuf::from("staging").join("vis"),
                &leftover.data().join("vis"),
            )];
            leftover.list("ArchiveWork", &lines);
            let earlier_orphan = leftover.folder("ArchiveWork.orphaned.1").join("kept.txt");
            if orphan_present {
                leftover
                    .files()
                    .add_file_with_contents(&earlier_orphan, "kept");
            }

            let execution = leftover.execute();

            let set_aside = leftover.folder(if orphan_present {
                "ArchiveWork.orphaned.2"
            } else {
                "ArchiveWork.orphaned.1"
            });
            assert_eq!(
                execution.warnings,
                [BuildWarning::ArchiveWorkSetAside {
                    from: leftover.folder("ArchiveWork"),
                    to: set_aside.clone(),
                    items: LeftoverItems::Listed(vec![PathBuf::from("staging").join("vis")]),
                }]
            );
            assert_eq!(
                leftover
                    .files()
                    .read_lossy(&set_aside.join(&staged_uvd))
                    .unwrap(),
                "staged"
            );
            assert_eq!(
                leftover
                    .files()
                    .read_lossy(&leftovers::restore_list(&set_aside))
                    .unwrap(),
                lines.concat()
            );
            assert!(!leftover.files().is_dir(&leftover.folder("ArchiveWork")));
            if orphan_present {
                assert_eq!(
                    leftover.files().read_lossy(&earlier_orphan).unwrap(),
                    "kept"
                );
            }
        }
    }

    /// An invalid line sets the folder aside before anything is moved.
    #[test]
    fn an_invalid_restore_list_sets_the_folder_aside_untouched() {
        for invalid in [
            "source\tstaging\\vis\n",
            "copied\tstaging\\vis\tData\\vis\n",
            "source\t..\\vis\tData\\vis\n",
        ] {
            let leftover = LeftoverRun::new();
            let staged = leftover
                .folder("ArchiveWork")
                .join("staging")
                .join("meshes")
                .join("precombined")
                .join("mesh.nif");
            leftover.files().add_file(&staged);
            let loose = leftover.data().join("meshes").join("precombined");
            leftover.list(
                "ArchiveWork",
                &[
                    leftovers::source_line(
                        &PathBuf::from("staging").join("meshes").join("precombined"),
                        &loose,
                    ),
                    invalid.to_string(),
                ],
            );

            let execution = leftover.execute();

            assert_eq!(
                execution.warnings,
                [BuildWarning::ArchiveWorkSetAside {
                    from: leftover.folder("ArchiveWork"),
                    to: leftover.folder("ArchiveWork.orphaned.1"),
                    items: LeftoverItems::UnreadableList,
                }],
                "line: {invalid:?}"
            );
            assert!(!leftover.files().is_dir(&loose), "line: {invalid:?}");
        }
    }

    /// Two entries, the first restorable and the second blocked: the first goes back with its
    /// warning, then the folder is set aside naming only the second.
    #[test]
    fn a_partial_restore_sets_aside_only_what_is_left() {
        let leftover = LeftoverRun::new();
        let work = leftover.folder("ArchiveWork");
        let loose_precombined = leftover.data().join("meshes").join("precombined");
        let loose_vis = leftover.data().join("vis");
        leftover.files().add_file(
            work.join("staging")
                .join("meshes")
                .join("precombined")
                .join("mesh.nif"),
        );
        leftover
            .files()
            .add_file(work.join("staging").join("vis").join("c.uvd"));
        leftover.files().add_file(loose_vis.join("other.uvd"));
        leftover.list(
            "ArchiveWork",
            &[
                leftovers::source_line(
                    &PathBuf::from("staging").join("meshes").join("precombined"),
                    &loose_precombined,
                ),
                leftovers::source_line(&PathBuf::from("staging").join("vis"), &loose_vis),
            ],
        );

        let execution = leftover.execute();

        assert_eq!(
            execution.warnings,
            [
                BuildWarning::ArchiveWorkRestored {
                    item: work.join("staging").join("meshes").join("precombined"),
                    target: loose_precombined.clone(),
                },
                BuildWarning::ArchiveWorkSetAside {
                    from: work,
                    to: leftover.folder("ArchiveWork.orphaned.1"),
                    items: LeftoverItems::Listed(vec![PathBuf::from("staging").join("vis")]),
                },
            ]
        );
        assert!(
            leftover
                .files()
                .is_file(&loose_precombined.join("mesh.nif"))
        );
    }

    /// Folders that only look like work folders are never touched.
    #[test]
    fn folders_that_are_not_work_folders_are_left_alone() {
        let leftover = LeftoverRun::new();
        let names = [
            "ArchiveWorkspace",
            "ArchiveWork.bak",
            "ArchiveWork.1x",
            "ArchiveWork.orphaned.1",
        ];
        for name in names {
            leftover
                .files()
                .add_file(leftover.folder(name).join("keep.txt"));
        }

        let execution = leftover.execute();

        for name in names {
            assert!(
                leftover
                    .files()
                    .is_file(&leftover.folder(name).join("keep.txt")),
                "{name}"
            );
        }
        assert_eq!(execution.warnings, []);
    }

    /// A folder that cannot be removed is one warning, the next leftover is still handled, and
    /// the run goes on to dispatch its first step.
    #[test]
    fn a_stuck_leftover_is_a_warning_and_the_run_goes_on() {
        let leftover = LeftoverRun::new();
        leftover
            .files()
            .add_file(leftover.folder("ArchiveWork").join(ARCHIVE_NAME));
        leftover
            .files()
            .add_file(leftover.folder("ArchiveWork.1").join(ARCHIVE_NAME));
        let files = FaultyFileSpace::over(leftover.files())
            .refusing_dir_removal(leftover.folder("ArchiveWork"));

        let execution = leftover.execute_through(&files);

        assert_eq!(
            execution.warnings,
            [BuildWarning::ArchiveWorkFolderNotCleared {
                path: leftover.folder("ArchiveWork"),
            }]
        );
        let at_launch = execution.folders_at_launch.unwrap();
        assert!(at_launch.contains(&leftover.folder("ArchiveWork")));
        assert!(!at_launch.contains(&leftover.folder("ArchiveWork.1")));
    }

    /// A blocked folder that cannot be set aside either is one warning and stays where it is,
    /// never removed.
    #[test]
    fn a_blocked_folder_that_cannot_be_set_aside_is_left_in_place() {
        let leftover = LeftoverRun::new();
        let staged = leftover
            .folder("ArchiveWork")
            .join("staging")
            .join("vis")
            .join("c.uvd");
        leftover.files().add_file(&staged);
        leftover
            .files()
            .add_file(leftover.data().join("vis").join("other.uvd"));
        leftover.list(
            "ArchiveWork",
            &[leftovers::source_line(
                &PathBuf::from("staging").join("vis"),
                &leftover.data().join("vis"),
            )],
        );
        let files = FaultyFileSpace::over(leftover.files())
            .refusing_dir_move_to(leftover.folder("ArchiveWork.orphaned.1"));

        let execution = leftover.execute_through(&files);

        assert_eq!(
            execution.warnings,
            [BuildWarning::ArchiveWorkNotRestored {
                path: leftover.folder("ArchiveWork"),
                items: LeftoverItems::Listed(vec![PathBuf::from("staging").join("vis")]),
            }]
        );
        assert!(leftover.files().is_file(&staged));
    }

    /// The restore runs whatever the resume point, and even when no archive tool was prepared:
    /// a resume at Step 7 under a registration without Step 8, which plans no archive step and
    /// prepares FO4Edit alone, clears a leftover before its merge.
    #[test]
    fn the_restore_runs_on_a_plan_with_no_archive_step() {
        let fixture = prepare_fo4edit_only_run(WorkflowStep::MergePrevis);
        assert!(fixture.run.archive().is_none());
        let leftover = fixture
            .data
            .parent()
            .unwrap()
            .join("ArchiveWork")
            .join(ARCHIVE_NAME);
        fixture.files.add_file(&leftover);
        add_previs_outputs(&fixture);

        assert_executes_one_fo4edit_merge(&fixture, "Batch_FO4MergePrevisandCleanRefr.pas");

        assert!(!fixture.files.is_file(&leftover));
    }
}
