//! Workflow Operations for the planned 8-step build.
//!
//! A Workflow Operation owns the domain flow for a step. External tools stay behind
//! adapters so tests can exercise ordering and postconditions without launching CK.

use std::path::Path;

use crate::config::{BuildMode, WorkflowStep};
use crate::error::{Error, Result};
use crate::files::{FileSpace, SystemFileSpace};
use crate::interactive;
use crate::run::WorkflowRun;
use crate::toolchain::ToolchainRequirements;
use crate::tools::{CkOperation, CreationKitOps};
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

type OperationExecution = fn(&WorkflowRun, &dyn OperationAdapters) -> Result<()>;

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

    /// Dispatch one registered Workflow Operation through the supplied adapters.
    ///
    /// Returns [`Error::StepNotImplemented`] when production has no operation for `step`, and
    /// otherwise propagates errors from the selected operation's domain flow or adapters.
    fn dispatch(
        self,
        step: WorkflowStep,
        run: &WorkflowRun,
        adapters: &dyn OperationAdapters,
    ) -> Result<()> {
        tracing::info!(step = step.number(), "{}", step.label());
        let operation = self
            .operation_for_step(step)
            .ok_or_else(|| Error::StepNotImplemented(step.number()))?;
        (operation.execute)(run, adapters)
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
    adapters: &dyn OperationAdapters,
) -> Result<()> {
    let operations = production_operation_source();
    for step in run.runnable_steps() {
        operations.dispatch(*step, run, adapters)?;
    }
    Ok(())
}

/// Adapters required by Workflow Operations.
pub(crate) trait OperationAdapters {
    /// Run a Creation Kit command-line operation for a Workflow Run.
    fn run_creation_kit(
        &self,
        run: &WorkflowRun,
        operation: CkOperation,
        plugin_file: &str,
        qualifiers: &str,
    ) -> Result<()>;

    /// Ask whether existing precombined meshes should be cleared before Step 1 resumes.
    fn confirm_clear_precombined(&self, precombined_dir: &Path) -> Result<bool>;

    /// The `FileSpace` every Workflow Operation reads and cleans up external-tool outputs through.
    ///
    /// One adapter shared by all planned steps, reached through this bundle like every other.
    fn files(&self) -> &dyn FileSpace;
}

/// Production adapters for external tools and interactive prompts.
#[derive(Debug, Default)]
pub(crate) struct ProductionOperationAdapters {
    ck: CreationKitOps,
    files: SystemFileSpace,
}

impl ProductionOperationAdapters {
    /// Create production operation adapters.
    #[must_use]
    pub(crate) const fn new() -> Self {
        Self {
            ck: CreationKitOps,
            files: SystemFileSpace,
        }
    }
}

impl OperationAdapters for ProductionOperationAdapters {
    fn files(&self) -> &dyn FileSpace {
        &self.files
    }

    fn run_creation_kit(
        &self,
        run: &WorkflowRun,
        operation: CkOperation,
        plugin_file: &str,
        qualifiers: &str,
    ) -> Result<()> {
        self.ck.run(
            run.tool_context(),
            operation,
            plugin_file,
            qualifiers,
            &self.files,
        )
    }

