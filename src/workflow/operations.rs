//! Workflow Operations for the planned 8-step build.
//!
//! A Workflow Operation owns the domain flow for a step. External tools stay behind ports so
//! tests can exercise ordering and postconditions without launching CK.

use std::path::Path;

use crate::config::{BuildMode, WorkflowStep};
use crate::error::{Error, Result};
use crate::files::FileSpace;
use crate::interactive;
use crate::run::WorkflowRun;
use crate::toolchain::ToolchainRequirements;
use crate::tools::CreationKitOps;
use crate::workflow::WorkflowPlan;

mod generate_precombines;
mod precombine_workspace;

/// The shared recording adapters, crate-visible so the Workflow Run tests reach them too.
#[cfg(test)]
pub(crate) mod recording_adapters;

macro_rules! register_production_operations {
    ($($operation:expr),+ $(,)?) => {
        const PRODUCTION_OPERATION_SOURCE: ProductionOperationSource =
            ProductionOperationSource::new(&[$($operation),+]);
    };
}

register_production_operations!(generate_precombines::DEFINITION);

type OperationExecution = fn(&WorkflowRun, &OperationPorts<'_>) -> Result<()>;

#[derive(Debug, Clone, Copy)]
struct WorkflowOperationDefinition {
    step: WorkflowStep,
    toolchain_requirements: ToolchainRequirements,
    execute: OperationExecution,
}

impl WorkflowOperationDefinition {
    const fn new(
        step: WorkflowStep,
        toolchain_requirements: ToolchainRequirements,
        execute: OperationExecution,
    ) -> Self {
        Self {
            step,
            toolchain_requirements,
            execute,
        }
    }
}

/// Immutable view of the Workflow Operations implemented by the production build.
#[derive(Debug, Clone, Copy)]
struct ProductionOperationSource {
    operations: &'static [WorkflowOperationDefinition],
}

impl ProductionOperationSource {
    /// Build an immutable operation source with one definition per Workflow Step.
    ///
    /// Panics immediately when `operations` contains duplicate step definitions.
    const fn new(operations: &'static [WorkflowOperationDefinition]) -> Self {
        let mut operation_index = 0;
        while operation_index < operations.len() {
            let mut comparison_index = operation_index + 1;
            while comparison_index < operations.len() {
                assert!(
                    operations[operation_index].step.number()
                        != operations[comparison_index].step.number(),
                    "duplicate Workflow Operation registration"
                );
                comparison_index += 1;
            }
            operation_index += 1;
        }

        Self { operations }
    }

    /// Whether the production build has an implementation for a Workflow Step.
    #[must_use]
    fn contains(self, step: WorkflowStep) -> bool {
        self.operation_for_step(step).is_some()
    }

    /// Supply this source's operation availability to Workflow Plan resolution.
    ///
    /// Returns [`Error::StepNotImplemented`] when the requested resume step is not registered or
    /// when no planned Workflow Operation is available.
    fn workflow_plan(
        self,
        build_mode: BuildMode,
        resume_from: Option<WorkflowStep>,
    ) -> Result<WorkflowPlan> {
        WorkflowPlan::resolve(build_mode, resume_from, |step| self.contains(step))
    }

    /// Aggregate static readiness requirements for the runnable prefix of a Workflow Plan.
    ///
    /// Requirement collection stops at the first unregistered step, matching the same dependency
    /// boundary used to derive runnable steps.
    #[must_use]
    fn toolchain_requirements_for_steps(self, steps: &[WorkflowStep]) -> ToolchainRequirements {
        steps
            .iter()
            .map_while(|step| self.operation_for_step(*step))
            .fold(ToolchainRequirements::none(), |requirements, operation| {
                requirements.union(operation.toolchain_requirements)
            })
    }

    /// Dispatch one registered Workflow Operation through the supplied ports.
    ///
    /// Returns [`Error::StepNotImplemented`] when production has no operation for `step`, and
    /// otherwise propagates errors from the selected operation's domain flow or ports.
    fn dispatch(
        self,
        step: WorkflowStep,
        run: &WorkflowRun,
        ports: &OperationPorts<'_>,
    ) -> Result<()> {
        tracing::info!(step = step.number(), "{}", step.label());
        let operation = self
            .operation_for_step(step)
            .ok_or_else(|| Error::StepNotImplemented(step.number()))?;
        (operation.execute)(run, ports)
    }

    fn operation_for_step(
        self,
        step: WorkflowStep,
    ) -> Option<&'static WorkflowOperationDefinition> {
        self.operations
            .iter()
            .find(|operation| operation.step == step)
    }
}

