//! Workflow Operations for the planned 8-step build.
//!
//! A Workflow Operation owns the domain flow for a step. External tools stay behind ports so
//! tests can exercise ordering and postconditions without launching CK.

use std::path::PathBuf;

use crate::config::{BuildMode, WorkflowStep};
use crate::error::{Error, Result};
use crate::files::FileSpace;
use crate::interactive;
use crate::run::WorkflowRun;
use crate::toolchain::ToolchainRequirements;
use crate::tools::CreationKitOps;
use crate::warning::BuildWarnings;
use crate::workflow::WorkflowPlan;

mod build_cdx;
mod compress_psg;
mod generate_precombines;
mod generate_previs;
mod precombine_workspace;
mod previs_workspace;

/// The shared recording adapters, crate-visible so the Workflow Run tests reach them too.
#[cfg(test)]
pub(crate) mod recording_adapters;

macro_rules! register_production_operations {
    ($($operation:expr),+ $(,)?) => {
        const PRODUCTION_OPERATION_SOURCE: ProductionOperationSource =
            ProductionOperationSource::new(&[$($operation),+]);
    };
}

register_production_operations!(
    generate_precombines::DEFINITION,
    compress_psg::DEFINITION,
    build_cdx::DEFINITION,
    generate_previs::DEFINITION,
);

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
    /// Where a Workflow Operation raises the Build Warnings it completes with.
    ///
    /// Concrete, not a trait, for the reason the bundle itself is a struct: there is one way
    /// to raise a warning. It is shared across every operation in the run, so a warning raised
    /// by an earlier step is still there when a later one stops.
    pub(crate) warnings: &'a BuildWarnings<'a>,
}

/// A yes/no question a Workflow Operation puts to the operator.
///
/// Each variant names *what* is being confirmed and carries the artifact it is about; the
/// console wording and default for each live in [`crate::interactive`], so an operation states
/// the decision it needs and never phrases the question. Finish adds `RemoveWorkingFiles`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Confirmation {
    /// Clear existing precombined meshes before Step 1 regenerates them (`:RePrecomb`).
    ClearPrecombined(PathBuf),
    /// Clear a non-empty `Data\vis` before Step 6 regenerates previs (`:RePreVis`, batch 246).
    ClearVis(PathBuf),
}

/// The operator confirmations a Workflow Operation asks for.
///
/// One method over a [`Confirmation`] rather than one method per question: the workflow's
/// confirmations — clear precombined at Step 1, clear vis at Step 6, remove working files at
/// Finish — are structurally identical, so a new one is a new variant rather than a new
/// method every implementor has to grow.
///
/// Implementors must be `Debug` so [`OperationPorts`] can keep deriving it.
pub(crate) trait Prompts: std::fmt::Debug {
    /// Ask the operator `confirmation`, returning `true` when they consent.
    ///
    /// Returns [`Error::Prompt`] when the console cannot be read.
    fn confirm(&self, confirmation: &Confirmation) -> Result<bool>;
}

/// The production [`Prompts`], backed by the interactive console.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct InteractivePrompts;