    fn confirm_clear_precombined(&self, precombined_dir: &Path) -> Result<bool> {
        interactive::confirm_clear_precombined(precombined_dir)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use tempfile::{TempDir, tempdir};

    use super::recording_adapters::RecordingOperationAdapters;
    use super::*;
    use crate::config::{ArchiveTool, BuildMode, PluginIdentity};
    use crate::run::WorkflowRequest;
    use crate::{discovery::ToolPaths, toolchain::WorkflowToolchainProbe};

    /// Provide an execution entry for registration-only tests that must never dispatch.
    fn unused_execution(_run: &WorkflowRun, _adapters: &dyn OperationAdapters) -> Result<()> {
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

    fn prepared_run(
        mode: BuildMode,
        resume_from: Option<WorkflowStep>,
        non_interactive: bool,
    ) -> (TempDir, WorkflowRun) {
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
            None,
        );

        // A per-call in-memory space, so this module's `MyMod` runs never share a session log
        // with the identically-named fixtures in `run` and `precombine_workspace`.
        let run = WorkflowRun::prepare(
            &request,
            dir.path(),
            &probe,
            &crate::files::InMemoryFileSpace::new(),
        )
        .unwrap();
        (dir, run)
    }

    #[test]
    fn step_one_selects_creation_kit_qualifiers_from_the_build_mode() {
        let cases = [
            (BuildMode::Clean, "clean all"),
            (BuildMode::Filtered, "filtered all"),
            (BuildMode::Xbox, "filtered all"),
        ];

        for (mode, expected_qualifiers) in cases {
            let (_dir, run) = prepared_run(mode, None, true);
            let adapters = RecordingOperationAdapters::new();

            generate_precombines::run(&run, &adapters).unwrap();

            let calls = adapters.creation_kit_calls();
            assert_eq!(calls.len(), 1, "mode: {mode:?}");
            assert_eq!(calls[0].operation, CkOperation::GeneratePrecombined);
            assert_eq!(calls[0].plugin_file, "MyMod.esp");
            assert_eq!(calls[0].qualifiers, expected_qualifiers, "mode: {mode:?}");
        }
    }

    #[test]
    fn step_one_rejects_existing_plugin_archive_before_ck() {
        let (_dir, run) = prepared_run(BuildMode::Filtered, None, true);
        let adapters = RecordingOperationAdapters::new();
        adapters.file_space().add_file(run.config().plugin_archive_path());

        let err = generate_precombines::run(&run, &adapters).unwrap_err();

        assert!(matches!(err, Error::PluginAlreadyHasArchive));
        assert!(adapters.creation_kit_calls().is_empty());
    }

    #[test]
    fn step_one_rejects_vis_uvd_files_before_ck() {
        let (_dir, run) = prepared_run(BuildMode::Filtered, None, true);
        let adapters = RecordingOperationAdapters::new();
        adapters
            .file_space()
            .add_file(run.config().vis_dir().join("cell.uvd"));

        let err = generate_precombines::run(&run, &adapters).unwrap_err();

        assert!(matches!(err, Error::VisUvdFilesExist));
        assert!(adapters.creation_kit_calls().is_empty());
    }

    #[test]
    fn step_one_reports_missing_combined_objects_from_operation() {
        let (_dir, run) = prepared_run(BuildMode::Clean, None, true);
        let adapters = RecordingOperationAdapters::new().without_combined_objects();

        let err = generate_precombines::run(&run, &adapters).unwrap_err();

        assert!(matches!(err, Error::MissingCombinedObjects));
    }

    #[test]
    fn step_one_reports_missing_geometry_psg_from_operation() {
        let (_dir, run) = prepared_run(BuildMode::Clean, None, true);
        let adapters = RecordingOperationAdapters::new().without_geometry_psg();

        let err = generate_precombines::run(&run, &adapters).unwrap_err();

        assert!(matches!(err, Error::MissingGeometryPsg(name) if name == "MyMod"));
    }

    #[test]
    fn step_one_reports_no_precombined_meshes_from_operation() {
        let (_dir, run) = prepared_run(BuildMode::Filtered, None, true);
        let adapters = RecordingOperationAdapters::new().without_precombined_meshes();

        let err = generate_precombines::run(&run, &adapters).unwrap_err();

        assert!(matches!(err, Error::NoPrecombinedMeshes));
    }

    #[test]
    fn step_one_reports_handle_array_log_error_from_operation() {
        let (_dir, run) = prepared_run(BuildMode::Filtered, None, true);
        let adapters = RecordingOperationAdapters::new()
            .with_ck_log_contents("DEFAULT: OUT OF HANDLE ARRAY ENTRIES\n");

        let err = generate_precombines::run(&run, &adapters).unwrap_err();

        assert!(matches!(err, Error::HandleArrayLogError));
    }

    #[test]
    fn step_one_prompt_clears_precombined_meshes_when_interactive() {
        let (_dir, run) = prepared_run(BuildMode::Filtered, None, false);
        let adapters = RecordingOperationAdapters::new();
        let existing_mesh = run.config().precombined_dir().join("old").join("mesh.nif");
        adapters.file_space().add_file(&existing_mesh);

        generate_precombines::run(&run, &adapters).unwrap();

        assert_eq!(adapters.clear_prompt_count(), 1);
        assert!(!adapters.file_space().is_file(&existing_mesh));
    }

    #[test]
    fn step_one_stops_when_the_clear_precombined_prompt_is_refused() {
        let (_dir, run) = prepared_run(BuildMode::Filtered, None, false);
        let adapters = RecordingOperationAdapters::new().refusing_clear_precombined();
        let existing_mesh = run.config().precombined_dir().join("old").join("mesh.nif");
        adapters.file_space().add_file(&existing_mesh);

        let err = generate_precombines::run(&run, &adapters).unwrap_err();

        assert!(
            matches!(err, Error::Other(message) if message
                == "precombined meshes not cleared - choose another resume step")
        );
        assert_eq!(adapters.clear_prompt_count(), 1);
        assert!(adapters.creation_kit_calls().is_empty());
        // A refusal leaves the meshes alone; the run stops rather than clearing anyway.
        assert!(adapters.file_space().is_file(&existing_mesh));
    }

    #[test]
    fn step_one_rejects_existing_precombined_meshes_without_prompting_when_non_interactive() {
        let (_dir, run) = prepared_run(BuildMode::Filtered, None, true);
        let adapters = RecordingOperationAdapters::new();
        adapters
            .file_space()
            .add_file(run.config().precombined_dir().join("old").join("mesh.nif"));

        let err = generate_precombines::run(&run, &adapters).unwrap_err();

        assert!(matches!(err, Error::PrecombinedMeshesExist));
        assert_eq!(adapters.clear_prompt_count(), 0);
        assert!(adapters.creation_kit_calls().is_empty());
    }

    #[test]
    fn unregistered_source_dispatch_fails_before_adapters() {
        let (_dir, run) = prepared_run(BuildMode::Filtered, None, true);
        let adapters = RecordingOperationAdapters::new();

        let err = production_operation_source()
            .dispatch(WorkflowStep::MergePrecombineObjects, &run, &adapters)
            .unwrap_err();

        assert!(matches!(err, Error::StepNotImplemented(2)));
        assert!(adapters.creation_kit_calls().is_empty());
        assert_eq!(adapters.clear_prompt_count(), 0);
    }
}
