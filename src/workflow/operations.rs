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
use crate::tools::{ArchiveOps, CreationKitOps, Fo4EditOps};
use crate::warning::BuildWarnings;
use crate::workflow::WorkflowPlan;

mod add_previs_to_archive;
mod build_cdx;
mod compress_psg;
mod create_ba2_from_precombines;
mod generate_precombines;
mod generate_previs;
mod merge_combined_objects;
mod merge_previs;
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
    merge_combined_objects::DEFINITION,
    create_ba2_from_precombines::DEFINITION,
    compress_psg::DEFINITION,
    build_cdx::DEFINITION,
    generate_previs::DEFINITION,
    merge_previs::DEFINITION,
    add_previs_to_archive::DEFINITION,
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

    /// This source with the operations for `unregistered` taken out, for tests of plans that
    /// production, with every step registered, no longer produces.
    ///
    /// Leaks the filtered list, because a source holds `'static` definitions; a test-only
    /// allocation per call is harmless.
    #[cfg(test)]
    fn without(self, unregistered: &[WorkflowStep]) -> Self {
        let operations: Vec<_> = self
            .operations
            .iter()
            .filter(|operation| !unregistered.contains(&operation.step))
            .copied()
            .collect();
        Self::new(Vec::leak(operations))
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
    prepare_workflow(production_operation_source(), build_mode, resume_from)
}

/// [`prepare_production_workflow`] under production's registration with `unregistered` taken
/// out.
///
/// Every Workflow Step is registered in production, so a partial plan — a run that stops short,
/// a resume at a step with no operation, a plan that needs no archive tool — can no longer arise
/// there. This stands in for the partial registrations production had while the port was being
/// built, so the handling of those plans stays under test.
///
/// Returns [`Error::StepNotImplemented`] as [`prepare_production_workflow`] does.
#[cfg(test)]
pub(crate) fn prepare_workflow_without(
    unregistered: &[WorkflowStep],
    build_mode: BuildMode,
    resume_from: Option<WorkflowStep>,
) -> Result<ProductionWorkflowPreparation> {
    prepare_workflow(
        production_operation_source().without(unregistered),
        build_mode,
        resume_from,
    )
}