impl Prompts for InteractivePrompts {
    fn confirm(&self, confirmation: &Confirmation) -> Result<bool> {
        match confirmation {
            Confirmation::ClearPrecombined(precombined_dir) => {
                interactive::confirm_clear_precombined(precombined_dir)
            }
            // The batch's question names `Data\vis` itself (246), so the path is not shown.
            Confirmation::ClearVis(_) => interactive::confirm_clear_vis(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::fs;
    use std::path::PathBuf;
    use tempfile::{TempDir, tempdir};

    use super::recording_adapters::{
        QUIET_CK_LOG, RecordingPrompts, record_successful_cdx_outputs,
        record_successful_compress_outputs, record_successful_precombine_outputs,
        record_successful_previs_outputs,
    };
    use super::*;
    use crate::config::{ArchiveTool, BuildMode, PluginIdentity};
    use crate::files::InMemoryFileSpace;
    use crate::run::WorkflowRequest;
    use crate::tools::CkPorts;
    use crate::tools::clock::ScriptedClock;
    use crate::tools::process::{ProcessRunner, RecordingProcessRunner};
    use crate::tools::wait::{MO2_DELAY_AFTER_CK_SECS, RecordingWait};
    use crate::warning::BuildWarning;
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
            (BuildMode::Clean, WorkflowStep::AddPrevisToArchive, 8),
            (BuildMode::Filtered, WorkflowStep::MergePrevis, 7),
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

    /// A prepared Workflow Run and the ports one of its operations will be driven through, minus
    /// the process runner.
    ///
    /// The process runner is left to the caller because it borrows [`Self::files`] when it
    /// simulates Creation Kit's outputs, and a struct cannot hold both ends of that borrow.
    struct OperationFixture {
        /// The real directory the toolchain probe validated; kept alive for the run's lifetime.
        _dir: TempDir,
        run: WorkflowRun,
        /// The one space the session log, the artifacts and the Creation Kit log all live in.
        ///
        /// Per fixture, so this module's `MyMod` runs never share a session log with the
        /// identically-named fixtures in `run` and `precombine_workspace`.
        files: InMemoryFileSpace,
        wait: RecordingWait,
        /// The `Start`/`Ended` readings the episode writes into the session log.
        ///
        /// Scripted so the session-log assertions below are not at the mercy of wall time; the
        /// two readings are distinct because the block records both.
        clock: ScriptedClock,
        prompts: RecordingPrompts,
    }

    impl OperationFixture {
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
            let ck = self.run.creation_kit().unwrap().bind(CkPorts {
                process,
                wait: &self.wait,
                clock: &self.clock,
                files: &self.files,
            });
            let warnings = BuildWarnings::new(self.run.log_path().to_path_buf(), &self.files);

            use_ports(&OperationPorts {
                ck: &ck,
                prompts: &self.prompts,
                files: &self.files,
                warnings: &warnings,
            })
        }

        /// Drive Step 1 over the real Creation Kit episode, with `process` standing in for the
        /// spawn and the fixture's recording `Wait` standing in for the mandated delay.
        fn run_step_one(&self, process: &dyn ProcessRunner) -> Result<()> {
            self.run_step_one_collecting_warnings(process).0
        }

        /// [`Self::run_step_one`], also returning every Build Warning the run raised.
        ///
        /// Read back after the operation returns, whatever it returned, because a warning
        /// raised before a stop must still be in the collector.
        fn run_step_one_collecting_warnings(
            &self,
            process: &dyn ProcessRunner,
        ) -> (Result<()>, Vec<BuildWarning>) {
            self.run_collecting_warnings(generate_precombines::run, process)
        }

        /// Drive Step 4 over the real Creation Kit episode; see [`Self::run_step_one`].
        fn run_step_four(&self, process: &dyn ProcessRunner) -> Result<()> {
            self.run_step_four_collecting_warnings(process).0
        }

        /// [`Self::run_step_four`], also returning every Build Warning the run raised.
        fn run_step_four_collecting_warnings(
            &self,
            process: &dyn ProcessRunner,
        ) -> (Result<()>, Vec<BuildWarning>) {
            self.run_collecting_warnings(compress_psg::run, process)
        }

        /// Drive Step 5 over the real Creation Kit episode; see [`Self::run_step_one`].
        fn run_step_five(&self, process: &dyn ProcessRunner) -> Result<()> {
            self.run_step_five_collecting_warnings(process).0
        }

        /// [`Self::run_step_five`], also returning every Build Warning the run raised.
        fn run_step_five_collecting_warnings(
            &self,
            process: &dyn ProcessRunner,
        ) -> (Result<()>, Vec<BuildWarning>) {
            self.run_collecting_warnings(build_cdx::run, process)
        }

        /// Drive Step 6 over the real Creation Kit episode; see [`Self::run_step_one`].
        fn run_step_six(&self, process: &dyn ProcessRunner) -> Result<()> {
            self.run_step_six_collecting_warnings(process).0
        }

        /// [`Self::run_step_six`], also returning every Build Warning the run raised.
        fn run_step_six_collecting_warnings(
            &self,
            process: &dyn ProcessRunner,
        ) -> (Result<()>, Vec<BuildWarning>) {
            self.run_collecting_warnings(generate_previs::run, process)
        }

        /// Drive `operation` over this fixture's ports and return how it ended, with every
        /// Build Warning it raised.
        ///
        /// The warnings are read back after the operation returns, whatever it returned,
        /// because a warning raised before a stop must still be in the collector.
        fn run_collecting_warnings(
            &self,
            operation: OperationExecution,
            process: &dyn ProcessRunner,
        ) -> (Result<()>, Vec<BuildWarning>) {
            self.with_ports(process, |ports| {
                let result = operation(&self.run, ports);
                (result, ports.warnings.raised())
            })
        }
    }

    /// Prepare a real Step 1 Workflow Run over a temporary Fallout 4 directory.
    fn step_one_fixture(mode: BuildMode, non_interactive: bool) -> OperationFixture {
        operation_fixture(mode, non_interactive, None)
    }

    /// Prepare a real non-interactive Workflow Run resumed at Step 4, the way an operator
    /// re-running Compress PSG after a failure enters it.
    fn step_four_fixture(mode: BuildMode) -> OperationFixture {
        operation_fixture(mode, true, Some(WorkflowStep::CompressPsg))
    }

    /// Prepare a real non-interactive Workflow Run resumed at Step 5, which the batch enters at
    /// `:BldCDX` (231), past Step 4's `.psg` check.
    fn step_five_fixture(mode: BuildMode) -> OperationFixture {
        operation_fixture(mode, true, Some(WorkflowStep::BuildCdx))
    }

    /// Prepare a real Workflow Run resumed at Step 6, the one entry (`:RePreVis`, 244–249) on
    /// which Step 6 may offer to clear a non-empty `Datais`.
    fn step_six_fixture(mode: BuildMode, non_interactive: bool) -> OperationFixture {
        operation_fixture(mode, non_interactive, Some(WorkflowStep::GeneratePrevis))
    }

    /// Prepare a real Workflow Run over a temporary Fallout 4 directory.
    ///
    /// The directory is the only thing that reaches the disk: it holds the `CreationKit.exe` and
    /// the CKPE ini the toolchain probe insists on seeing, because probing is not behind a seam.
    /// Everything the run then does — its session log, the artifacts, the Creation Kit log —
    /// lands in the fixture's own [`InMemoryFileSpace`].
    ///
    /// `resume_from` must name a registered step, because preparation resolves the plan.
    fn operation_fixture(
        mode: BuildMode,
        non_interactive: bool,
        resume_from: Option<WorkflowStep>,
    ) -> OperationFixture {
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
            resume_from,
        );

        let files = InMemoryFileSpace::new();
        let run = WorkflowRun::prepare(&request, dir.path(), &probe, &files).unwrap();

        OperationFixture {
            _dir: dir,
            run,
            files,
            wait: RecordingWait::new(),
            clock: ScriptedClock::new([CK_RUN_STARTED_AT, CK_RUN_ENDED_AT]),
            prompts: RecordingPrompts::new(),
        }
    }

    /// The scripted `Start`/`Ended` readings a fixture's Creation Kit run reports.
    const CK_RUN_STARTED_AT: &str = "09:00:00.00";
    const CK_RUN_ENDED_AT: &str = "09:04:12.34";

    /// A Creation Kit spawn that leaves a successful precombine run's outputs behind.
    fn successful_spawn(fixture: &OperationFixture) -> RecordingProcessRunner<'_> {
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
    /// Filtered identical argv, and one that invented its own mapping would split Clean from
    /// Xbox. Which qualifiers each mode actually produces is asserted in `tools::creation_kit`,
    /// against the same recorded argv — and leaving them there is what keeps Creation Kit's
    /// command grammar out of `src/workflow/` entirely.
    #[test]
    fn step_one_asks_creation_kit_to_generate_precombines_for_the_build_mode() {
        let clean = only_spawn_of_step_one(BuildMode::Clean);
        let filtered = only_spawn_of_step_one(BuildMode::Filtered);
        let xbox = only_spawn_of_step_one(BuildMode::Xbox);

        assert_ne!(clean, filtered);
        // V2.99 Xbox is a clean build (batch 268–272 test `NEQ "filtered"`): it differs from
        // Clean only in skipping `CompressPSG` (no archive gets Xbox compression since 391 was
        // REM'd), which is not Step 1, so the precombine request is the same one.
        assert_eq!(clean, xbox);
    }

    /// V2.99 Xbox fails Step 1 on a missing `<plugin> - Geometry.psg`, exactly as Clean does
    /// (batch line 270).
    ///
    /// Driven through the real Step 1 rather than only the Precombine Workspace, because the
    /// spawn must also have run first: the check is a postcondition of the `clean all` request,
    /// not a precondition.
    #[test]
    fn step_one_fails_an_xbox_run_that_leaves_no_geometry_psg() {
        let fixture = step_one_fixture(BuildMode::Xbox, true);
        let config = fixture.run.config().clone();
        let ck_log = fixture.ck_log_path();
        let psg = config
            .fo4edit_data_dir()
            .join(format!("{} - Geometry.psg", config.plugin.base_name));
        let process = RecordingProcessRunner::new().with_effects(&fixture.files, move |space| {
            record_successful_precombine_outputs(space, &config, &ck_log);
            // Creation Kit ran but wrote no PSG — the one output this test is about.
            space.remove_file(&psg).unwrap();
        });

        let err = fixture.run_step_one(&process).unwrap_err();

        assert!(matches!(err, Error::MissingGeometryPsg(name) if name == "MyMod"));
        assert_eq!(process.calls().len(), 1);
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
        // The Creation Kit log reached the session log rather than being read and dropped, and
        // it arrived framed: attributable to the operation that produced it, and bracketed by
        // the run's own timestamps.
        let session = fixture.files.read_lossy(fixture.run.log_path()).unwrap();
        assert!(session.contains(QUIET_CK_LOG), "session log: {session}");
        assert!(
            session.contains("Running CK option GeneratePrecombined:"),
            "session log: {session}"
        );
        assert!(
            session.contains(&format!(
                "Start {CK_RUN_STARTED_AT}\nEnded {CK_RUN_ENDED_AT}\n"
            )),
            "session log: {session}"
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

    /// A Creation Kit that exits non-zero but leaves every output behind completes Step 1 with
    /// exactly one warning, in the batch's words, on the console and in the session log (batch
    /// 472).
    #[test]
    fn step_one_warns_about_a_non_zero_exit_that_still_produced_its_outputs() {
        let fixture = step_one_fixture(BuildMode::Clean, true);
        let config = fixture.run.config().clone();
        let ck_log = fixture.ck_log_path();
        let process = RecordingProcessRunner::new()
            .returning_exit_code(3)
            .with_effects(&fixture.files, move |space| {
                record_successful_precombine_outputs(space, &config, &ck_log);
            });

        let (result, warnings) = fixture.run_step_one_collecting_warnings(&process);

        result.unwrap();
        assert_eq!(
            warnings,
            vec![BuildWarning::CreationKitNonZeroExit {
                operation: "GeneratePrecombined",
                code: Some(3),
            }]
        );
        let session = fixture.files.read_lossy(fixture.run.log_path()).unwrap();
        assert!(
            session.ends_with(
                "WARNING - GeneratePrecombined ended with error 3 but seemed to finish so error ignored.\n"
            ),
            "session log: {session}"
        );
    }

    /// The same exit with the output missing stops on the output, and says nothing about the
    /// exit: "seemed to finish" is never claimed for a run that did not (batch 471 before 472).
    #[test]
    fn step_one_raises_no_exit_warning_when_its_output_is_missing() {
        let fixture = step_one_fixture(BuildMode::Clean, true);
        let config = fixture.run.config().clone();
        let ck_log = fixture.ck_log_path();
        let combined_objects = config.fo4edit_data_dir().join("CombinedObjects.esp");
        let process = RecordingProcessRunner::new()
            .returning_exit_code(3)
            .with_effects(&fixture.files, move |space| {
                record_successful_precombine_outputs(space, &config, &ck_log);
                space.remove_file(&combined_objects).unwrap();
            });

        let (result, warnings) = fixture.run_step_one_collecting_warnings(&process);

        assert!(matches!(result, Err(Error::MissingCombinedObjects)));
        assert!(warnings.is_empty(), "warnings: {warnings:?}");
        let session = fixture.files.read_lossy(fixture.run.log_path()).unwrap();
        assert!(!session.contains("WARNING - "), "session log: {session}");
    }

    /// Regression pin for the `:RunCK` fall-through divergence (docs/episodes.md § *Creation
    /// Kit*): the batch's non-interactive `goto :eof` inside a `Call` carries on past a missing
    /// output, and the port deliberately stops instead.
    #[test]
    fn a_non_interactive_step_one_with_no_combined_objects_stops() {
        let fixture = step_one_fixture(BuildMode::Filtered, true);
        assert!(fixture.run.config().non_interactive);
        // Creation Kit ran, exited cleanly, and wrote nothing at all.
        let process = RecordingProcessRunner::new();

        let err = fixture.run_step_one(&process).unwrap_err();

        assert!(matches!(err, Error::MissingCombinedObjects));
        assert_eq!(process.calls().len(), 1);
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

        assert_eq!(
            fixture.prompts.asked(),
            vec![Confirmation::ClearPrecombined(
                fixture.run.config().precombined_dir()
            )]
        );
        assert!(!fixture.files.is_file(&existing_mesh));
        assert_eq!(process.calls().len(), 1);
    }

    #[test]
    fn step_one_stops_when_the_clear_precombined_prompt_is_refused() {
        let fixture = OperationFixture {
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
        assert_eq!(
            fixture.prompts.asked(),
            vec![Confirmation::ClearPrecombined(
                fixture.run.config().precombined_dir()
            )]
        );
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
        assert_eq!(fixture.prompts.asked(), []);
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
        assert_eq!(fixture.prompts.asked(), []);
    }

    /// `Data\<base name> - Geometry.psg` for the fixture's plugin.
    fn geometry_psg(fixture: &OperationFixture) -> PathBuf {
        fixture
            .run
            .config()
            .fo4edit_data_dir()
            .join("MyMod - Geometry.psg")
    }

    /// `Data\<base name> - Geometry.csg` for the fixture's plugin.
    fn geometry_csg(fixture: &OperationFixture) -> PathBuf {
        fixture
            .run
            .config()
            .fo4edit_data_dir()
            .join("MyMod - Geometry.csg")
    }

    /// A Creation Kit spawn that leaves a successful `CompressPSG` run's outputs behind.
    fn successful_compress_spawn(
        fixture: &OperationFixture,
        exit_code: i32,
    ) -> RecordingProcessRunner<'_> {
        let config = fixture.run.config().clone();
        let ck_log = fixture.ck_log_path();

        RecordingProcessRunner::new()
            .returning_exit_code(exit_code)
            .with_effects(&fixture.files, move |space| {
                record_successful_compress_outputs(space, &config, &ck_log);
            })
    }

    #[test]
    fn compress_psg_is_registered_and_requires_creation_kit() {
        let source = production_operation_source();
        let requirements = source.toolchain_requirements_for_steps(&[WorkflowStep::CompressPsg]);

        assert!(source.contains(WorkflowStep::CompressPsg));
        assert!(requirements.needs_creation_kit());
        assert!(!requirements.needs_fo4edit());
        assert!(!requirements.needs_archive());
    }

    /// A Clean resume at 4 runs Steps 4 to 6: Step 7 is the first planned step with no
    /// registered operation.
    #[test]
    fn a_clean_resume_at_step_four_runs_steps_four_to_six() {
        let plan =
            production_workflow_plan(BuildMode::Clean, Some(WorkflowStep::CompressPsg)).unwrap();

        assert_eq!(
            plan.planned_steps().first(),
            Some(&WorkflowStep::CompressPsg)
        );
        assert_eq!(
            plan.runnable_steps(),
            &[
                WorkflowStep::CompressPsg,
                WorkflowStep::BuildCdx,
                WorkflowStep::GeneratePrevis,
            ]
        );
    }

    /// Batch parity: `CHOICE /C:123456780` accepts a hidden 4 in Filtered, and `:CompPSG`
    /// forwards Filtered to `:PreVis` (296), so the run plans from Step 6 — a *non-resume*
    /// entry to it, which is why Step 6 hard-stops on a non-empty `vis` there.
    #[test]
    fn a_filtered_resume_at_step_four_plans_from_step_six() {
        assert_eq!(
            WorkflowPlan::steps_for(BuildMode::Filtered, Some(WorkflowStep::CompressPsg)),
            [
                WorkflowStep::GeneratePrevis,
                WorkflowStep::MergePrevis,
                WorkflowStep::AddPrevisToArchive,
            ]
        );

        let plan =
            production_workflow_plan(BuildMode::Filtered, Some(WorkflowStep::CompressPsg)).unwrap();

        assert_eq!(plan.runnable_steps(), &[WorkflowStep::GeneratePrevis]);
    }

    /// V2.99 Xbox checks the `.psg` and then skips `CompressPSG` (297–298): the geometry file
    /// it ships is the uncompressed one, so nothing may touch it.
    #[test]
    fn step_four_keeps_an_xbox_geometry_psg_without_running_creation_kit() {
        let fixture = step_four_fixture(BuildMode::Xbox);
        let psg = geometry_psg(&fixture);
        fixture.files.add_file_with_contents(&psg, "geometry");
        let session_before = fixture.files.read_lossy(fixture.run.log_path()).unwrap();
        let process = successful_compress_spawn(&fixture, 0);

        let (result, warnings) = fixture.run_step_four_collecting_warnings(&process);

        result.unwrap();
        assert!(process.calls().is_empty());
        assert!(fixture.wait.delays().is_empty());
        assert!(warnings.is_empty(), "warnings: {warnings:?}");
        assert_eq!(fixture.files.read_lossy(&psg).unwrap(), "geometry");
        assert!(!fixture.files.is_file(&geometry_csg(&fixture)));
        // The skip is a console-only note, not a Build Warning: the session log is untouched.
        assert_eq!(
            fixture.files.read_lossy(fixture.run.log_path()).unwrap(),
            session_before
        );
    }

    /// Both clean builds stop on a missing `.psg` before anything is spawned (batch 297).
    #[test]
    fn step_four_stops_on_a_missing_geometry_psg_before_creation_kit() {
        for mode in [BuildMode::Clean, BuildMode::Xbox] {
            let fixture = step_four_fixture(mode);
            let process = successful_compress_spawn(&fixture, 0);

            let err = fixture.run_step_four(&process).unwrap_err();

            assert!(
                matches!(&err, Error::MissingGeometryPsg(name) if name == "MyMod"),
                "mode: {mode:?}, error: {err:?}"
            );
            assert!(process.calls().is_empty(), "mode: {mode:?}");
            assert!(fixture.wait.delays().is_empty(), "mode: {mode:?}");
        }
    }

    /// Clean compresses the `.psg` into a `.csg` and only then deletes the `.psg` (299–301).
    #[test]
    fn step_four_compresses_and_then_deletes_the_geometry_psg_in_a_clean_build() {
        let fixture = step_four_fixture(BuildMode::Clean);
        fixture.files.add_file(geometry_psg(&fixture));
        let process = successful_compress_spawn(&fixture, 0);

        let (result, warnings) = fixture.run_step_four_collecting_warnings(&process);

        result.unwrap();
        assert!(warnings.is_empty(), "warnings: {warnings:?}");
        assert!(fixture.files.is_file(&geometry_csg(&fixture)));
        assert!(!fixture.files.is_file(&geometry_psg(&fixture)));
        // One spawn of the run's own Creation Kit for the run's own plugin, with the whole
        // episode around it; the verb itself is pinned in `tools::creation_kit`.
        let calls = process.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].exe, fixture.run.creation_kit().unwrap().exe);
        assert!(
            calls[0]
                .args
                .iter()
                .any(|arg| arg.to_string_lossy().contains("MyMod.esp")),
            "args: {:?}",
            calls[0].args
        );
        assert_eq!(fixture.wait.delays(), vec![MO2_DELAY_AFTER_CK_SECS]);
        let session = fixture.files.read_lossy(fixture.run.log_path()).unwrap();
        assert!(
            session.contains("Running CK option CompressPSG:"),
            "session log: {session}"
        );
    }

    /// Data-loss pin. A `.csg` left by an earlier run must not pass for this run's output: if
    /// it did, a Creation Kit that wrote nothing would get the only `.psg` deleted. The stale
    /// `.csg` is cleared before the spawn (a divergence — the batch never clears it), so the
    /// output check fails and the run stops, non-interactively, with the `.psg` intact.
    #[test]
    fn a_stale_csg_never_lets_a_silent_creation_kit_delete_the_geometry_psg() {
        let fixture = step_four_fixture(BuildMode::Clean);
        assert!(fixture.run.config().non_interactive);
        fixture
            .files
            .add_file_with_contents(geometry_psg(&fixture), "geometry");
        fixture.files.add_file(geometry_csg(&fixture));
        // Creation Kit ran, exited cleanly, and wrote nothing at all.
        let process = RecordingProcessRunner::new();

        let (result, warnings) = fixture.run_step_four_collecting_warnings(&process);

        let err = result.unwrap_err();
        assert_eq!(
            err.to_string(),
            "CompressPSG failed to create file MyMod - Geometry.csg with exit status 0"
        );
        assert!(warnings.is_empty(), "warnings: {warnings:?}");
        assert_eq!(process.calls().len(), 1);
        assert_eq!(
            fixture.files.read_lossy(&geometry_psg(&fixture)).unwrap(),
            "geometry"
        );
        assert!(!fixture.files.is_file(&geometry_csg(&fixture)));
    }

    /// A non-zero exit with the `.csg` present completes with one warning in the batch's
    /// words, and the `.psg` is still cleaned up (472, then 301).
    #[test]
    fn step_four_warns_about_a_non_zero_exit_that_still_produced_its_csg() {
        let fixture = step_four_fixture(BuildMode::Clean);
        fixture.files.add_file(geometry_psg(&fixture));
        let process = successful_compress_spawn(&fixture, 3);

        let (result, warnings) = fixture.run_step_four_collecting_warnings(&process);

        result.unwrap();
        assert_eq!(
            warnings,
            vec![BuildWarning::CreationKitNonZeroExit {
                operation: "CompressPSG",
                code: Some(3),
            }]
        );
        let session = fixture.files.read_lossy(fixture.run.log_path()).unwrap();
        assert!(
            session.ends_with(
                "WARNING - CompressPSG ended with error 3 but seemed to finish so error ignored.\n"
            ),
            "session log: {session}"
        );
        assert!(!fixture.files.is_file(&geometry_psg(&fixture)));
    }

    /// The same exit without the `.csg` stops on the output and says nothing about the exit,
    /// and the `.psg` survives (batch 471 before 472 and 301).
    #[test]
    fn step_four_raises_no_exit_warning_when_its_csg_is_missing() {
        let fixture = step_four_fixture(BuildMode::Clean);
        fixture.files.add_file(geometry_psg(&fixture));
        let process = RecordingProcessRunner::new().returning_exit_code(3);

        let (result, warnings) = fixture.run_step_four_collecting_warnings(&process);

        let err = result.unwrap_err();
        assert!(
            matches!(
                &err,
                Error::MissingCreationKitOutput { operation, file, code }
                    if *operation == "CompressPSG"
                        && file == "MyMod - Geometry.csg"
                        && *code == Some(3)
            ),
            "error: {err:?}"
        );
        assert!(warnings.is_empty(), "warnings: {warnings:?}");
        let session = fixture.files.read_lossy(fixture.run.log_path()).unwrap();
        assert!(!session.contains("WARNING - "), "session log: {session}");
        assert!(fixture.files.is_file(&geometry_psg(&fixture)));
    }

    /// `Data\<base name>.cdx` for the fixture's plugin.
    fn cdx(fixture: &OperationFixture) -> PathBuf {
        fixture.run.config().fo4edit_data_dir().join("MyMod.cdx")
    }

    /// A Creation Kit spawn that leaves a successful `BuildCDX` run's outputs behind.
    fn successful_cdx_spawn(
        fixture: &OperationFixture,
        exit_code: i32,
    ) -> RecordingProcessRunner<'_> {
        let config = fixture.run.config().clone();
        let ck_log = fixture.ck_log_path();

        RecordingProcessRunner::new()
            .returning_exit_code(exit_code)
            .with_effects(&fixture.files, move |space| {
                record_successful_cdx_outputs(space, &config, &ck_log);
            })
    }