/// Return the immutable source of Workflow Operations implemented by production.
#[must_use]
const fn production_operation_source() -> ProductionOperationSource {
    PRODUCTION_OPERATION_SOURCE
}

/// Build the Workflow Plan supplied by the immutable production operation registration.
///
/// Returns [`Error::StepNotImplemented`] when the selected resume point has no registered
/// Workflow Operation or when production cannot run the first planned step.
pub fn production_workflow_plan(
    build_mode: BuildMode,
    resume_from: Option<WorkflowStep>,
) -> Result<WorkflowPlan> {
    production_operation_source().workflow_plan(build_mode, resume_from)
}

/// A Workflow Plan paired with the requirements from the registration that resolved it.
pub(crate) struct ProductionWorkflowPreparation {
    plan: WorkflowPlan,
    requirements: ToolchainRequirements,
}

impl ProductionWorkflowPreparation {
    /// Consume the registration-resolved preparation inputs for Workflow Run validation.
    pub(crate) fn into_parts(self) -> (WorkflowPlan, ToolchainRequirements) {
        (self.plan, self.requirements)
    }
}

/// Prepare the production Workflow Plan and its complete runnable-operation requirement union.
///
/// Both values come from the immutable production registration so Workflow Run planning and
/// toolchain readiness cannot observe different operation availability.
pub(crate) fn prepare_production_workflow(
    build_mode: BuildMode,
    resume_from: Option<WorkflowStep>,
) -> Result<ProductionWorkflowPreparation> {
    let operations = production_operation_source();
    let plan = operations.workflow_plan(build_mode, resume_from)?;
    let requirements = operations.toolchain_requirements_for_steps(plan.runnable_steps());
    Ok(ProductionWorkflowPreparation { plan, requirements })
}

/// Execute exactly a prepared Workflow Run's runnable sequence through production registration.
///
/// Workflow Plan order remains authoritative; direct dispatch still returns
/// [`Error::StepNotImplemented`] defensively if a prepared step has no registered operation.
pub(crate) fn execute_registered_workflow(
    run: &WorkflowRun,
    ports: &OperationPorts<'_>,
) -> Result<()> {
    let operations = production_operation_source();
    for step in run.runnable_steps() {
        operations.dispatch(*step, run, ports)?;
    }
    Ok(())
}

/// The ports a Workflow Operation runs through, as data rather than as an interface.
///
/// Deliberately a struct: the bundle itself has exactly one shape, so a trait over it would be
/// a hypothetical seam. The *fields* are the real seams — `files` has two adapters, `prompts`
/// will, and `ck` sits above the `ProcessRunner` and `Wait` ports, which have two each.
///
/// `ck` is concrete rather than `&dyn CreationKit` for that same reason. Tests substitute
/// underneath it, at `ProcessRunner`/`Wait`, and run the real [`CreationKitOps`] — which is
/// what puts the DLL guard, the MO2 delay and the log lifecycle under a Step 1 test instead of
/// leaving the composition of them untested behind a one-implementation trait.
#[derive(Debug)]
pub(crate) struct OperationPorts<'a> {
    /// The Creation Kit episode: a domain verb per batch operation, its command grammar hidden.
    pub(crate) ck: &'a CreationKitOps<'a>,
    /// The operator questions a Workflow Operation is allowed to ask.
    pub(crate) prompts: &'a dyn Prompts,
    /// The space every Workflow Operation observes and cleans up external-tool outputs in.
    pub(crate) files: &'a dyn FileSpace,
}