/// Resolve the Workflow Plan and its runnable-operation requirement union from one source, so
/// planning and readiness cannot observe different operation availability.
fn prepare_workflow(
    operations: ProductionOperationSource,
    build_mode: BuildMode,
    resume_from: Option<WorkflowStep>,
) -> Result<ProductionWorkflowPreparation> {
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
/// will, and the three tool episodes sit above the `ProcessRunner`, `Wait` and `DesktopWindows`
/// ports, which have two each.
///
/// `ck`, `fo4edit` and `archive` are concrete episodes rather than `&dyn` traits for that same
/// reason. Tests substitute underneath them, at `ProcessRunner`/`Wait`/`DesktopWindows`/
/// `FileSpace`, and run the real [`CreationKitOps`], [`Fo4EditOps`] and [`ArchiveOps`] — which
/// is what puts the DLL guard, the MO2 delays, the log lifecycle, the Module Selection ladder,
/// the close sequence, the archive swap and the BSArch move-backs under an operation's tests
/// instead of leaving their composition untested behind a one-implementation trait.
///
/// Each episode is optional, because a Workflow Run prepares only the tools its runnable
/// operations require: a resume at Step 7 needs FO4Edit and the archive tool but no Creation
/// Kit, and a resume at Step 8 needs only the archive tool. An operation reaches its episode
/// through [`Self::ck`], [`Self::fo4edit`] or [`Self::archive`], and an absent one is a
/// preparation bug rather than a user state.
#[derive(Debug)]
pub(crate) struct OperationPorts<'a> {
    /// The Creation Kit episode: a domain verb per batch operation, its command grammar hidden.
    ///
    /// `None` when no runnable operation required Creation Kit. Read it through [`Self::ck`].
    pub(crate) ck: Option<&'a CreationKitOps<'a>>,
    /// The FO4Edit episode: one domain verb per merge script, its command grammar hidden.
    ///
    /// `None` when no runnable operation required FO4Edit. Read it through [`Self::fo4edit`].
    pub(crate) fo4edit: Option<&'a Fo4EditOps<'a>>,
    /// The Archive episode: one domain verb per archive step, the tool's command grammar, the
    /// work folder and the swap hidden.
    ///
    /// `None` when no runnable operation required the archive tool. Read it through
    /// [`Self::archive`].
    pub(crate) archive: Option<&'a ArchiveOps<'a>>,
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

impl<'a> OperationPorts<'a> {
    /// The Creation Kit episode, for an operation registered with Creation Kit readiness.
    ///
    /// Returns [`Error::CreationKitNotPrepared`] when the Workflow Run prepared no Creation Kit,
    /// which means planning and readiness disagreed about the running operation.
    pub(crate) fn ck(&self) -> Result<&'a CreationKitOps<'a>> {
        self.ck.ok_or(Error::CreationKitNotPrepared)
    }

    /// The FO4Edit episode, for an operation registered with FO4Edit readiness.
    ///
    /// Returns [`Error::Fo4EditNotPrepared`] when the Workflow Run prepared no FO4Edit, which
    /// means planning and readiness disagreed about the running operation.
    pub(crate) fn fo4edit(&self) -> Result<&'a Fo4EditOps<'a>> {
        self.fo4edit.ok_or(Error::Fo4EditNotPrepared)
    }

    /// The Archive episode, for an operation registered with archive readiness.
    ///
    /// Returns [`Error::ArchiveNotPrepared`] when the Workflow Run prepared no archive tool,
    /// which means planning and readiness disagreed about the running operation.
    pub(crate) fn archive(&self) -> Result<&'a ArchiveOps<'a>> {
        self.archive.ok_or(Error::ArchiveNotPrepared)
    }
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
        COMPLETED_COMBINED_OBJECTS_MERGE_LOG, COMPLETED_PREVIS_MERGE_LOG, FO4EDIT_MAIN_FORM,
        FO4EDIT_MODULE_SELECTION, ONE_POLL_MERGE_DELAYS, PACKED_ARCHIVE, QUIET_CK_LOG,
        RecordingPrompts, archive2_extract_writing_precombines, archive2_pack_writing_archive,
        behaving_fo4edit_windows, fo4edit_exiting_when, record_successful_cdx_outputs,
        record_successful_combined_objects_merge, record_successful_compress_outputs,
        record_successful_precombine_outputs, record_successful_previs_merge,
        record_successful_previs_outputs,
    };
    use super::*;
    use crate::config::{ArchiveTool, BuildMode, PluginIdentity};
    use crate::files::InMemoryFileSpace;
    use crate::logging;
    use crate::run::WorkflowRequest;
    use crate::tools::clock::ScriptedClock;
    use crate::tools::desktop::{DesktopAction, DesktopWindows, RecordingDesktopWindows};
    use crate::tools::process::{
        ExitFlag, ProcessCallKind, ProcessRunner, RecordedProcessCall, RecordingProcessRunner,
    };
    use crate::tools::wait::{
        MO2_DELAY_AFTER_ARCHIVE2_EXTRACT_SECS, MO2_DELAY_AFTER_CK_SECS, RecordingWait,
    };
    use crate::tools::{ArchivePorts, CkPorts, Fo4EditPaths, Fo4EditPorts};
    use crate::warning::BuildWarning;
    use crate::{
        discovery::ToolPaths,
        toolchain::{WorkflowToolchainProbe, archive2_exe, write_fo4edit_install},
    };

    /// Provide an execution entry for registration-only tests that must never dispatch.
    fn unused_execution(_run: &WorkflowRun, _ports: &OperationPorts<'_>) -> Result<()> {
        unreachable!("filtering registered steps must not execute operations")
    }

    /// The Clean steps production runs: every one, since Step 8 was registered.
    const CLEAN_RUNNABLE_STEPS: [WorkflowStep; 8] = [
        WorkflowStep::GeneratePrecombines,
        WorkflowStep::MergePrecombineObjects,
        WorkflowStep::CreateBa2FromPrecombines,
        WorkflowStep::CompressPsg,
        WorkflowStep::BuildCdx,
        WorkflowStep::GeneratePrevis,
        WorkflowStep::MergePrevis,
        WorkflowStep::AddPrevisToArchive,
    ];

    #[test]
    fn production_workflow_plan_runs_every_planned_step() {
        let plan = production_workflow_plan(BuildMode::Clean, None).unwrap();

        assert_eq!(plan.planned_steps().len(), 8);
        assert_eq!(plan.runnable_steps(), &CLEAN_RUNNABLE_STEPS);
        assert_eq!(plan.skipped_unrunnable_count(), 0);
    }

    /// Production registers every step, so a registration without Step 8 stands in for the
    /// unregistered resume point it no longer has.
    #[test]
    fn a_registration_rejects_a_resume_at_an_unregistered_step() {
        let cases = [
            (BuildMode::Clean, WorkflowStep::AddPrevisToArchive, 8),
            (BuildMode::Filtered, WorkflowStep::AddPrevisToArchive, 8),
        ];

        for (mode, resume, expected_step) in cases {
            let Err(error) =
                prepare_workflow_without(&[WorkflowStep::AddPrevisToArchive], mode, Some(resume))
            else {
                panic!("mode: {mode:?}, resume: {resume:?}: a plan was prepared");
            };

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

        assert_eq!(plan.runnable_steps(), &CLEAN_RUNNABLE_STEPS);
        assert!(requirements.needs_creation_kit());
        assert!(requirements.needs_fo4edit());
        assert!(requirements.needs_archive());
    }

    #[test]
    fn production_source_registers_every_step_in_plan_order() {
        let source = production_operation_source();
        let plan = source.workflow_plan(BuildMode::Clean, None).unwrap();

        for step in WorkflowStep::steps_for_mode(BuildMode::Clean) {
            assert!(source.contains(*step), "step: {step:?}");
        }
        assert_eq!(plan.runnable_steps(), &CLEAN_RUNNABLE_STEPS);
    }

    /// Taking a step out of a registration leaves every other step registered, and stops the
    /// runnable plan short of the one taken out.
    #[test]
    fn a_registration_without_a_step_stops_its_plan_before_that_step() {
        let (plan, requirements) =
            prepare_workflow_without(&[WorkflowStep::AddPrevisToArchive], BuildMode::Clean, None)
                .unwrap()
                .into_parts();

        assert_eq!(plan.runnable_steps(), &CLEAN_RUNNABLE_STEPS[..7]);
        assert_eq!(plan.skipped_unrunnable_count(), 1);
        // Step 3 still needs the archive tool.
        assert!(requirements.needs_archive());
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
        assert!(requirements.needs_fo4edit());
        assert!(requirements.needs_archive());

        // Production registers every step, so Step 8 is taken out to have one that is not.
        let unregistered_requirements = source
            .without(&[WorkflowStep::AddPrevisToArchive])
            .toolchain_requirements_for_steps(&[WorkflowStep::AddPrevisToArchive]);
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

        /// Assemble the ports for this fixture and hand them to `use_ports`, with a desktop that
        /// has no windows at all.
        ///
        /// For the Creation Kit steps, which never look at a window; see
        /// [`Self::with_desktop_ports`].
        fn with_ports<T>(
            &self,
            process: &dyn ProcessRunner,
            use_ports: impl FnOnce(&OperationPorts<'_>) -> T,
        ) -> T {
            self.with_desktop_ports(process, &RecordingDesktopWindows::new(), use_ports)
        }

        /// Assemble the ports for this fixture, with `desktop` as FO4Edit's windows, and hand
        /// them to `use_ports`.
        ///
        /// Binds each tool episode the run prepared and leaves the other absent, as
        /// `WorkflowRun::execute` does, so a resume at Step 2 runs with FO4Edit alone.
        ///
        /// A closure rather than a returned bundle: each episode borrows the ports it was bound
        /// to, and `OperationPorts` borrows the episodes in turn, so the whole chain has to live
        /// inside one stack frame. Every test that needs ports goes through here, which is what
        /// keeps the assembly spelled out once.
        fn with_desktop_ports<T>(
            &self,
            process: &dyn ProcessRunner,
            desktop: &dyn DesktopWindows,
            use_ports: impl FnOnce(&OperationPorts<'_>) -> T,
        ) -> T {
            let ck = self.run.creation_kit().map(|paths| {
                paths.bind(CkPorts {
                    process,
                    wait: &self.wait,
                    clock: &self.clock,
                    files: &self.files,
                })
            });
            let fo4edit = self.run.fo4edit().map(|paths| {
                paths.bind(Fo4EditPorts {
                    process,
                    wait: &self.wait,
                    desktop,
                    files: &self.files,
                })
            });
            let warnings = BuildWarnings::new(self.run.log_path().to_path_buf(), &self.files);
            let archive = self.run.archive().map(|paths| {
                paths.bind(ArchivePorts {
                    process,
                    wait: &self.wait,
                    files: &self.files,
                    warnings: &warnings,
                })
            });

            use_ports(&OperationPorts {
                ck: ck.as_ref(),
                fo4edit: fo4edit.as_ref(),
                archive: archive.as_ref(),
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

        /// Drive Step 2 over the real FO4Edit episode, with `desktop` as FO4Edit's windows,
        /// and return how it ended with every Build Warning it raised.
        fn run_step_two_collecting_warnings(
            &self,
            process: &dyn ProcessRunner,
            desktop: &dyn DesktopWindows,
        ) -> (Result<()>, Vec<BuildWarning>) {
            self.run_collecting_warnings_on(merge_combined_objects::run, process, desktop)
        }

        /// Drive Step 3 over the real Archive episode, with `process` standing in for the
        /// archive tool, and return how it ended with every Build Warning it raised.
        fn run_step_three_collecting_warnings(
            &self,
            process: &dyn ProcessRunner,
        ) -> (Result<()>, Vec<BuildWarning>) {
            self.run_collecting_warnings(create_ba2_from_precombines::run, process)
        }

        /// Drive Step 8 over the real Archive episode; see
        /// [`Self::run_step_three_collecting_warnings`].
        fn run_step_eight_collecting_warnings(
            &self,
            process: &dyn ProcessRunner,
        ) -> (Result<()>, Vec<BuildWarning>) {
            self.run_collecting_warnings(add_previs_to_archive::run, process)
        }

        /// Drive Step 7 over the real FO4Edit episode; see
        /// [`Self::run_step_two_collecting_warnings`].
        fn run_step_seven_collecting_warnings(
            &self,
            process: &dyn ProcessRunner,
            desktop: &dyn DesktopWindows,
        ) -> (Result<()>, Vec<BuildWarning>) {
            self.run_collecting_warnings_on(merge_previs::run, process, desktop)
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
            self.run_collecting_warnings_on(operation, process, &RecordingDesktopWindows::new())
        }

        /// [`Self::run_collecting_warnings`], with `desktop` as FO4Edit's windows, for the
        /// FO4Edit steps.
        fn run_collecting_warnings_on(
            &self,
            operation: OperationExecution,
            process: &dyn ProcessRunner,
            desktop: &dyn DesktopWindows,
        ) -> (Result<()>, Vec<BuildWarning>) {
            self.with_desktop_ports(process, desktop, |ports| {
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
    /// which Step 6 may offer to clear a non-empty `Data\vis`.
    fn step_six_fixture(mode: BuildMode, non_interactive: bool) -> OperationFixture {
        operation_fixture(mode, non_interactive, Some(WorkflowStep::GeneratePrevis))
    }

    /// Prepare a real Workflow Run resumed at Step 2, which runs through Step 8 and so prepares
    /// every tool, FO4Edit among them.
    fn step_two_fixture(non_interactive: bool) -> OperationFixture {
        operation_fixture(
            BuildMode::Clean,
            non_interactive,
            Some(WorkflowStep::MergePrecombineObjects),
        )
    }

    /// Prepare a real Workflow Run resumed at Step 3, which runs through Step 8 and so prepares
    /// the archive tool beside Creation Kit and FO4Edit.
    fn step_three_fixture(non_interactive: bool) -> OperationFixture {
        operation_fixture(
            BuildMode::Clean,
            non_interactive,
            Some(WorkflowStep::CreateBa2FromPrecombines),
        )
    }

    /// Prepare a real Workflow Run resumed at Step 7, which needs FO4Edit and the archive tool
    /// but no Creation Kit.
    fn step_seven_fixture(non_interactive: bool) -> OperationFixture {
        operation_fixture(
            BuildMode::Clean,
            non_interactive,
            Some(WorkflowStep::MergePrevis),
        )
    }

    /// Prepare a real Workflow Run over a temporary Fallout 4 directory.
    ///
    /// The directory is the only thing that reaches the disk: it holds the `CreationKit.exe` and
    /// the CKPE ini, and an FO4Edit install with its merge scripts, which the toolchain probe
    /// insists on seeing, because probing is not behind a seam. Everything the run then does —
    /// its session log, the artifacts, the Creation Kit log, FO4Edit's log, the archive work
    /// folder — lands in the fixture's own [`InMemoryFileSpace`].
    ///
    /// Archive2 is discovered at [`archive2_exe`] but never written: archive readiness checks
    /// only that discovery found a path, and nothing launches it.
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
            fo4edit: Some(write_fo4edit_install(&dir.path().join("FO4Edit"))),
            archive2: Some(archive2_exe(&fallout4_dir)),
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
        assert_eq!(process.calls(), Vec::<RecordedProcessCall>::new());
        // Nothing ran, so nothing waited either — the whole episode is skipped, not just spawn.
        assert_eq!(fixture.wait.delays(), Vec::<u64>::new());
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
        assert_eq!(process.calls(), Vec::<RecordedProcessCall>::new());
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
        assert_eq!(process.calls(), Vec::<RecordedProcessCall>::new());
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
        assert_eq!(process.calls(), Vec::<RecordedProcessCall>::new());
    }

    /// Production registers every step, so Step 8 is taken out to have one to dispatch.
    #[test]
    fn unregistered_source_dispatch_fails_before_the_ports() {
        let fixture = step_one_fixture(BuildMode::Filtered, true);
        let process = successful_spawn(&fixture);

        let err = fixture
            .with_ports(&process, |ports| {
                production_operation_source()
                    .without(&[WorkflowStep::AddPrevisToArchive])
                    .dispatch(WorkflowStep::AddPrevisToArchive, &fixture.run, ports)
            })
            .unwrap_err();

        assert!(matches!(err, Error::StepNotImplemented(8)));
        assert_eq!(process.calls(), Vec::<RecordedProcessCall>::new());
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

    /// A Clean resume at 4 runs Steps 4 to 8.
    #[test]
    fn a_clean_resume_at_step_four_runs_steps_four_to_eight() {
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
                WorkflowStep::MergePrevis,
                WorkflowStep::AddPrevisToArchive,
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

        assert_eq!(
            plan.runnable_steps(),
            &[
                WorkflowStep::GeneratePrevis,
                WorkflowStep::MergePrevis,
                WorkflowStep::AddPrevisToArchive,
            ]
        );
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
        assert_eq!(process.calls(), Vec::<RecordedProcessCall>::new());
        assert_eq!(fixture.wait.delays(), Vec::<u64>::new());
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

    /// With every step registered, a fresh clean build runs Steps 1 to 8.
    #[test]
    fn a_fresh_clean_build_runs_every_step_through_step_eight() {
        for mode in [BuildMode::Clean, BuildMode::Xbox] {
            let plan = production_workflow_plan(mode, None).unwrap();

            assert_eq!(plan.planned_steps().len(), 8, "mode: {mode:?}");
            assert_eq!(
                plan.runnable_steps(),
                &CLEAN_RUNNABLE_STEPS,
                "mode: {mode:?}"
            );
        }
    }

    /// A Filtered build has no Steps 4 and 5 (`:CompPSG` forwards it to `:PreVis`, 296), so a
    /// fresh one archives its precombines, goes straight on to previs, and adds it to the archive.
    #[test]
    fn a_fresh_filtered_build_archives_its_precombines_before_previs() {
        let plan = production_workflow_plan(BuildMode::Filtered, None).unwrap();

        assert_eq!(
            plan.runnable_steps(),
            &[
                WorkflowStep::GeneratePrecombines,
                WorkflowStep::MergePrecombineObjects,
                WorkflowStep::CreateBa2FromPrecombines,
                WorkflowStep::GeneratePrevis,
                WorkflowStep::MergePrevis,
                WorkflowStep::AddPrevisToArchive,
            ]
        );
    }

    /// A resume at 3 plans Steps 3 to 8 in a Clean build, so it needs Creation Kit and FO4Edit
    /// for the later steps as well as the archive tool for Steps 3 and 8.
    #[test]
    fn a_clean_resume_at_step_three_runs_steps_three_to_eight() {
        let (plan, requirements) = prepare_production_workflow(
            BuildMode::Clean,
            Some(WorkflowStep::CreateBa2FromPrecombines),
        )
        .unwrap()
        .into_parts();

        assert_eq!(plan.runnable_steps(), &CLEAN_RUNNABLE_STEPS[2..]);
        assert!(requirements.needs_archive());
        assert!(requirements.needs_creation_kit());
        assert!(requirements.needs_fo4edit());
    }

    /// A Clean or Xbox resume at 5 runs Steps 5 to 8.
    #[test]
    fn a_clean_build_resumed_at_step_five_runs_steps_five_to_eight() {
        for mode in [BuildMode::Clean, BuildMode::Xbox] {
            let plan = production_workflow_plan(mode, Some(WorkflowStep::BuildCdx)).unwrap();

            assert_eq!(
                plan.runnable_steps(),
                &[
                    WorkflowStep::BuildCdx,
                    WorkflowStep::GeneratePrevis,
                    WorkflowStep::MergePrevis,
                    WorkflowStep::AddPrevisToArchive,
                ],
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

        assert_eq!(
            plan.runnable_steps(),
            &[
                WorkflowStep::GeneratePrevis,
                WorkflowStep::MergePrevis,
                WorkflowStep::AddPrevisToArchive,
            ]
        );
    }

    /// In every Build Mode resumes at 6, 7 and 8 run through Step 8. A resume at 7 needs
    /// FO4Edit and the archive tool but no Creation Kit, and a resume at 8 needs the archive
    /// tool alone.
    #[test]
    fn resumes_at_steps_six_seven_and_eight_run_through_step_eight() {
        for mode in [BuildMode::Clean, BuildMode::Filtered, BuildMode::Xbox] {
            let (plan, _) = prepare_production_workflow(mode, Some(WorkflowStep::GeneratePrevis))
                .unwrap()
                .into_parts();
            assert_eq!(
                plan.runnable_steps(),
                &[
                    WorkflowStep::GeneratePrevis,
                    WorkflowStep::MergePrevis,
                    WorkflowStep::AddPrevisToArchive,
                ],
                "mode: {mode:?}"
            );

            let (plan, requirements) =
                prepare_production_workflow(mode, Some(WorkflowStep::MergePrevis))
                    .unwrap()
                    .into_parts();
            assert_eq!(
                plan.runnable_steps(),
                &[WorkflowStep::MergePrevis, WorkflowStep::AddPrevisToArchive],
                "mode: {mode:?}"
            );
            assert!(requirements.needs_fo4edit(), "mode: {mode:?}");
            assert!(!requirements.needs_creation_kit(), "mode: {mode:?}");
            assert!(requirements.needs_archive(), "mode: {mode:?}");

            let (plan, requirements) =
                prepare_production_workflow(mode, Some(WorkflowStep::AddPrevisToArchive))
                    .unwrap()
                    .into_parts();
            assert_eq!(
                plan.runnable_steps(),
                &[WorkflowStep::AddPrevisToArchive],
                "mode: {mode:?}"
            );
            assert!(requirements.needs_archive(), "mode: {mode:?}");
            assert!(!requirements.needs_fo4edit(), "mode: {mode:?}");
            assert!(!requirements.needs_creation_kit(), "mode: {mode:?}");
        }
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

    /// Each tool episode is reached through an accessor that names the missing preparation,
    /// so an operation never has to unwrap an absent one itself.
    #[test]
    fn an_absent_tool_episode_is_a_preparation_error() {
        let fixture = step_one_fixture(BuildMode::Clean, true);
        let warnings = BuildWarnings::new(fixture.run.log_path().to_path_buf(), &fixture.files);
        let ports = OperationPorts {
            ck: None,
            fo4edit: None,
            archive: None,
            prompts: &fixture.prompts,
            files: &fixture.files,
            warnings: &warnings,
        };

        assert!(matches!(ports.ck(), Err(Error::CreationKitNotPrepared)));
        assert!(matches!(ports.fo4edit(), Err(Error::Fo4EditNotPrepared)));
        assert!(matches!(ports.archive(), Err(Error::ArchiveNotPrepared)));
    }

    /// A prepared episode comes back from its accessor as the very episode that was bound.
    #[test]
    fn a_prepared_tool_episode_is_returned_by_its_accessor() {
        let fixture = step_one_fixture(BuildMode::Clean, true);
        let process = RecordingProcessRunner::new();
        let desktop = RecordingDesktopWindows::new();
        let fo4edit_paths = Fo4EditPaths {
            exe: PathBuf::from("FO4Edit.exe"),
            data_dir_override: None,
            session_log: fixture.run.log_path().to_path_buf(),
        };
        let fo4edit = fo4edit_paths.bind(Fo4EditPorts {
            process: &process,
            wait: &fixture.wait,
            desktop: &desktop,
            files: &fixture.files,
        });
        let ck = fixture.run.creation_kit().unwrap().bind(CkPorts {
            process: &process,
            wait: &fixture.wait,
            clock: &fixture.clock,
            files: &fixture.files,
        });
        let warnings = BuildWarnings::new(fixture.run.log_path().to_path_buf(), &fixture.files);
        let ports = OperationPorts {
            ck: Some(&ck),
            fo4edit: Some(&fo4edit),
            archive: None,
            prompts: &fixture.prompts,
            files: &fixture.files,
            warnings: &warnings,
        };

        assert!(std::ptr::eq(ports.ck().unwrap(), &raw const ck));
        assert!(std::ptr::eq(ports.fo4edit().unwrap(), &raw const fo4edit));
    }

    /// Step 1 reaches Creation Kit through `ports.ck()?`: a run that prepared none stops with
    /// the preparation error instead of launching anything.
    #[test]
    fn step_one_without_creation_kit_stops_before_any_launch() {
        let fixture = step_one_fixture(BuildMode::Clean, true);
        let warnings = BuildWarnings::new(fixture.run.log_path().to_path_buf(), &fixture.files);
        let ports = OperationPorts {
            ck: None,
            fo4edit: None,
            archive: None,
            prompts: &fixture.prompts,
            files: &fixture.files,
            warnings: &warnings,
        };

        let error = generate_precombines::run(&fixture.run, &ports).unwrap_err();

        assert!(
            matches!(error, Error::CreationKitNotPrepared),
            "error: {error:?}"
        );
        assert_eq!(fixture.wait.delays(), Vec::<u64>::new());
    }

    /// Seed what Step 1 leaves for Step 2 to merge: a precombined mesh and `CombinedObjects.esp`.
    ///
    /// Seeded rather than recorded as an effect, because they are Step 2's *inputs*: a resume at
    /// 2 finds them already on disk.
    fn add_precombine_outputs(fixture: &OperationFixture) {
        let config = fixture.run.config();
        fixture
            .files
            .add_file(config.precombined_dir().join("cell").join("mesh.nif"));
        fixture
            .files
            .add_file(config.fo4edit_data_dir().join("CombinedObjects.esp"));
    }

    /// FO4Edit's windows for a Step 2 merge whose script leaves `log` as its log once Module
    /// Selection is dismissed, in place of the clean log
    /// [`record_successful_combined_objects_merge`] writes.
    fn merge_windows_logging<'a>(
        fixture: &'a OperationFixture,
        exit: &ExitFlag,
        log: &'a str,
    ) -> RecordingDesktopWindows<'a> {
        let files = &fixture.files;
        behaving_fo4edit_windows(exit, move || {
            files.add_file_with_contents(logging::unattended_log_path(files), log);
        })
    }

    #[test]
    fn merge_combined_objects_is_registered_and_requires_fo4edit_alone() {
        let source = production_operation_source();
        let requirements =
            source.toolchain_requirements_for_steps(&[WorkflowStep::MergePrecombineObjects]);

        assert!(source.contains(WorkflowStep::MergePrecombineObjects));
        assert!(requirements.needs_fo4edit());
        assert!(!requirements.needs_creation_kit());
        assert!(!requirements.needs_archive());
    }

    /// A resume at 2 merges `CombinedObjects.esp` through the whole FO4Edit episode, and nothing
    /// is warned about. That Step 2 needs FO4Edit alone is pinned by its registration test, since
    /// a resume at 2 now runs on through Step 8 and so prepares every tool.
    #[test]
    fn step_two_merges_combined_objects_through_the_whole_fo4edit_episode() {
        let fixture = step_two_fixture(true);
        add_precombine_outputs(&fixture);
        let exit = ExitFlag::new();
        let process = fo4edit_exiting_when(&exit);
        let files = &fixture.files;
        let desktop =
            behaving_fo4edit_windows(&exit, || record_successful_combined_objects_merge(files));

        let (result, warnings) = fixture.run_step_two_collecting_warnings(&process, &desktop);

        result.unwrap();
        assert!(warnings.is_empty(), "warnings: {warnings:?}");
        // One launch of the run's own FO4Edit, left running, for the Step 2 script and the
        // run's own plugin; the rest of the argv is pinned in `tools::fo4edit`.
        let calls = process.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].kind, ProcessCallKind::Spawn);
        assert_eq!(calls[0].exe, fixture.run.fo4edit().unwrap().exe);
        for expected in [
            "-Script:Batch_FO4MergeCombinedObjectsAndCheck.pas",
            "-Mod:MyMod.esp",
        ] {
            assert!(
                calls[0].args.contains(&std::ffi::OsString::from(expected)),
                "args: {:?}",
                calls[0].args
            );
        }
        assert_eq!(fixture.wait.delays(), ONE_POLL_MERGE_DELAYS);
        assert_eq!(
            desktop.actions(),
            vec![
                DesktopAction::ClickButton {
                    window: FO4EDIT_MODULE_SELECTION,
                    caption: "OK".to_owned(),
                },
                DesktopAction::RequestClose {
                    window: FO4EDIT_MAIN_FORM,
                },
            ]
        );
        let session = fixture.files.read_lossy(fixture.run.log_path()).unwrap();
        assert!(
            session.contains(
                "Running xEdit script Batch_FO4MergeCombinedObjectsAndCheck.pas against MyMod.esp\n"
            ),
            "session log: {session}"
        );
        assert!(
            session.ends_with(COMPLETED_COMBINED_OBJECTS_MERGE_LOG),
            "session log: {session}"
        );
    }

    /// `Error: ` anywhere in the log, in any case, completes Step 2 with exactly one warning in
    /// the batch's words (284–285). The marker is spelled out by hand in each case.
    #[test]
    fn a_merge_log_with_errors_completes_step_two_with_one_warning() {
        for log in [
            "Merging CombinedObjects.esp\nError: could not copy REFR [0001F00D]\nError: and another\nCompleted: 2 errors.\n",
            "Merging CombinedObjects.esp\n[00:03] error: could not copy REFR [0001F00D]\nCompleted: 1 errors.\n",
        ] {
            let fixture = step_two_fixture(true);
            add_precombine_outputs(&fixture);
            let exit = ExitFlag::new();
            let process = fo4edit_exiting_when(&exit);
            let desktop = merge_windows_logging(&fixture, &exit, log);

            let (result, warnings) = fixture.run_step_two_collecting_warnings(&process, &desktop);

            result.unwrap();
            assert_eq!(
                warnings,
                vec![BuildWarning::MergePrecombinesHadErrors],
                "log: {log}"
            );
            let session = fixture.files.read_lossy(fixture.run.log_path()).unwrap();
            assert!(
                session.ends_with(&format!("{log}WARNING - Merge Precombines had errors\n")),
                "session log: {session}"
            );
        }
    }

    /// No `.nif` under `meshes\precombined` stops Step 2 with the batch's own words (281),
    /// before FO4Edit is launched or any delay runs.
    #[test]
    fn step_two_stops_without_a_precombined_mesh_before_fo4edit() {
        let fixture = step_two_fixture(true);
        fixture.files.add_file(
            fixture
                .run
                .config()
                .fo4edit_data_dir()
                .join("CombinedObjects.esp"),
        );
        let exit = ExitFlag::new();
        let process = fo4edit_exiting_when(&exit);
        let files = &fixture.files;
        let desktop =
            behaving_fo4edit_windows(&exit, || record_successful_combined_objects_merge(files));

        let (result, warnings) = fixture.run_step_two_collecting_warnings(&process, &desktop);

        let err = result.unwrap_err();
        assert!(matches!(err, Error::NoPrecombinedMeshesFound), "{err:?}");
        assert_eq!(err.to_string(), "No Precombined meshes found");
        assert!(warnings.is_empty(), "warnings: {warnings:?}");
        assert_eq!(process.calls(), Vec::<RecordedProcessCall>::new());
        assert_eq!(fixture.wait.delays(), Vec::<u64>::new());
        assert_eq!(desktop.actions(), vec![]);
    }

    /// A missing `CombinedObjects.esp` stops Step 2 before launch — a divergence: the batch
    /// learns of it only through FO4Edit's "missing modules" fatal, after about 50s of delays.
    #[test]
    fn step_two_stops_without_combined_objects_before_fo4edit() {
        let fixture = step_two_fixture(true);
        fixture.files.add_file(
            fixture
                .run
                .config()
                .precombined_dir()
                .join("cell")
                .join("mesh.nif"),
        );
        let exit = ExitFlag::new();
        let process = fo4edit_exiting_when(&exit);
        let files = &fixture.files;
        let desktop =
            behaving_fo4edit_windows(&exit, || record_successful_combined_objects_merge(files));

        let (result, _) = fixture.run_step_two_collecting_warnings(&process, &desktop);

        let err = result.unwrap_err();
        assert!(matches!(err, Error::MissingCombinedObjects), "{err:?}");
        assert_eq!(process.calls(), Vec::<RecordedProcessCall>::new());
        assert_eq!(fixture.wait.delays(), Vec::<u64>::new());
        // Nothing was attempted, so the session log names no script run.
        let session = fixture.files.read_lossy(fixture.run.log_path()).unwrap();
        assert!(
            !session.contains("Running xEdit script"),
            "session log: {session}"
        );
    }

    /// Regression pin for the `:RunScript` fall-through divergence (docs/episodes.md §
    /// *FO4Edit*): the batch's non-interactive `goto failed` inside a `Call` carried on into
    /// Step 3 with an unmerged plugin. Both shared fatals stop Step 2 whether or not the run is
    /// interactive, and neither leaves a Step 2 warning behind, though both logs hold `Error: `.
    #[test]
    fn the_shared_fo4edit_fatals_stop_step_two_whether_or_not_it_is_interactive() {
        let missing_modules = "Error: Missing [MyMod.esp] or [CombinedObjects.esp] modules\n\
                               Completed: 1 errors.\n";
        let never_completed = "Merging CombinedObjects.esp\nError: access violation\n";

        for non_interactive in [true, false] {
            for log in [missing_modules, never_completed] {
                let fixture = step_two_fixture(non_interactive);
                assert_eq!(fixture.run.config().non_interactive, non_interactive);
                add_precombine_outputs(&fixture);
                let exit = ExitFlag::new();
                let process = fo4edit_exiting_when(&exit);
                let desktop = merge_windows_logging(&fixture, &exit, log);

                let (result, warnings) =
                    fixture.run_step_two_collecting_warnings(&process, &desktop);

                let err = result.unwrap_err();
                let case = format!("non-interactive: {non_interactive}, log: {log}");
                if log == missing_modules {
                    assert!(
                        matches!(err, Error::Fo4EditScriptMissingModules { .. }),
                        "{case}, error: {err:?}"
                    );
                } else {
                    assert!(
                        matches!(err, Error::Fo4EditScriptFailed { .. }),
                        "{case}, error: {err:?}"
                    );
                }
                assert!(warnings.is_empty(), "{case}, warnings: {warnings:?}");
            }
        }
    }

    /// Step 2 reaches FO4Edit through `ports.fo4edit()?`: a run that prepared none stops with
    /// the preparation error before the workspace is looked at.
    #[test]
    fn step_two_without_fo4edit_stops_before_any_launch() {
        let fixture = step_two_fixture(true);
        let warnings = BuildWarnings::new(fixture.run.log_path().to_path_buf(), &fixture.files);
        let ports = OperationPorts {
            ck: None,
            fo4edit: None,
            archive: None,
            prompts: &fixture.prompts,
            files: &fixture.files,
            warnings: &warnings,
        };

        // No mesh either: the preparation error must win over the workspace's own stop.
        let error = merge_combined_objects::run(&fixture.run, &ports).unwrap_err();

        assert!(
            matches!(error, Error::Fo4EditNotPrepared),
            "error: {error:?}"
        );
        assert_eq!(fixture.wait.delays(), Vec::<u64>::new());
    }

    /// A loose precombined mesh Step 1 left in `Data`, nested as Creation Kit writes them.
    fn loose_precombined_mesh(fixture: &OperationFixture) -> PathBuf {
        fixture
            .run
            .config()
            .precombined_dir()
            .join("cell")
            .join("mesh.nif")
    }

    /// `Data\<base name> - Main.ba2`, the Plugin Archive Step 3 packs into.
    fn plugin_archive(fixture: &OperationFixture) -> PathBuf {
        fixture.run.config().plugin_archive_path()
    }

    /// An archive tool whose single pack writes the built archive where its `-c=` says.
    fn successful_archive2_pack(fixture: &OperationFixture) -> RecordingProcessRunner<'_> {
        RecordingProcessRunner::new()
            .scripting_call(0, archive2_pack_writing_archive(&fixture.files))
    }

    #[test]
    fn create_ba2_from_precombines_is_registered_and_requires_the_archive_tool_alone() {
        let source = production_operation_source();
        let requirements =
            source.toolchain_requirements_for_steps(&[WorkflowStep::CreateBa2FromPrecombines]);

        assert!(source.contains(WorkflowStep::CreateBa2FromPrecombines));
        assert!(requirements.needs_archive());
        assert!(!requirements.needs_creation_kit());
        assert!(!requirements.needs_fo4edit());
    }

    /// Loose meshes go into the Plugin Archive through the whole Archive episode, and the loose
    /// copies are gone once the archive holds them.
    #[test]
    fn step_three_packs_the_loose_precombines_through_the_whole_archive_episode() {
        let fixture = step_three_fixture(true);
        fixture.files.add_file(loose_precombined_mesh(&fixture));
        let process = successful_archive2_pack(&fixture);

        let (result, warnings) = fixture.run_step_three_collecting_warnings(&process);

        result.unwrap();
        assert!(warnings.is_empty(), "warnings: {warnings:?}");
        // One capturing run of the run's own archive tool, in `Data`, building the run's own
        // archive; the rest of the argv is pinned in `tools::archive`.
        let calls = process.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].exe, fixture.run.archive().unwrap().exe);
        assert_eq!(calls[0].kind, ProcessCallKind::RunCapturing);
        assert_eq!(calls[0].cwd, fixture.run.config().fo4edit_data_dir());
        assert!(
            calls[0]
                .args
                .iter()
                .any(|arg| arg.to_string_lossy().ends_with("MyMod - Main.ba2")),
            "args: {:?}",
            calls[0].args
        );
        // Archive2 has no MO2 wait around a pack.
        assert_eq!(fixture.wait.delays(), Vec::<u64>::new());
        assert_eq!(
            fixture.files.read_lossy(&plugin_archive(&fixture)).unwrap(),
            PACKED_ARCHIVE
        );
        assert!(
            !fixture
                .files
                .is_dir(&fixture.run.config().precombined_dir())
        );
        let session = fixture.files.read_lossy(fixture.run.log_path()).unwrap();
        assert!(
            session.contains("Creating Archive MyMod - Main.ba2 of meshes\\precombined:\n"),
            "session log: {session}"
        );
    }

    /// Batch parity on a resume at 3: loose meshes are packed even over an archive an earlier
    /// attempt built, which the new one replaces.
    #[test]
    fn step_three_repacks_over_an_existing_archive() {
        let fixture = step_three_fixture(true);
        fixture.files.add_file(loose_precombined_mesh(&fixture));
        fixture
            .files
            .add_file_with_contents(plugin_archive(&fixture), "old archive");
        let process = successful_archive2_pack(&fixture);

        let (result, warnings) = fixture.run_step_three_collecting_warnings(&process);

        result.unwrap();
        assert!(warnings.is_empty(), "warnings: {warnings:?}");
        assert_eq!(process.calls().len(), 1);
        assert_eq!(
            fixture.files.read_lossy(&plugin_archive(&fixture)).unwrap(),
            PACKED_ARCHIVE
        );
        assert!(
            !fixture
                .files
                .is_dir(&fixture.run.config().precombined_dir())
        );
    }

    /// No loose meshes, but an archive that already holds them (Step 3 ran on an earlier
    /// attempt): nothing to do, so the tool never runs and nothing is warned about. The skip is
    /// a console line, not a Build Warning, so the session log is untouched.
    #[test]
    fn step_three_completes_with_nothing_to_do_when_the_archive_already_holds_the_precombines() {
        let fixture = step_three_fixture(true);
        fixture
            .files
            .add_file_with_contents(plugin_archive(&fixture), "old archive");
        let session_before = fixture.files.read_lossy(fixture.run.log_path()).unwrap();
        let process = successful_archive2_pack(&fixture);

        let (result, warnings) = fixture.run_step_three_collecting_warnings(&process);

        result.unwrap();
        assert!(warnings.is_empty(), "warnings: {warnings:?}");
        assert_eq!(process.calls(), Vec::<RecordedProcessCall>::new());
        assert_eq!(fixture.wait.delays(), Vec::<u64>::new());
        assert_eq!(
            fixture.files.read_lossy(&plugin_archive(&fixture)).unwrap(),
            "old archive"
        );
        assert_eq!(
            fixture.files.read_lossy(fixture.run.log_path()).unwrap(),
            session_before
        );
    }

    /// Neither loose meshes nor an archive: a divergence from the batch's silent skip (289),
    /// because Step 8 would then have no precombines to add previs to. Only a `.nif` counts as
    /// a mesh, so a stray file under `meshes\precombined` does not satisfy the check.
    #[test]
    fn step_three_stops_when_there_are_neither_loose_precombines_nor_an_archive() {
        let fixture = step_three_fixture(true);
        fixture
            .files
            .add_file(fixture.run.config().precombined_dir().join("notes.txt"));
        let process = successful_archive2_pack(&fixture);

        let (result, warnings) = fixture.run_step_three_collecting_warnings(&process);

        let err = result.unwrap_err();
        assert!(
            matches!(&err, Error::NoPrecombinesToArchive { name } if name == "MyMod - Main.ba2"),
            "error: {err:?}"
        );
        assert_eq!(
            err.to_string(),
            "No Precombined meshes found to archive, and MyMod - Main.ba2 does not exist"
        );
        assert!(warnings.is_empty(), "warnings: {warnings:?}");
        assert_eq!(process.calls(), Vec::<RecordedProcessCall>::new());
        assert!(!fixture.files.is_file(&plugin_archive(&fixture)));
    }

    /// Regression pin for the `:Archive` fall-through divergence (docs/episodes.md § *Archive*):
    /// the batch enters `:Archive` by `Call`, so a non-interactive run carried on past its
    /// fatals. A failed pack and a pack that built nothing both stop Step 3 whether or not the
    /// run is interactive, and leave the loose meshes where they were.
    #[test]
    fn a_failed_archive_pack_stops_step_three_whether_or_not_it_is_interactive() {
        for non_interactive in [true, false] {
            for exit_code in [1, 0] {
                let fixture = step_three_fixture(non_interactive);
                assert_eq!(fixture.run.config().non_interactive, non_interactive);
                fixture.files.add_file(loose_precombined_mesh(&fixture));
                // A pack that exits `exit_code` and writes nothing at all.
                let process = RecordingProcessRunner::new().returning_exit_code(exit_code);

                let (result, warnings) = fixture.run_step_three_collecting_warnings(&process);

                let err = result.unwrap_err();
                let case = format!("non-interactive: {non_interactive}, exit: {exit_code}");
                if exit_code == 0 {
                    assert!(
                        matches!(err, Error::NoPluginArchiveCreated),
                        "{case}, error: {err:?}"
                    );
                } else {
                    assert!(
                        matches!(err, Error::Archive2Failed { code: Some(1) }),
                        "{case}, error: {err:?}"
                    );
                }
                assert!(warnings.is_empty(), "{case}, warnings: {warnings:?}");
                assert_eq!(process.calls().len(), 1, "{case}");
                assert!(
                    fixture.files.is_file(&loose_precombined_mesh(&fixture)),
                    "{case}"
                );
                assert!(!fixture.files.is_file(&plugin_archive(&fixture)), "{case}");
            }
        }
    }

    /// Step 3 needs the archive tool and nothing else: with only the Archive episode bound, it
    /// packs and completes.
    #[test]
    fn step_three_runs_with_the_archive_tool_alone() {
        let fixture = step_three_fixture(true);
        fixture.files.add_file(loose_precombined_mesh(&fixture));
        let process = successful_archive2_pack(&fixture);
        let warnings = BuildWarnings::new(fixture.run.log_path().to_path_buf(), &fixture.files);
        let archive = fixture.run.archive().unwrap().bind(ArchivePorts {
            process: &process,
            wait: &fixture.wait,
            files: &fixture.files,
            warnings: &warnings,
        });
        let ports = OperationPorts {
            ck: None,
            fo4edit: None,
            archive: Some(&archive),
            prompts: &fixture.prompts,
            files: &fixture.files,
            warnings: &warnings,
        };

        create_ba2_from_precombines::run(&fixture.run, &ports).unwrap();

        assert_eq!(process.calls().len(), 1);
        assert_eq!(
            fixture.files.read_lossy(&plugin_archive(&fixture)).unwrap(),
            PACKED_ARCHIVE
        );
    }

    /// Step 3 reaches the archive tool through `ports.archive()?`: a run that prepared none
    /// stops with the preparation error before the workspace is looked at.
    #[test]
    fn step_three_without_the_archive_tool_stops_before_anything_is_looked_at() {
        let fixture = step_three_fixture(true);
        let warnings = BuildWarnings::new(fixture.run.log_path().to_path_buf(), &fixture.files);
        let ports = OperationPorts {
            ck: None,
            fo4edit: None,
            archive: None,
            prompts: &fixture.prompts,
            files: &fixture.files,
            warnings: &warnings,
        };

        // No meshes and no archive either: the preparation error must win over Step 3's own stop.
        let error = create_ba2_from_precombines::run(&fixture.run, &ports).unwrap_err();

        assert!(
            matches!(error, Error::ArchiveNotPrepared),
            "error: {error:?}"
        );
    }

    /// A `.uvd` beneath `Data\vis`, for Step 7's first entry check.
    ///
    /// Nested, as Creation Kit writes one per cluster cell, so the check is shown to look below
    /// `vis` itself (`dir /s`, 323).
    fn vis_uvd(fixture: &OperationFixture) -> PathBuf {
        fixture
            .run
            .config()
            .vis_dir()
            .join("cell")
            .join("cluster.uvd")
    }

    /// Seed what Step 6 leaves for Step 7 to merge: a `.uvd` and `Previs.esp`.
    ///
    /// Seeded rather than recorded as an effect, because they are Step 7's *inputs*: a resume at
    /// 7 finds them already on disk.
    fn add_previs_outputs(fixture: &OperationFixture) {
        fixture.files.add_file(vis_uvd(fixture));
        fixture.files.add_file(previs_plugin(fixture));
    }

    #[test]
    fn merge_previs_is_registered_and_requires_fo4edit_alone() {
        let source = production_operation_source();
        let requirements = source.toolchain_requirements_for_steps(&[WorkflowStep::MergePrevis]);

        assert!(source.contains(WorkflowStep::MergePrevis));
        assert!(requirements.needs_fo4edit());
        assert!(!requirements.needs_creation_kit());
        assert!(!requirements.needs_archive());
    }

    /// A resume at 7 merges `Previs.esp` through the whole FO4Edit episode, with no Creation Kit
    /// prepared even though one is installed, and nothing to warn about.
    #[test]
    fn step_seven_merges_previs_through_the_whole_fo4edit_episode() {
        let fixture = step_seven_fixture(true);
        assert!(fixture.run.creation_kit().is_none());
        add_previs_outputs(&fixture);
        let exit = ExitFlag::new();
        let process = fo4edit_exiting_when(&exit);
        let files = &fixture.files;
        let desktop = behaving_fo4edit_windows(&exit, || record_successful_previs_merge(files));

        let (result, warnings) = fixture.run_step_seven_collecting_warnings(&process, &desktop);

        result.unwrap();
        assert!(warnings.is_empty(), "warnings: {warnings:?}");
        // One launch of the run's own FO4Edit, left running, for the Step 7 script and the
        // run's own plugin; the rest of the argv is pinned in `tools::fo4edit`.
        let calls = process.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].kind, ProcessCallKind::Spawn);
        assert_eq!(calls[0].exe, fixture.run.fo4edit().unwrap().exe);
        for expected in [
            "-Script:Batch_FO4MergePrevisandCleanRefr.pas",
            "-Mod:MyMod.esp",
        ] {
            assert!(
                calls[0].args.contains(&std::ffi::OsString::from(expected)),
                "args: {:?}",
                calls[0].args
            );
        }
        assert_eq!(fixture.wait.delays(), ONE_POLL_MERGE_DELAYS);
        assert_eq!(
            desktop.actions(),
            vec![
                DesktopAction::ClickButton {
                    window: FO4EDIT_MODULE_SELECTION,
                    caption: "OK".to_owned(),
                },
                DesktopAction::RequestClose {
                    window: FO4EDIT_MAIN_FORM,
                },
            ]
        );
        let session = fixture.files.read_lossy(fixture.run.log_path()).unwrap();
        assert!(
            session.contains(
                "Running xEdit script Batch_FO4MergePrevisandCleanRefr.pas against MyMod.esp\n"
            ),
            "session log: {session}"
        );
        assert!(
            session.ends_with(COMPLETED_PREVIS_MERGE_LOG),
            "session log: {session}"
        );
    }

    /// A log that completed but lacks `Completed: No Errors.` finishes Step 7 with exactly one
    /// warning in the batch's words (327–328). The logs are spelled out by hand, and carry the
    /// `Completed: ` the shared fatal needs, so only Step 7's own criterion is in play.
    #[test]
    fn a_merge_log_lacking_no_errors_completes_step_seven_with_one_warning() {
        for log in [
            "Merging Previs.esp\nError: could not copy REFR [0001F00D]\nCompleted: 1 errors.\n",
            // No `Error: ` at all: Step 7 does not look for errors, only for the clean marker.
            "Merging Previs.esp\nCompleted: No Errors\n",
        ] {
            let fixture = step_seven_fixture(true);
            add_previs_outputs(&fixture);
            let exit = ExitFlag::new();
            let process = fo4edit_exiting_when(&exit);
            let desktop = merge_windows_logging(&fixture, &exit, log);

            let (result, warnings) = fixture.run_step_seven_collecting_warnings(&process, &desktop);

            result.unwrap();
            assert_eq!(
                warnings,
                vec![BuildWarning::MergePrevisHadErrors],
                "log: {log}"
            );
            let session = fixture.files.read_lossy(fixture.run.log_path()).unwrap();
            assert!(
                session.ends_with(&format!("{log}WARNING - Merge Previs had errors\n")),
                "session log: {session}"
            );
        }
    }

    /// `Findstr /I` (327): `Completed: No Errors.` in another letter case still counts as a
    /// clean run, so no warning is raised.
    #[test]
    fn the_no_errors_marker_in_another_case_raises_no_step_seven_warning() {
        for log in [
            "Merging Previs.esp\n[00:42] completed: no errors.\n",
            "Merging Previs.esp\nCOMPLETED: NO ERRORS.\n",
        ] {
            let fixture = step_seven_fixture(true);
            add_previs_outputs(&fixture);
            let exit = ExitFlag::new();
            let process = fo4edit_exiting_when(&exit);
            let desktop = merge_windows_logging(&fixture, &exit, log);

            let (result, warnings) = fixture.run_step_seven_collecting_warnings(&process, &desktop);

            result.unwrap();
            assert!(warnings.is_empty(), "log: {log}, warnings: {warnings:?}");
        }
    }

    /// No `.uvd` under `Data\vis` stops Step 7 with the batch's own words (323), before FO4Edit
    /// is launched or any delay runs — and before `Previs.esp` is looked at, so a run missing
    /// both is told about the `.uvd` files first, as the batch tells it.
    #[test]
    fn step_seven_stops_without_visibility_files_before_fo4edit() {
        for with_previs_plugin in [true, false] {
            let fixture = step_seven_fixture(true);
            if with_previs_plugin {
                fixture.files.add_file(previs_plugin(&fixture));
            }
            // A non-`.uvd` file in `vis` is not previs, so it does not satisfy the check.
            fixture
                .files
                .add_file(fixture.run.config().vis_dir().join("notes.txt"));
            let exit = ExitFlag::new();
            let process = fo4edit_exiting_when(&exit);
            let files = &fixture.files;
            let desktop = behaving_fo4edit_windows(&exit, || record_successful_previs_merge(files));

            let (result, warnings) = fixture.run_step_seven_collecting_warnings(&process, &desktop);

            let err = result.unwrap_err();
            assert!(
                matches!(err, Error::NoVisibilityFiles),
                "with Previs.esp: {with_previs_plugin}, error: {err:?}"
            );
            assert_eq!(err.to_string(), "No Visibility files Generated");
            assert!(warnings.is_empty(), "warnings: {warnings:?}");
            assert_eq!(process.calls(), Vec::<RecordedProcessCall>::new());
            assert_eq!(fixture.wait.delays(), Vec::<u64>::new());
            assert_eq!(desktop.actions(), vec![]);
        }
    }

    /// No `Data\Previs.esp` stops Step 7 with the batch's own words (324), before FO4Edit is
    /// launched or any delay runs.
    #[test]
    fn step_seven_stops_without_a_previs_plugin_before_fo4edit() {
        let fixture = step_seven_fixture(true);
        fixture.files.add_file(vis_uvd(&fixture));
        let exit = ExitFlag::new();
        let process = fo4edit_exiting_when(&exit);
        let files = &fixture.files;
        let desktop = behaving_fo4edit_windows(&exit, || record_successful_previs_merge(files));

        let (result, warnings) = fixture.run_step_seven_collecting_warnings(&process, &desktop);

        let err = result.unwrap_err();
        assert!(matches!(err, Error::NoPrevisPlugin), "{err:?}");
        assert_eq!(err.to_string(), "No Previs.esp Generated");
        assert!(warnings.is_empty(), "warnings: {warnings:?}");
        assert_eq!(process.calls(), Vec::<RecordedProcessCall>::new());
        assert_eq!(fixture.wait.delays(), Vec::<u64>::new());
        assert_eq!(desktop.actions(), vec![]);
        // Nothing was attempted, so the session log names no script run.
        let session = fixture.files.read_lossy(fixture.run.log_path()).unwrap();
        assert!(
            !session.contains("Running xEdit script"),
            "session log: {session}"
        );
    }

    /// The `:RunScript` fall-through divergence again, for Step 7: the batch's non-interactive
    /// `goto failed` inside a `Call` carried on into Step 8 with an unmerged plugin. Both shared
    /// fatals stop Step 7 whether or not the run is interactive, and neither leaves a Step 7
    /// warning behind, though neither log holds `Completed: No Errors.`.
    #[test]
    fn the_shared_fo4edit_fatals_stop_step_seven_whether_or_not_it_is_interactive() {
        let missing_modules = "Error: Missing [MyMod.esp] or [Previs.esp] modules\n\
                               Completed: 1 errors.\n";
        let never_completed = "Merging Previs.esp\nError: access violation\n";

        for non_interactive in [true, false] {
            for log in [missing_modules, never_completed] {
                let fixture = step_seven_fixture(non_interactive);
                assert_eq!(fixture.run.config().non_interactive, non_interactive);
                add_previs_outputs(&fixture);
                let exit = ExitFlag::new();
                let process = fo4edit_exiting_when(&exit);
                let desktop = merge_windows_logging(&fixture, &exit, log);

                let (result, warnings) =
                    fixture.run_step_seven_collecting_warnings(&process, &desktop);

                let err = result.unwrap_err();
                let case = format!("non-interactive: {non_interactive}, log: {log}");
                if log == missing_modules {
                    assert!(
                        matches!(err, Error::Fo4EditScriptMissingModules { .. }),
                        "{case}, error: {err:?}"
                    );
                } else {
                    assert!(
                        matches!(err, Error::Fo4EditScriptFailed { .. }),
                        "{case}, error: {err:?}"
                    );
                }
                assert!(warnings.is_empty(), "{case}, warnings: {warnings:?}");
            }
        }
    }

    /// Step 7 reaches FO4Edit through `ports.fo4edit()?`: a run that prepared none stops with
    /// the preparation error before the workspace is looked at.
    #[test]
    fn step_seven_without_fo4edit_stops_before_any_launch() {
        let fixture = step_seven_fixture(true);
        let warnings = BuildWarnings::new(fixture.run.log_path().to_path_buf(), &fixture.files);
        let ports = OperationPorts {
            ck: None,
            fo4edit: None,
            archive: None,
            prompts: &fixture.prompts,
            files: &fixture.files,
            warnings: &warnings,
        };

        // No `.uvd` either: the preparation error must win over the workspace's own stop.
        let error = merge_previs::run(&fixture.run, &ports).unwrap_err();

        assert!(
            matches!(error, Error::Fo4EditNotPrepared),
            "error: {error:?}"
        );
        assert_eq!(fixture.wait.delays(), Vec::<u64>::new());
    }

    /// Prepare a real Workflow Run resumed at Step 8, which needs the archive tool alone.
    fn step_eight_fixture(non_interactive: bool) -> OperationFixture {
        operation_fixture(
            BuildMode::Clean,
            non_interactive,
            Some(WorkflowStep::AddPrevisToArchive),
        )
    }

    /// Seed what Steps 3 and 6 leave for Step 8: the Plugin Archive Step 3 packed, and a `.uvd`
    /// under `Data\vis`.
    ///
    /// Seeded rather than recorded as an effect, because they are Step 8's *inputs*: a resume
    /// at 8 finds them already on disk.
    fn add_archive_and_previs(fixture: &OperationFixture) {
        fixture
            .files
            .add_file_with_contents(plugin_archive(fixture), "old archive");
        fixture.files.add_file(vis_uvd(fixture));
    }

    /// An archive tool whose extract writes the archive's precombines into `Data`, and whose
    /// repack then writes the built archive where its `-c=` says.
    fn successful_archive2_rebuild(fixture: &OperationFixture) -> RecordingProcessRunner<'_> {
        RecordingProcessRunner::new()
            .scripting_call(
                0,
                archive2_extract_writing_precombines(
                    &fixture.files,
                    fixture.run.config().fo4edit_data_dir(),
                    false,
                ),
            )
            .scripting_call(1, archive2_pack_writing_archive(&fixture.files))
    }

    #[test]
    fn add_previs_to_archive_is_registered_and_requires_the_archive_tool_alone() {
        let source = production_operation_source();
        let requirements =
            source.toolchain_requirements_for_steps(&[WorkflowStep::AddPrevisToArchive]);

        assert!(source.contains(WorkflowStep::AddPrevisToArchive));
        assert!(requirements.needs_archive());
        assert!(!requirements.needs_creation_kit());
        assert!(!requirements.needs_fo4edit());
    }

    /// A resume at 8 rebuilds the Plugin Archive with the new previs through the whole Archive
    /// episode, with neither Creation Kit nor FO4Edit prepared, and nothing to warn about.
    #[test]
    fn step_eight_adds_previs_through_the_whole_archive_episode() {
        let fixture = step_eight_fixture(true);
        assert!(fixture.run.creation_kit().is_none());
        assert!(fixture.run.fo4edit().is_none());
        add_archive_and_previs(&fixture);
        let process = successful_archive2_rebuild(&fixture);

        let (result, warnings) = fixture.run_step_eight_collecting_warnings(&process);

        result.unwrap();
        assert!(warnings.is_empty(), "warnings: {warnings:?}");
        // The extract, then the repack: two capturing runs of the run's own archive tool, in
        // `Data`, each naming the run's own archive; the rest of the argv is pinned in
        // `tools::archive`.
        let calls = process.calls();
        assert_eq!(calls.len(), 2);
        for call in &calls {
            assert_eq!(call.exe, fixture.run.archive().unwrap().exe);
            assert_eq!(call.kind, ProcessCallKind::RunCapturing);
            assert_eq!(call.cwd, fixture.run.config().fo4edit_data_dir());
            assert!(
                call.args
                    .iter()
                    .any(|arg| arg.to_string_lossy().ends_with("MyMod - Main.ba2")),
                "args: {:?}",
                call.args
            );
        }
        // Archive2's required MO2 wait after the extract (docs/workarounds.md §2).
        assert_eq!(
            fixture.wait.delays(),
            vec![MO2_DELAY_AFTER_ARCHIVE2_EXTRACT_SECS]
        );
        assert_eq!(
            fixture.files.read_lossy(&plugin_archive(&fixture)).unwrap(),
            PACKED_ARCHIVE
        );
        assert!(!fixture.files.is_dir(&fixture.run.config().vis_dir()));
        assert!(
            !fixture
                .files
                .is_dir(&fixture.run.config().precombined_dir())
        );
        let session = fixture.files.read_lossy(fixture.run.log_path()).unwrap();
        assert!(
            session.contains("Extracting Archive MyMod - Main.ba2:\n"),
            "session log: {session}"
        );
        assert!(
            session.contains("Creating Archive MyMod - Main.ba2 of meshes\\precombined,vis:\n"),
            "session log: {session}"
        );
    }

    /// No `.uvd` under `Data\vis` is the normal state after a Step 8 that succeeded, so a resume
    /// at 8 warns and completes (batch 333) rather than failing a finished build. The check comes
    /// before the archive's, as in the batch (333 precedes 424), so it holds even when the
    /// archive is missing too.
    #[test]
    fn step_eight_warns_and_completes_when_there_are_no_visibility_files() {
        for archive_exists in [true, false] {
            let fixture = step_eight_fixture(true);
            if archive_exists {
                fixture
                    .files
                    .add_file_with_contents(plugin_archive(&fixture), "old archive");
            }
            // Only a `.uvd` counts, so a stray file under `vis` does not satisfy the check.
            fixture
                .files
                .add_file(fixture.run.config().vis_dir().join("notes.txt"));
            let process = successful_archive2_rebuild(&fixture);

            let (result, warnings) = fixture.run_step_eight_collecting_warnings(&process);

            let case = format!("archive exists: {archive_exists}");
            result.unwrap();
            assert_eq!(
                warnings,
                [BuildWarning::NoVisibilityFilesToArchive],
                "{case}"
            );
            assert_eq!(process.calls(), Vec::<RecordedProcessCall>::new(), "{case}");
            assert_eq!(fixture.wait.delays(), Vec::<u64>::new(), "{case}");
            assert_eq!(
                fixture.files.is_file(&plugin_archive(&fixture)),
                archive_exists,
                "{case}"
            );
            let session = fixture.files.read_lossy(fixture.run.log_path()).unwrap();
            assert!(
                session.ends_with("WARNING - No Visibility files found to archive\n"),
                "{case}, session log: {session}"
            );
        }
    }

    /// A missing Plugin Archive is a stop, not the batch's `:ArchiveOnly` fallback (424): a
    /// `vis`-only archive drops the precombined meshes previs refers to, which is a broken build.
    #[test]
    fn step_eight_stops_when_the_plugin_archive_is_missing() {
        let fixture = step_eight_fixture(true);
        fixture.files.add_file(vis_uvd(&fixture));
        let process = successful_archive2_rebuild(&fixture);

        let (result, warnings) = fixture.run_step_eight_collecting_warnings(&process);

        let err = result.unwrap_err();
        assert!(
            matches!(&err, Error::PluginArchiveMissing { name } if name == "MyMod - Main.ba2"),
            "error: {err:?}"
        );
        assert_eq!(
            err.to_string(),
            "Plugin archive MyMod - Main.ba2 not found in Data. It holds the precombined meshes \
             previs refers to, so restore it or rebuild from Step 1"
        );
        assert!(warnings.is_empty(), "warnings: {warnings:?}");
        assert_eq!(process.calls(), Vec::<RecordedProcessCall>::new());
        assert_eq!(fixture.wait.delays(), Vec::<u64>::new());
        assert!(fixture.files.is_file(&vis_uvd(&fixture)));
    }

    /// Regression pin for the `:Archive` fall-through divergence (docs/episodes.md § *Archive*),
    /// for Step 8: a failed extract stops the step whether or not the run is interactive, and
    /// leaves the old archive and the new previs where they were.
    #[test]
    fn a_failed_extract_stops_step_eight_whether_or_not_it_is_interactive() {
        for non_interactive in [true, false] {
            let fixture = step_eight_fixture(non_interactive);
            assert_eq!(fixture.run.config().non_interactive, non_interactive);
            add_archive_and_previs(&fixture);
            // An extract that exits 1 and writes nothing at all.
            let process = RecordingProcessRunner::new().returning_exit_code(1);

            let (result, warnings) = fixture.run_step_eight_collecting_warnings(&process);

            let err = result.unwrap_err();
            let case = format!("non-interactive: {non_interactive}");
            assert!(
                matches!(err, Error::Archive2ExtractFailed { code: Some(1) }),
                "{case}, error: {err:?}"
            );
            assert!(warnings.is_empty(), "{case}, warnings: {warnings:?}");
            assert_eq!(process.calls().len(), 1, "{case}");
            assert_eq!(
                fixture.files.read_lossy(&plugin_archive(&fixture)).unwrap(),
                "old archive",
                "{case}"
            );
            assert!(fixture.files.is_file(&vis_uvd(&fixture)), "{case}");
        }
    }

    /// Step 8 needs the archive tool and nothing else: with only the Archive episode bound, it
    /// rebuilds the archive and completes.
    #[test]
    fn step_eight_runs_with_the_archive_tool_alone() {
        let fixture = step_eight_fixture(true);
        add_archive_and_previs(&fixture);
        let process = successful_archive2_rebuild(&fixture);
        let warnings = BuildWarnings::new(fixture.run.log_path().to_path_buf(), &fixture.files);
        let archive = fixture.run.archive().unwrap().bind(ArchivePorts {
            process: &process,
            wait: &fixture.wait,
            files: &fixture.files,
            warnings: &warnings,
        });
        let ports = OperationPorts {
            ck: None,
            fo4edit: None,
            archive: Some(&archive),
            prompts: &fixture.prompts,
            files: &fixture.files,
            warnings: &warnings,
        };

        add_previs_to_archive::run(&fixture.run, &ports).unwrap();

        assert_eq!(process.calls().len(), 2);
        assert_eq!(
            fixture.files.read_lossy(&plugin_archive(&fixture)).unwrap(),
            PACKED_ARCHIVE
        );
    }

    /// Step 8 reaches the archive tool through `ports.archive()?`: a run that prepared none
    /// stops with the preparation error before the workspace is looked at.
    #[test]
    fn step_eight_without_the_archive_tool_stops_before_anything_is_looked_at() {
        let fixture = step_eight_fixture(true);
        let warnings = BuildWarnings::new(fixture.run.log_path().to_path_buf(), &fixture.files);
        let ports = OperationPorts {
            ck: None,
            fo4edit: None,
            archive: None,
            prompts: &fixture.prompts,
            files: &fixture.files,
            warnings: &warnings,
        };

        // No `.uvd` either: the preparation error must win over Step 8's own warning.
        let error = add_previs_to_archive::run(&fixture.run, &ports).unwrap_err();

        assert!(
            matches!(error, Error::ArchiveNotPrepared),
            "error: {error:?}"
        );
        assert_eq!(warnings.raised(), []);
    }
}