    #[test]
    fn build_cdx_is_registered_and_requires_creation_kit() {
        let source = production_operation_source();
        let requirements = source.toolchain_requirements_for_steps(&[WorkflowStep::BuildCdx]);

        assert!(source.contains(WorkflowStep::BuildCdx));
        assert!(requirements.needs_creation_kit());
        assert!(!requirements.needs_fo4edit());
        assert!(!requirements.needs_archive());
    }

    /// Registering Steps 4 to 6 leaves a fresh clean build where it was: Step 2 is still the
    /// first planned step with no registered operation, so only Step 1 runs.
    #[test]
    fn a_fresh_clean_build_still_runs_only_the_contiguous_registered_prefix() {
        for mode in [BuildMode::Clean, BuildMode::Xbox] {
            let plan = production_workflow_plan(mode, None).unwrap();

            assert_eq!(plan.planned_steps().len(), 8, "mode: {mode:?}");
            assert_eq!(
                plan.runnable_steps(),
                &[WorkflowStep::GeneratePrecombines],
                "mode: {mode:?}"
            );
        }
    }

    /// A Clean or Xbox resume at 5 runs Steps 5 and 6: Step 7 is not registered yet.
    #[test]
    fn a_clean_build_resumed_at_step_five_runs_steps_five_and_six() {
        for mode in [BuildMode::Clean, BuildMode::Xbox] {
            let plan = production_workflow_plan(mode, Some(WorkflowStep::BuildCdx)).unwrap();

            assert_eq!(
                plan.runnable_steps(),
                &[WorkflowStep::BuildCdx, WorkflowStep::GeneratePrevis],
                "mode: {mode:?}"
            );
        }
    }