/// The operator confirmations a Workflow Operation asks for.
///
/// Deliberately one method rather than a general `confirm(Question)`: there is one caller. The
/// full workflow has three confirmations — clear precombined here, clear vis at step 6, remove
/// working files at finish — and they are structurally identical, so generalising is mechanical
/// once step 6 gives the enum a second variant with a real caller.
///
/// Implementors must be `Debug` so [`OperationPorts`] can keep deriving it.
pub(crate) trait Prompts: std::fmt::Debug {
    /// Ask whether existing precombined meshes should be cleared before Step 1 resumes.
    fn confirm_clear_precombined(&self, precombined_dir: &Path) -> Result<bool>;
}

/// The production [`Prompts`], backed by the interactive console.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct InteractivePrompts;

impl Prompts for InteractivePrompts {
    fn confirm_clear_precombined(&self, precombined_dir: &Path) -> Result<bool> {
        interactive::confirm_clear_precombined(precombined_dir)
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::fs;
    use std::path::PathBuf;
    use tempfile::{TempDir, tempdir};

    use super::recording_adapters::{
        QUIET_CK_LOG, RecordingPrompts, record_successful_precombine_outputs,
    };
    use super::*;
    use crate::config::{ArchiveTool, BuildMode, PluginIdentity};
    use crate::files::InMemoryFileSpace;
    use crate::run::WorkflowRequest;
    use crate::tools::process::{ProcessRunner, RecordingProcessRunner};
    use crate::tools::wait::{MO2_DELAY_AFTER_CK_SECS, RecordingWait};
    use crate::{discovery::ToolPaths, toolchain::WorkflowToolchainProbe};

    /// Provide an execution entry for registration-only tests that must never dispatch.
    fn unused_execution(_run: &WorkflowRun, _ports: &OperationPorts<'_>) -> Result<()> {
        unreachable!("filtering registered steps must not execute operations")
    }

    #[test]
    fn production_workflow_plan_filters_registered_operations() {
        let plan = production_workflow_plan(BuildMode::Clean, None).unwrap();

        assert_eq!(plan.planned_steps().len(), 8);
        assert_eq!(plan.runnable_steps(), &[WorkflowStep::GeneratePrecombines]);
        assert_eq!(plan.skipped_unrunnable_count(), 7);
    }

    #[test]
    fn production_workflow_plan_rejects_unregistered_resume_step() {
        let cases = [
            (BuildMode::Clean, WorkflowStep::GeneratePrevis, 6),
            (BuildMode::Filtered, WorkflowStep::CompressPsg, 4),
        ];

        for (mode, resume, expected_step) in cases {
            let error = production_workflow_plan(mode, Some(resume)).unwrap_err();

            assert!(
                matches!(error, Error::StepNotImplemented(step) if step == expected_step),
                "mode: {mode:?}, resume: {resume:?}, error: {error:?}"
            );
        }
    }

    #[test]
    fn production_preparation_unions_requirements_for_the_runnable_plan() {
        let (plan, requirements) = prepare_production_workflow(BuildMode::Clean, None)
            .unwrap()
            .into_parts();

        assert_eq!(plan.runnable_steps(), &[WorkflowStep::GeneratePrecombines]);
        assert!(requirements.needs_creation_kit());
        assert!(!requirements.needs_fo4edit());
        assert!(!requirements.needs_archive());
    }

    #[test]
    fn production_source_filters_registered_steps_in_plan_order() {
        let source = production_operation_source();
        let plan = source.workflow_plan(BuildMode::Clean, None).unwrap();

        assert!(source.contains(WorkflowStep::GeneratePrecombines));
        assert!(!source.contains(WorkflowStep::MergePrecombineObjects));
        assert_eq!(plan.runnable_steps(), &[WorkflowStep::GeneratePrecombines]);
    }

    #[test]
    fn source_registration_order_does_not_change_plan_order() {
        const MERGE_PRECOMBINE_OBJECTS: WorkflowOperationDefinition =
            WorkflowOperationDefinition::new(
                WorkflowStep::MergePrecombineObjects,
                ToolchainRequirements::none(),
                unused_execution,
            );
        const REVERSED_OPERATIONS: &[WorkflowOperationDefinition] =
            &[MERGE_PRECOMBINE_OBJECTS, generate_precombines::DEFINITION];
        let source = ProductionOperationSource::new(REVERSED_OPERATIONS);
        let plan = source.workflow_plan(BuildMode::Clean, None).unwrap();

        assert_eq!(
            plan.runnable_steps(),
            &[
                WorkflowStep::GeneratePrecombines,
                WorkflowStep::MergePrecombineObjects,
            ]
        );
    }

    #[test]
    #[should_panic(expected = "duplicate Workflow Operation registration")]
    fn source_rejects_duplicate_step_registration() {
        const DUPLICATE_OPERATIONS: &[WorkflowOperationDefinition] = &[
            generate_precombines::DEFINITION,
            generate_precombines::DEFINITION,
        ];

        let _source = ProductionOperationSource::new(DUPLICATE_OPERATIONS);
    }

    #[test]
    fn source_stops_before_first_unregistered_planned_step() {
        const CREATE_BA2_FROM_PRECOMBINES: WorkflowOperationDefinition =
            WorkflowOperationDefinition::new(
                WorkflowStep::CreateBa2FromPrecombines,
                ToolchainRequirements::none(),
                unused_execution,
            );
        const GAPPED_OPERATIONS: &[WorkflowOperationDefinition] = &[
            CREATE_BA2_FROM_PRECOMBINES,
            generate_precombines::DEFINITION,
        ];
        let source = ProductionOperationSource::new(GAPPED_OPERATIONS);
        let plan = source.workflow_plan(BuildMode::Clean, None).unwrap();

        assert_eq!(plan.runnable_steps(), &[WorkflowStep::GeneratePrecombines]);
    }

    #[test]
    fn source_allows_explicit_resume_at_registered_later_step() {
        const GENERATE_PREVIS: WorkflowOperationDefinition = WorkflowOperationDefinition::new(
            WorkflowStep::GeneratePrevis,
            ToolchainRequirements::none(),
            unused_execution,
        );
        const MERGE_PREVIS: WorkflowOperationDefinition = WorkflowOperationDefinition::new(
            WorkflowStep::MergePrevis,
            ToolchainRequirements::none(),
            unused_execution,
        );
        const LATER_OPERATIONS: &[WorkflowOperationDefinition] = &[MERGE_PREVIS, GENERATE_PREVIS];
        let source = ProductionOperationSource::new(LATER_OPERATIONS);
        let plan = source
            .workflow_plan(BuildMode::Clean, Some(WorkflowStep::GeneratePrevis))
            .unwrap();

        assert_eq!(
            plan.runnable_steps(),
            &[WorkflowStep::GeneratePrevis, WorkflowStep::MergePrevis]
        );
    }

    #[test]
    fn production_source_aggregates_requirements_only_for_registered_steps() {
        let source = production_operation_source();
        let requirements =
            source.toolchain_requirements_for_steps(WorkflowStep::steps_for_mode(BuildMode::Clean));

        assert!(requirements.needs_creation_kit());
        assert!(!requirements.needs_fo4edit());
        assert!(!requirements.needs_archive());

        let unregistered_requirements = source.toolchain_requirements_for_steps(&[
            WorkflowStep::GeneratePrevis,
            WorkflowStep::MergePrevis,
            WorkflowStep::AddPrevisToArchive,
        ]);
        assert!(!unregistered_requirements.needs_creation_kit());
        assert!(!unregistered_requirements.needs_fo4edit());
        assert!(!unregistered_requirements.needs_archive());
    }

    #[test]
    fn source_aggregates_requirements_only_for_contiguous_prefix() {
        const FO4EDIT_REQUIREMENTS: ToolchainRequirements = {
            let mut requirements = ToolchainRequirements::none();
            requirements.require_fo4edit();
            requirements
        };
        const ARCHIVE_REQUIREMENTS: ToolchainRequirements = {
            let mut requirements = ToolchainRequirements::none();
            requirements.require_archive();
            requirements
        };
        const MERGE_PRECOMBINE_OBJECTS: WorkflowOperationDefinition =
            WorkflowOperationDefinition::new(
                WorkflowStep::MergePrecombineObjects,
                FO4EDIT_REQUIREMENTS,
                unused_execution,
            );
        const COMPRESS_PSG: WorkflowOperationDefinition = WorkflowOperationDefinition::new(
            WorkflowStep::CompressPsg,
            ARCHIVE_REQUIREMENTS,
            unused_execution,
        );
        const GAPPED_OPERATIONS: &[WorkflowOperationDefinition] = &[
            COMPRESS_PSG,
            MERGE_PRECOMBINE_OBJECTS,
            generate_precombines::DEFINITION,
        ];
        let source = ProductionOperationSource::new(GAPPED_OPERATIONS);

        let requirements =
            source.toolchain_requirements_for_steps(WorkflowStep::steps_for_mode(BuildMode::Clean));

        assert!(requirements.needs_creation_kit());
        assert!(requirements.needs_fo4edit());
        assert!(!requirements.needs_archive());
    }

    /// A prepared Step 1 run and the ports it will be driven through, minus the process runner.
    ///
    /// The process runner is left to the caller because it borrows [`Self::files`] when it
    /// simulates Creation Kit's outputs, and a struct cannot hold both ends of that borrow.
    struct Step1Fixture {
        /// The real directory the toolchain probe validated; kept alive for the run's lifetime.
        _dir: TempDir,
        run: WorkflowRun,
        /// The one space the session log, the artifacts and the Creation Kit log all live in.
        ///
        /// Per fixture, so this module's `MyMod` runs never share a session log with the
        /// identically-named fixtures in `run` and `precombine_workspace`.
        files: InMemoryFileSpace,
        wait: RecordingWait,
        prompts: RecordingPrompts,
    }

    impl Step1Fixture {
        /// The log CKPE configured for this run, which the Creation Kit episode owns.
        fn ck_log_path(&self) -> PathBuf {
            self.run.creation_kit().unwrap().ck_log_path.clone()
        }

        /// Assemble the ports for this fixture and hand them to `use_ports`.
        ///
        /// A closure rather than a returned bundle: `CreationKitOps` borrows the three ports it
        /// was bound to, and `OperationPorts` borrows that in turn, so the whole chain has to
        /// live inside one stack frame. Every test that needs ports goes through here, which is
        /// what keeps the assembly spelled out once.
        fn with_ports<T>(
            &self,
            process: &dyn ProcessRunner,
            use_ports: impl FnOnce(&OperationPorts<'_>) -> T,
        ) -> T {
            let ck = self
                .run
                .creation_kit()
                .unwrap()
                .bind(process, &self.wait, &self.files);

            use_ports(&OperationPorts {
                ck: &ck,
                prompts: &self.prompts,
                files: &self.files,
            })
        }

        /// Drive Step 1 over the real Creation Kit episode, with `process` standing in for the
        /// spawn and the fixture's recording `Wait` standing in for the mandated delay.
        fn run_step_one(&self, process: &dyn ProcessRunner) -> Result<()> {
            self.with_ports(process, |ports| generate_precombines::run(&self.run, ports))
        }
    }

    /// Prepare a real Step 1 Workflow Run over a temporary Fallout 4 directory.
    ///
    /// The directory is the only thing that reaches the disk: it holds the `CreationKit.exe` and
    /// the CKPE ini the toolchain probe insists on seeing, because probing is not behind a seam.
    /// Everything the run then does — its session log, the artifacts, the Creation Kit log —
    /// lands in the fixture's own [`InMemoryFileSpace`].
    fn step_one_fixture(mode: BuildMode, non_interactive: bool) -> Step1Fixture {
        let dir = tempdir().unwrap();
        let fallout4_dir = dir.path().join("Fallout4");
        fs::create_dir_all(&fallout4_dir).unwrap();
        fs::write(fallout4_dir.join("CreationKit.exe"), b"").unwrap();
        fs::write(
            fallout4_dir.join("fallout4_test.ini"),
            "[CreationKit]\nBSHandleRefObjectPatch=true\n[CreationKit_Log]\nOutputFile=CK.log\n",
        )
        .unwrap();

        let probe = WorkflowToolchainProbe::from_tool_paths(ToolPaths {
            fallout4_dir: Some(fallout4_dir.clone()),
            creation_kit: Some(fallout4_dir.join("CreationKit.exe")),
            ..ToolPaths::default()
        })
        .unwrap();

        let request = WorkflowRequest::new(
            mode,
            ArchiveTool::Archive2,
            PluginIdentity::parse("MyMod"),
            non_interactive,
            None,
            None,
        );

        let files = InMemoryFileSpace::new();
        let run = WorkflowRun::prepare(&request, dir.path(), &probe, &files).unwrap();

        Step1Fixture {
            _dir: dir,
            run,
            files,
            wait: RecordingWait::new(),
            prompts: RecordingPrompts::new(),
        }
    }

    /// A Creation Kit spawn that leaves a successful precombine run's outputs behind.
    fn successful_spawn(fixture: &Step1Fixture) -> RecordingProcessRunner<'_> {
        let config = fixture.run.config().clone();
        let ck_log = fixture.ck_log_path();

        RecordingProcessRunner::new().with_effects(&fixture.files, move |space| {
            record_successful_precombine_outputs(space, &config, &ck_log);
        })
    }

    /// Drive one Step 1 run to completion and return the single spawn it produced.
    ///
    /// Asserts the invariants every mode shares — one spawn, of the run's own Creation Kit, in
    /// the run's Fallout 4 directory, naming the run's own plugin — and hands the argv back so
    /// the caller can compare modes against each other.
    fn only_spawn_of_step_one(mode: BuildMode) -> Vec<OsString> {
        let fixture = step_one_fixture(mode, true);
        let process = successful_spawn(&fixture);

        fixture.run_step_one(&process).unwrap();

        let mut calls = process.calls();
        assert_eq!(calls.len(), 1, "mode: {mode:?}");
        let call = calls.remove(0);
        assert_eq!(call.exe, fixture.run.creation_kit().unwrap().exe);
        assert_eq!(call.cwd, fixture.run.config().fallout4_dir);
        assert!(
            call.args
                .iter()
                .any(|arg| arg.to_string_lossy().contains("MyMod.esp")),
            "mode: {mode:?}, args: {:?}",
            call.args
        );

        call.args
    }

    /// Step 1 asks for precombines once, for the run's own plugin and the run's own build mode.
    ///
    /// The build mode is pinned by *difference* rather than by spelling out what it becomes on
    /// the command line: a Step 1 that ignored `config.build_mode` would send Clean and
    /// Filtered identical argv, and one that invented its own mapping would split Filtered from
    /// Xbox. Which qualifiers each mode actually produces is asserted in `tools::creation_kit`,
    /// against the same recorded argv — and leaving them there is what keeps Creation Kit's
    /// command grammar out of `src/workflow/` entirely.
    #[test]
    fn step_one_asks_creation_kit_to_generate_precombines_for_the_build_mode() {
        let clean = only_spawn_of_step_one(BuildMode::Clean);
        let filtered = only_spawn_of_step_one(BuildMode::Filtered);
        let xbox = only_spawn_of_step_one(BuildMode::Xbox);

        assert_ne!(clean, filtered);
        // Xbox differs from Filtered only in how the archive is compressed, which is steps 5
        // and 8; the precombine request is the same one.
        assert_eq!(filtered, xbox);
    }

    /// A Step 1 run keeps the whole Creation Kit episode, not just the spawn.
    ///
    /// None of this was observable from a Step 1 test while a fake stood in for the episode:
    /// the mandated MO2 delay, the log lifecycle, and the DLL guard were asserted only against
    /// `CreationKitOps` in isolation, or not at all. Here the real episode runs.
    #[test]
    fn step_one_runs_the_whole_creation_kit_episode() {
        let fixture = step_one_fixture(BuildMode::Clean, true);
        let enb_dll = fixture.run.config().fallout4_dir.join("d3d11.dll");
        fixture.files.add_file_with_contents(&enb_dll, "enb");
        let process = successful_spawn(&fixture);

        fixture.run_step_one(&process).unwrap();

        // The delay workarounds.md §2 mandates, once, at its batch duration.
        assert_eq!(fixture.wait.delays(), vec![MO2_DELAY_AFTER_CK_SECS]);
        // The Creation Kit log reached the session log rather than being read and dropped.
        assert!(
            fixture
                .files
                .read_lossy(fixture.run.log_path())
                .unwrap()
                .contains(QUIET_CK_LOG)
        );
        // The DLL guard ran and put the ENB DLL back, contents intact.
        assert_eq!(fixture.files.read_lossy(&enb_dll).unwrap(), "enb");
        assert!(
            !fixture.files.is_file(
                &fixture
                    .run
                    .config()
                    .fallout4_dir
                    .join("d3d11.dll-PJMdisabled")
            )
        );
    }

    #[test]
    fn step_one_rejects_existing_plugin_archive_before_ck() {
        let fixture = step_one_fixture(BuildMode::Filtered, true);
        fixture
            .files
            .add_file(fixture.run.config().plugin_archive_path());
        let process = successful_spawn(&fixture);

        let err = fixture.run_step_one(&process).unwrap_err();

        assert!(matches!(err, Error::PluginAlreadyHasArchive));
        assert!(process.calls().is_empty());
        // Nothing ran, so nothing waited either — the whole episode is skipped, not just spawn.
        assert!(fixture.wait.delays().is_empty());
    }

    #[test]
    fn step_one_rejects_vis_uvd_files_before_ck() {
        let fixture = step_one_fixture(BuildMode::Filtered, true);
        fixture
            .files
            .add_file(fixture.run.config().vis_dir().join("cell.uvd"));
        let process = successful_spawn(&fixture);

        let err = fixture.run_step_one(&process).unwrap_err();

        assert!(matches!(err, Error::VisUvdFilesExist));
        assert!(process.calls().is_empty());
    }

    #[test]
    fn step_one_prompt_clears_precombined_meshes_when_interactive() {
        let fixture = step_one_fixture(BuildMode::Filtered, false);
        let existing_mesh = fixture
            .run
            .config()
            .precombined_dir()
            .join("old")
            .join("mesh.nif");
        fixture.files.add_file(&existing_mesh);
        let process = successful_spawn(&fixture);

        fixture.run_step_one(&process).unwrap();

        assert_eq!(fixture.prompts.clear_prompt_count(), 1);
        assert!(!fixture.files.is_file(&existing_mesh));
        assert_eq!(process.calls().len(), 1);
    }

    #[test]
    fn step_one_stops_when_the_clear_precombined_prompt_is_refused() {
        let fixture = Step1Fixture {
            prompts: RecordingPrompts::new().refusing_clear_precombined(),
            ..step_one_fixture(BuildMode::Filtered, false)
        };
        let existing_mesh = fixture
            .run
            .config()
            .precombined_dir()
            .join("old")
            .join("mesh.nif");
        fixture.files.add_file(&existing_mesh);
        let process = successful_spawn(&fixture);

        let err = fixture.run_step_one(&process).unwrap_err();

        assert!(matches!(err, Error::Other(message) if message
                == "precombined meshes not cleared - choose another resume step"));
        assert_eq!(fixture.prompts.clear_prompt_count(), 1);
        assert!(process.calls().is_empty());
        // A refusal leaves the meshes alone; the run stops rather than clearing anyway.
        assert!(fixture.files.is_file(&existing_mesh));
    }

    #[test]
    fn step_one_rejects_existing_precombined_meshes_without_prompting_when_non_interactive() {
        let fixture = step_one_fixture(BuildMode::Filtered, true);
        fixture.files.add_file(
            fixture
                .run
                .config()
                .precombined_dir()
                .join("old")
                .join("mesh.nif"),
        );
        let process = successful_spawn(&fixture);

        let err = fixture.run_step_one(&process).unwrap_err();

        assert!(matches!(err, Error::PrecombinedMeshesExist));
        assert_eq!(fixture.prompts.clear_prompt_count(), 0);
        assert!(process.calls().is_empty());
    }

    #[test]
    fn unregistered_source_dispatch_fails_before_the_ports() {
        let fixture = step_one_fixture(BuildMode::Filtered, true);
        let process = successful_spawn(&fixture);

        let err = fixture
            .with_ports(&process, |ports| {
                production_operation_source().dispatch(
                    WorkflowStep::MergePrecombineObjects,
                    &fixture.run,
                    ports,
                )
            })
            .unwrap_err();

        assert!(matches!(err, Error::StepNotImplemented(2)));
        assert!(process.calls().is_empty());
        assert_eq!(fixture.prompts.clear_prompt_count(), 0);
    }
}