    /// Batch parity, as for a hidden 4: `:BldCDX` forwards Filtered to `:PreVis` (305), so a
    /// Filtered resume at 5 plans from Step 6.
    #[test]
    fn a_filtered_resume_at_step_five_plans_from_step_six() {
        let plan =
            production_workflow_plan(BuildMode::Filtered, Some(WorkflowStep::BuildCdx)).unwrap();

        assert_eq!(plan.runnable_steps(), &[WorkflowStep::GeneratePrevis]);
    }

    /// Step 5 has no pre-checks of its own (304–307): a resume at 5 enters at `:BldCDX` (231)
    /// and skips Step 4's `.psg` check, so a missing `.psg` must not stop it.
    #[test]
    fn a_resume_at_step_five_builds_the_cdx_without_a_geometry_psg() {
        for mode in [BuildMode::Clean, BuildMode::Xbox] {
            let fixture = step_five_fixture(mode);
            assert!(!fixture.files.is_file(&geometry_psg(&fixture)));
            let process = successful_cdx_spawn(&fixture, 0);

            let (result, warnings) = fixture.run_step_five_collecting_warnings(&process);

            result.unwrap();
            assert!(
                warnings.is_empty(),
                "mode: {mode:?}, warnings: {warnings:?}"
            );
            assert!(fixture.files.is_file(&cdx(&fixture)), "mode: {mode:?}");
            // One spawn of the run's own Creation Kit for the run's own plugin, with the whole
            // episode around it; the verb itself is pinned in `tools::creation_kit`.
            let calls = process.calls();
            assert_eq!(calls.len(), 1, "mode: {mode:?}");
            assert_eq!(calls[0].exe, fixture.run.creation_kit().unwrap().exe);
            assert!(
                calls[0]
                    .args
                    .iter()
                    .any(|arg| arg.to_string_lossy().contains("MyMod.esp")),
                "mode: {mode:?}, args: {:?}",
                calls[0].args
            );
            assert_eq!(fixture.wait.delays(), vec![MO2_DELAY_AFTER_CK_SECS]);
            let session = fixture.files.read_lossy(fixture.run.log_path()).unwrap();
            assert!(
                session.contains("Running CK option BuildCDX:"),
                "mode: {mode:?}, session log: {session}"
            );
        }
    }

    /// A `.cdx` left by an earlier run is gone by the time Creation Kit is spawned (a
    /// divergence — the batch never clears it), so the `.cdx` the output check finds is this
    /// run's.
    #[test]
    fn step_five_deletes_a_stale_cdx_before_creation_kit_runs() {
        let fixture = step_five_fixture(BuildMode::Clean);
        fixture.files.add_file_with_contents(cdx(&fixture), "stale");
        let config = fixture.run.config().clone();
        let ck_log = fixture.ck_log_path();
        let cdx_path = cdx(&fixture);
        let stale_at_spawn = std::cell::Cell::new(None);
        let process = RecordingProcessRunner::new().with_effects(&fixture.files, |space| {
            stale_at_spawn.set(Some(space.is_file(&cdx_path)));
            record_successful_cdx_outputs(space, &config, &ck_log);
        });

        fixture.run_step_five(&process).unwrap();

        assert_eq!(stale_at_spawn.get(), Some(false));
        assert!(fixture.files.is_file(&cdx(&fixture)));
    }

    /// A stale `.cdx` must not pass for this run's output: with it cleared before the spawn, a
    /// Creation Kit that writes nothing stops the run, non-interactively, with the shared
    /// missing-output error.
    #[test]
    fn a_stale_cdx_never_passes_for_a_silent_creation_kit_run() {
        let fixture = step_five_fixture(BuildMode::Clean);
        assert!(fixture.run.config().non_interactive);
        fixture.files.add_file(cdx(&fixture));
        // Creation Kit ran, exited cleanly, and wrote nothing at all.
        let process = RecordingProcessRunner::new();

        let (result, warnings) = fixture.run_step_five_collecting_warnings(&process);

        let err = result.unwrap_err();
        assert_eq!(
            err.to_string(),
            "BuildCDX failed to create file MyMod.cdx with exit status 0"
        );
        assert!(warnings.is_empty(), "warnings: {warnings:?}");
        assert_eq!(process.calls().len(), 1);
        assert!(!fixture.files.is_file(&cdx(&fixture)));
    }

    /// A non-zero exit with the `.cdx` present completes with one warning in the batch's words
    /// (472).
    #[test]
    fn step_five_warns_about_a_non_zero_exit_that_still_produced_its_cdx() {
        let fixture = step_five_fixture(BuildMode::Clean);
        let process = successful_cdx_spawn(&fixture, 3);

        let (result, warnings) = fixture.run_step_five_collecting_warnings(&process);

        result.unwrap();
        assert_eq!(
            warnings,
            vec![BuildWarning::CreationKitNonZeroExit {
                operation: "BuildCDX",
                code: Some(3),
            }]
        );
        let session = fixture.files.read_lossy(fixture.run.log_path()).unwrap();
        assert!(
            session.ends_with(
                "WARNING - BuildCDX ended with error 3 but seemed to finish so error ignored.\n"
            ),
            "session log: {session}"
        );
        assert!(fixture.files.is_file(&cdx(&fixture)));
    }

    /// The same exit without the `.cdx` stops on the output and says nothing about the exit
    /// (batch 471 before 472).
    #[test]
    fn step_five_raises_no_exit_warning_when_its_cdx_is_missing() {
        let fixture = step_five_fixture(BuildMode::Clean);
        let process = RecordingProcessRunner::new().returning_exit_code(3);

        let (result, warnings) = fixture.run_step_five_collecting_warnings(&process);

        let err = result.unwrap_err();
        assert!(
            matches!(
                &err,
                Error::MissingCreationKitOutput { operation, file, code }
                    if *operation == "BuildCDX" && file == "MyMod.cdx" && *code == Some(3)
            ),
            "error: {err:?}"
        );
        assert!(warnings.is_empty(), "warnings: {warnings:?}");
        let session = fixture.files.read_lossy(fixture.run.log_path()).unwrap();
        assert!(!session.contains("WARNING - "), "session log: {session}");
    }

    /// `Data\vis\old\cluster.uvd`: previs an earlier run left behind.
    ///
    /// Nested, and named apart from the cluster the simulated Creation Kit writes, so a test can
    /// tell a cleared `vis` from one that was never touched.
    fn stale_vis_uvd(fixture: &OperationFixture) -> PathBuf {
        fixture
            .run
            .config()
            .vis_dir()
            .join("old")
            .join("cluster.uvd")
    }

    /// `Data\Previs.esp` for the fixture.
    fn previs_plugin(fixture: &OperationFixture) -> PathBuf {
        fixture.run.config().fo4edit_data_dir().join("Previs.esp")
    }

    /// A Creation Kit spawn that leaves a successful `GeneratePreVisData` run's outputs behind,
    /// with `ck_log` as the log it wrote, or no log at all for `None`.
    fn previs_spawn<'a>(
        fixture: &'a OperationFixture,
        exit_code: i32,
        ck_log: Option<&'static str>,
    ) -> RecordingProcessRunner<'a> {
        let config = fixture.run.config().clone();
        let ck_log_path = fixture.ck_log_path();

        RecordingProcessRunner::new()
            .returning_exit_code(exit_code)
            .with_effects(&fixture.files, move |space| {
                record_successful_previs_outputs(space, &config, &ck_log_path);
                match ck_log {
                    Some(contents) => space.add_file_with_contents(&ck_log_path, contents),
                    None => space.remove_file(&ck_log_path).unwrap(),
                }
            })
    }

    /// A Creation Kit spawn that leaves a successful, quiet `GeneratePreVisData` run behind.
    fn successful_previs_spawn(fixture: &OperationFixture) -> RecordingProcessRunner<'_> {
        previs_spawn(fixture, 0, Some(QUIET_CK_LOG))
    }

    #[test]
    fn generate_previs_is_registered_and_requires_creation_kit() {
        let source = production_operation_source();
        let requirements = source.toolchain_requirements_for_steps(&[WorkflowStep::GeneratePrevis]);

        assert!(source.contains(WorkflowStep::GeneratePrevis));
        assert!(requirements.needs_creation_kit());
        assert!(!requirements.needs_fo4edit());
        assert!(!requirements.needs_archive());
    }

    /// A resume at 6 over an empty `vis` runs the whole Creation Kit episode once and leaves a
    /// fresh `Previs.esp`, with nothing asked and nothing to warn about.
    #[test]
    fn step_six_generates_previs_over_an_empty_vis_directory() {
        let fixture = step_six_fixture(BuildMode::Clean, true);
        let process = successful_previs_spawn(&fixture);

        let (result, warnings) = fixture.run_step_six_collecting_warnings(&process);

        result.unwrap();
        assert!(warnings.is_empty(), "warnings: {warnings:?}");
        assert_eq!(fixture.prompts.asked(), []);
        assert!(fixture.files.is_file(&previs_plugin(&fixture)));
        // One spawn of the run's own Creation Kit for the run's own plugin, with the whole
        // episode around it; the verb itself is pinned in `tools::creation_kit`.
        let calls = process.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].exe, fixture.run.creation_kit().unwrap().exe);
        assert!(
            calls[0]
                .args
                .iter()
                .any(|arg| arg.to_string_lossy().contains("MyMod.esp")),
            "args: {:?}",
            calls[0].args
        );
        assert_eq!(fixture.wait.delays(), vec![MO2_DELAY_AFTER_CK_SECS]);
        let session = fixture.files.read_lossy(fixture.run.log_path()).unwrap();
        assert!(
            session.contains("Running CK option GeneratePreVisData:"),
            "session log: {session}"
        );
    }

    /// `clean all` is hardcoded for every Build Mode (317), unlike Step 1's qualifiers, so every
    /// mode sends Creation Kit the same previs request. Pinned by sameness here; the qualifier
    /// text itself is pinned in `tools::creation_kit`, which keeps the grammar out of this
    /// module.
    #[test]
    fn step_six_sends_the_same_previs_request_in_every_build_mode() {
        let argv_for = |mode| {
            let fixture = step_six_fixture(mode, true);
            let process = successful_previs_spawn(&fixture);
            fixture.run_step_six(&process).unwrap();
            let mut calls = process.calls();
            assert_eq!(calls.len(), 1, "mode: {mode:?}");
            calls.remove(0).args
        };

        let clean = argv_for(BuildMode::Clean);

        assert_eq!(argv_for(BuildMode::Filtered), clean);
        assert_eq!(argv_for(BuildMode::Xbox), clean);
    }

    /// `:RePreVis` (245–248): an interactive resume at 6 asks, and **Y** clears the whole `vis`
    /// before Creation Kit runs.
    #[test]
    fn an_interactive_resume_at_step_six_clears_vis_when_the_operator_agrees() {
        let fixture = step_six_fixture(BuildMode::Clean, false);
        let stale = stale_vis_uvd(&fixture);
        fixture.files.add_file(&stale);
        let stale_at_spawn = std::cell::Cell::new(None);
        let config = fixture.run.config().clone();
        let ck_log = fixture.ck_log_path();
        let process = RecordingProcessRunner::new().with_effects(&fixture.files, |space| {
            stale_at_spawn.set(Some(space.is_file(&stale)));
            record_successful_previs_outputs(space, &config, &ck_log);
        });

        fixture.run_step_six(&process).unwrap();

        assert_eq!(
            fixture.prompts.asked(),
            vec![Confirmation::ClearVis(fixture.run.config().vis_dir())]
        );
        assert_eq!(stale_at_spawn.get(), Some(false));
        assert!(!fixture.files.is_file(&stale));
        assert_eq!(process.calls().len(), 1);
    }

    /// **N** stops the run before anything is deleted or spawned, telling the operator to pick
    /// another resume step, as Step 1's refusal does.
    #[test]
    fn an_interactive_resume_at_step_six_stops_when_the_operator_refuses() {
        let fixture = OperationFixture {
            prompts: RecordingPrompts::new().refusing_clear_vis(),
            ..step_six_fixture(BuildMode::Clean, false)
        };
        let stale = stale_vis_uvd(&fixture);
        fixture.files.add_file(&stale);
        fixture.files.add_file(previs_plugin(&fixture));
        let process = successful_previs_spawn(&fixture);

        let err = fixture.run_step_six(&process).unwrap_err();

        assert!(
            matches!(&err, Error::Other(message)
                if message == "previs directory not cleared - choose another resume step"),
            "error: {err:?}"
        );
        assert_eq!(
            fixture.prompts.asked(),
            vec![Confirmation::ClearVis(fixture.run.config().vis_dir())]
        );
        assert_eq!(process.calls().len(), 0);
        assert!(fixture.files.is_file(&stale));
        // The precondition runs before the stale-plugin delete, so a refusal touches nothing.
        assert!(fixture.files.is_file(&previs_plugin(&fixture)));
    }

    /// An unattended resume at 6 never blocks on a prompt: it stops for the operator to clear
    /// `vis` themselves.
    #[test]
    fn a_non_interactive_resume_at_step_six_stops_on_a_non_empty_vis_without_asking() {
        let fixture = step_six_fixture(BuildMode::Clean, true);
        let stale = stale_vis_uvd(&fixture);
        fixture.files.add_file(&stale);
        let process = successful_previs_spawn(&fixture);

        let err = fixture.run_step_six(&process).unwrap_err();

        assert!(matches!(err, Error::VisUvdFilesExist), "error: {err:?}");
        assert_eq!(fixture.prompts.asked(), []);
        assert_eq!(process.calls().len(), 0);
        assert_eq!(fixture.wait.delays(), []);
        assert!(fixture.files.is_file(&stale));
    }

    /// Every entry other than a resume at 6 is `:PreVis`, which hard-stops on a non-empty `vis`
    /// (311–313) even when interactive: a fresh run, and resumes at 4 or 5 — including a
    /// Filtered resume at 4 or 5, which is forwarded to `:PreVis` (296, 305).
    #[test]
    fn any_other_entry_to_step_six_stops_on_a_non_empty_vis_without_asking() {
        let entries = [
            (BuildMode::Clean, None),
            (BuildMode::Clean, Some(WorkflowStep::CompressPsg)),
            (BuildMode::Clean, Some(WorkflowStep::BuildCdx)),
            (BuildMode::Filtered, Some(WorkflowStep::CompressPsg)),
            (BuildMode::Filtered, Some(WorkflowStep::BuildCdx)),
        ];

        for (mode, resume_from) in entries {
            let fixture = operation_fixture(mode, false, resume_from);
            let stale = stale_vis_uvd(&fixture);
            fixture.files.add_file(&stale);
            let process = successful_previs_spawn(&fixture);

            let err = fixture.run_step_six(&process).unwrap_err();

            let entry = format!("mode: {mode:?}, resume: {resume_from:?}");
            assert!(
                matches!(err, Error::VisUvdFilesExist),
                "{entry}, error: {err:?}"
            );
            assert!(fixture.prompts.asked().is_empty(), "{entry}");
            assert!(process.calls().is_empty(), "{entry}");
            assert!(fixture.files.is_file(&stale), "{entry}");
        }
    }

    /// A `Previs.esp` from an earlier run is gone by the time Creation Kit is spawned (315).
    #[test]
    fn step_six_deletes_a_stale_previs_plugin_before_creation_kit_runs() {
        let fixture = step_six_fixture(BuildMode::Clean, true);
        fixture
            .files
            .add_file_with_contents(previs_plugin(&fixture), "stale");
        let config = fixture.run.config().clone();
        let ck_log = fixture.ck_log_path();
        let previs = previs_plugin(&fixture);
        let stale_at_spawn = std::cell::Cell::new(None);
        let process = RecordingProcessRunner::new().with_effects(&fixture.files, |space| {
            stale_at_spawn.set(Some(space.is_file(&previs)));
            record_successful_previs_outputs(space, &config, &ck_log);
        });

        fixture.run_step_six(&process).unwrap();

        assert_eq!(stale_at_spawn.get(), Some(false));
        assert!(fixture.files.is_file(&previs_plugin(&fixture)));
    }

    /// A stale `Previs.esp` must not pass for this run's output, or Step 7 would merge the
    /// wrong plugin: a Creation Kit that writes nothing stops the run, non-interactively, with
    /// the shared missing-output error.
    #[test]
    fn a_stale_previs_plugin_never_passes_for_a_silent_creation_kit_run() {
        let fixture = step_six_fixture(BuildMode::Clean, true);
        assert!(fixture.run.config().non_interactive);
        fixture.files.add_file(previs_plugin(&fixture));
        // Creation Kit ran, exited cleanly, and wrote nothing at all.
        let process = RecordingProcessRunner::new();

        let (result, warnings) = fixture.run_step_six_collecting_warnings(&process);

        let err = result.unwrap_err();
        assert_eq!(
            err.to_string(),
            "GeneratePreVisData failed to create file Previs.esp with exit status 0"
        );
        assert!(warnings.is_empty(), "warnings: {warnings:?}");
        assert_eq!(process.calls().len(), 1);
        assert!(!fixture.files.is_file(&previs_plugin(&fixture)));
    }

    /// With `Previs.esp` confirmed, both warnings are raised: visibility first, then the
    /// non-zero exit, on the console and in the session log in that order.
    #[test]
    fn step_six_raises_the_visibility_warning_before_the_exit_warning() {
        let fixture = step_six_fixture(BuildMode::Clean, true);
        let process = previs_spawn(
            &fixture,
            3,
            Some("Masterfile: Fallout4.esm\nDEFAULT: ERROR: visibility task did not complete.\n"),
        );

        let (result, warnings) = fixture.run_step_six_collecting_warnings(&process);

        result.unwrap();
        assert_eq!(
            warnings,
            vec![
                BuildWarning::VisibilityTaskIncomplete,
                BuildWarning::CreationKitNonZeroExit {
                    operation: "GeneratePreVisData",
                    code: Some(3),
                },
            ]
        );
        let session = fixture.files.read_lossy(fixture.run.log_path()).unwrap();
        assert!(
            session.ends_with(
                "WARNING - GeneratePreVisData failed to build at least one Cluster uvd\n\
                 WARNING - GeneratePreVisData ended with error 3 but seemed to finish so error ignored.\n"
            ),
            "session log: {session}"
        );
    }

    /// The batch skips the visibility scan when Creation Kit wrote no log (318).
    #[test]
    fn step_six_raises_no_visibility_warning_when_creation_kit_wrote_no_log() {
        let fixture = step_six_fixture(BuildMode::Clean, true);
        let process = previs_spawn(&fixture, 0, None);

        let (result, warnings) = fixture.run_step_six_collecting_warnings(&process);

        result.unwrap();
        assert!(warnings.is_empty(), "warnings: {warnings:?}");
    }

    /// A missing `Previs.esp` stops the run and raises neither warning, even with a non-zero
    /// exit and the visibility marker in the log (471 before 472 and 319–320).
    #[test]
    fn step_six_raises_no_warning_when_its_previs_plugin_is_missing() {
        let fixture = step_six_fixture(BuildMode::Clean, true);
        let config = fixture.run.config().clone();
        let ck_log = fixture.ck_log_path();
        let previs = previs_plugin(&fixture);
        let process = RecordingProcessRunner::new()
            .returning_exit_code(3)
            .with_effects(&fixture.files, move |space| {
                record_successful_previs_outputs(space, &config, &ck_log);
                space.add_file_with_contents(&ck_log, "ERROR: visibility task did not complete.\n");
                space.remove_file(&previs).unwrap();
            });

        let (result, warnings) = fixture.run_step_six_collecting_warnings(&process);

        let err = result.unwrap_err();
        assert!(
            matches!(
                &err,
                Error::MissingCreationKitOutput { operation, file, code }
                    if *operation == "GeneratePreVisData"
                        && file == "Previs.esp"
                        && *code == Some(3)
            ),
            "error: {err:?}"
        );
        assert!(warnings.is_empty(), "warnings: {warnings:?}");
        let session = fixture.files.read_lossy(fixture.run.log_path()).unwrap();
        assert!(!session.contains("WARNING - "), "session log: {session}");
    }
}
