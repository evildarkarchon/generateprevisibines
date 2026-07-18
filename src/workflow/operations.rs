//! Workflow Operations for the planned 8-step build.
//!
//! A Workflow Operation owns the domain flow for a step. External tools stay behind
//! adapters so tests can exercise ordering and postconditions without launching CK.

use std::path::Path;

use crate::config::WorkflowStep;
use crate::error::{Error, Result};
use crate::interactive;
use crate::run::WorkflowRun;
use crate::toolchain::ToolchainRequirements;
use crate::tools::{CkOperation, CreationKitOps};
use crate::workflow::OperationCapability;

mod generate_precombines;
mod precombine_workspace;

macro_rules! register_production_operations {
    ($($operation:expr),+ $(,)?) => {
        const PRODUCTION_OPERATION_SOURCE: ProductionOperationSource =
            ProductionOperationSource::new(&[$($operation),+]);

        // Ticket #8 removes this compatibility slice; generate it here so registration cannot drift meanwhile.
        const PRODUCTION_RUNNABLE_STEPS: &[WorkflowStep] = &[$(($operation).step),+];
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
pub struct ProductionOperationSource {
    operations: &'static [WorkflowOperationDefinition],
}

impl ProductionOperationSource {
    const fn new(operations: &'static [WorkflowOperationDefinition]) -> Self {
        Self { operations }
    }

    /// Whether the production build has an implementation for a Workflow Step.
    #[must_use]
    pub fn contains(self, step: WorkflowStep) -> bool {
        self.operation_for_step(step).is_some()
    }

    /// Filter planned steps to registered operations without changing plan order.
    #[must_use]
    pub fn filter_steps(self, steps: &[WorkflowStep]) -> Vec<WorkflowStep> {
        steps
            .iter()
            .copied()
            .filter(|step| self.contains(*step))
            .collect()
    }

    /// Aggregate static readiness requirements for registered steps in a Workflow Plan.
    #[must_use]
    pub fn toolchain_requirements_for_steps(self, steps: &[WorkflowStep]) -> ToolchainRequirements {
        steps
            .iter()
            .filter_map(|step| self.operation_for_step(*step))
            .fold(ToolchainRequirements::none(), |requirements, operation| {
                requirements.union(operation.toolchain_requirements)
            })
    }

    /// Dispatch one registered Workflow Operation through the supplied adapters.
    ///
    /// Returns [`Error::StepNotImplemented`] when production has no operation for `step`, and
    /// otherwise propagates errors from the selected operation's domain flow or adapters.
    pub fn dispatch(
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
pub const fn production_operation_source() -> ProductionOperationSource {
    PRODUCTION_OPERATION_SOURCE
}

/// Adapters required by Workflow Operations.
pub trait OperationAdapters {
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
}

/// Production adapters for external tools and interactive prompts.
#[derive(Debug, Default)]
pub struct ProductionOperationAdapters {
    ck: CreationKitOps,
}

impl ProductionOperationAdapters {
    /// Create production operation adapters.
    #[must_use]
    pub const fn new() -> Self {
        Self { ck: CreationKitOps }
    }
}

impl OperationAdapters for ProductionOperationAdapters {
    fn run_creation_kit(
        &self,
        run: &WorkflowRun,
        operation: CkOperation,
        plugin_file: &str,
        qualifiers: &str,
    ) -> Result<()> {
        self.ck
            .run(run.tool_context(), operation, plugin_file, qualifiers)
    }

    fn confirm_clear_precombined(&self, precombined_dir: &Path) -> Result<bool> {
        interactive::confirm_clear_precombined(precombined_dir)
    }
}

/// Executes Workflow Operations through a supplied adapter bundle.
#[derive(Debug)]
pub struct WorkflowOperationExecutor<A> {
    adapters: A,
    capability: OperationCapability,
}

impl WorkflowOperationExecutor<ProductionOperationAdapters> {
    /// Create an executor backed by production adapters.
    #[must_use]
    pub const fn production() -> Self {
        Self::new(ProductionOperationAdapters::new())
    }
}

impl<A: OperationAdapters> WorkflowOperationExecutor<A> {
    /// Create an executor backed by the provided adapters.
    #[must_use]
    pub const fn new(adapters: A) -> Self {
        Self::new_with_capability(
            adapters,
            OperationCapability::new(PRODUCTION_RUNNABLE_STEPS),
        )
    }

    /// Create an executor backed by the provided adapters and explicit operation capability.
    #[must_use]
    pub const fn new_with_capability(adapters: A, capability: OperationCapability) -> Self {
        Self {
            adapters,
            capability,
        }
    }

    /// Steps this operation executor can run.
    #[must_use]
    pub const fn capability(&self) -> OperationCapability {
        self.capability
    }

    /// Steps implemented by the production operation executor.
    #[must_use]
    pub const fn production_capability() -> OperationCapability {
        OperationCapability::new(PRODUCTION_RUNNABLE_STEPS)
    }

    /// Toolchain requirements for a set of runnable Workflow Operations.
    #[must_use]
    pub fn toolchain_requirements_for_steps(steps: &[WorkflowStep]) -> ToolchainRequirements {
        production_operation_source().toolchain_requirements_for_steps(steps)
    }

    /// Execute the runnable subset of a Workflow Plan.
    pub fn run_steps(&self, steps: &[WorkflowStep], run: &WorkflowRun) -> Result<()> {
        for step in steps {
            self.run_step(*step, run)?;
        }
        Ok(())
    }

    /// Execute one Workflow Operation.
    pub fn run_step(&self, step: WorkflowStep, run: &WorkflowRun) -> Result<()> {
        production_operation_source().dispatch(step, run, &self.adapters)
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::fs;
    use tempfile::{TempDir, tempdir};

    use super::*;
    use crate::config::{ArchiveTool, BuildMode, PluginIdentity};
    use crate::run::WorkflowRequest;
    use crate::{discovery::ToolPaths, toolchain::WorkflowToolchainProbe};

    #[test]
    fn production_source_filters_registered_steps_in_plan_order() {
        let source = production_operation_source();
        let planned = WorkflowStep::steps_for_mode(BuildMode::Clean);

        assert!(source.contains(WorkflowStep::GeneratePrecombines));
        assert!(!source.contains(WorkflowStep::MergePrecombineObjects));
        assert_eq!(
            source.filter_steps(planned),
            vec![WorkflowStep::GeneratePrecombines]
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
    fn compatibility_requirements_delegate_to_production_source() {
        let requirements = WorkflowOperationExecutor::<ProductionOperationAdapters>::
            toolchain_requirements_for_steps(WorkflowStep::steps_for_mode(BuildMode::Clean));

        assert!(requirements.needs_creation_kit());
        assert!(!requirements.needs_fo4edit());
        assert!(!requirements.needs_archive());
    }

    #[derive(Debug)]
    struct RecordingAdapters {
        ck_calls: RefCell<Vec<(CkOperation, String, String)>>,
        clear_prompts: Cell<usize>,
        clear_response: bool,
        artifacts: RecordedArtifacts,
        ck_log_contents: &'static [u8],
    }

    #[derive(Debug, Clone, Copy)]
    struct RecordedArtifacts {
        combined_objects: ArtifactState,
        precombined_mesh: ArtifactState,
        geometry_psg: ArtifactState,
    }

    impl RecordedArtifacts {
        const fn with_combined_objects(combined_objects: ArtifactState) -> Self {
            Self {
                combined_objects,
                precombined_mesh: ArtifactState::Created,
                geometry_psg: ArtifactState::Created,
            }
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum ArtifactState {
        Created,
        Missing,
    }

    impl ArtifactState {
        const fn should_create(self) -> bool {
            matches!(self, Self::Created)
        }
    }

    impl RecordingAdapters {
        fn new(create_combined: bool) -> Self {
            let combined_objects = if create_combined {
                ArtifactState::Created
            } else {
                ArtifactState::Missing
            };

            Self {
                ck_calls: RefCell::new(Vec::new()),
                clear_prompts: Cell::new(0),
                clear_response: true,
                artifacts: RecordedArtifacts::with_combined_objects(combined_objects),
                ck_log_contents: b"ok\n",
            }
        }

        fn without_precombined_mesh(mut self) -> Self {
            self.artifacts.precombined_mesh = ArtifactState::Missing;
            self
        }

        fn without_psg(mut self) -> Self {
            self.artifacts.geometry_psg = ArtifactState::Missing;
            self
        }

        fn with_ck_log_contents(mut self, contents: &'static [u8]) -> Self {
            self.ck_log_contents = contents;
            self
        }
    }

    impl OperationAdapters for RecordingAdapters {
        fn run_creation_kit(
            &self,
            run: &WorkflowRun,
            operation: CkOperation,
            plugin_file: &str,
            qualifiers: &str,
        ) -> Result<()> {
            self.ck_calls.borrow_mut().push((
                operation,
                plugin_file.to_string(),
                qualifiers.to_string(),
            ));

            let data = run.config().fo4edit_data_dir();
            fs::create_dir_all(&data)?;
            if self.artifacts.combined_objects.should_create() {
                fs::write(data.join("CombinedObjects.esp"), b"combined")?;
            }

            if self.artifacts.precombined_mesh.should_create() {
                let precombined_mesh = run.config().precombined_dir().join("test").join("mesh.nif");
                fs::create_dir_all(precombined_mesh.parent().unwrap())?;
                fs::write(precombined_mesh, b"nif")?;
            }

            if run.config().build_mode == BuildMode::Clean
                && self.artifacts.geometry_psg.should_create()
            {
                fs::write(
                    data.join(format!("{} - Geometry.psg", run.config().plugin.base_name)),
                    b"psg",
                )?;
            }

            fs::write(&run.tool_context().ck_log_path, self.ck_log_contents)?;

            Ok(())
        }

        fn confirm_clear_precombined(&self, _precombined_dir: &Path) -> Result<bool> {
            self.clear_prompts.set(self.clear_prompts.get() + 1);
            Ok(self.clear_response)
        }
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

        let run = WorkflowRun::prepare(&request, dir.path(), &probe).unwrap();
        (dir, run)
    }

    #[test]
    fn production_source_dispatches_registered_operation_through_recording_adapter() {
        let (_dir, run) = prepared_run(BuildMode::Filtered, None, true);
        let adapters = RecordingAdapters::new(true);

        production_operation_source()
            .dispatch(WorkflowStep::GeneratePrecombines, &run, &adapters)
            .unwrap();

        let calls = adapters.ck_calls.borrow();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, CkOperation::GeneratePrecombined);
        assert_eq!(calls[0].1, "MyMod.esp");
        assert_eq!(calls[0].2, "filtered all");
    }

    #[test]
    fn production_capability_starts_with_step_one_only() {
        let capability =
            WorkflowOperationExecutor::<ProductionOperationAdapters>::production_capability();
        assert_eq!(
            capability.runnable_steps(),
            &[WorkflowStep::GeneratePrecombines]
        );
    }

    #[test]
    fn step_one_runs_through_operation_executor() {
        let (_dir, run) = prepared_run(BuildMode::Clean, None, true);
        let adapters = RecordingAdapters::new(true);
        let executor = WorkflowOperationExecutor::new(adapters);

        executor
            .run_step(WorkflowStep::GeneratePrecombines, &run)
            .unwrap();

        let calls = executor.adapters.ck_calls.borrow();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, CkOperation::GeneratePrecombined);
        assert_eq!(calls[0].1, "MyMod.esp");
        assert_eq!(calls[0].2, "clean all");
        assert!(
            run.config()
                .precombined_dir()
                .join("test")
                .join("mesh.nif")
                .is_file()
        );
    }

    #[test]
    fn step_one_uses_filtered_qualifiers_for_filtered_mode() {
        let (_dir, run) = prepared_run(BuildMode::Filtered, None, true);
        let adapters = RecordingAdapters::new(true);
        let executor = WorkflowOperationExecutor::new(adapters);

        executor
            .run_step(WorkflowStep::GeneratePrecombines, &run)
            .unwrap();

        let calls = executor.adapters.ck_calls.borrow();
        assert_eq!(calls[0].2, "filtered all");
    }

    #[test]
    fn step_one_rejects_existing_plugin_archive_before_ck() {
        let (_dir, run) = prepared_run(BuildMode::Filtered, None, true);
        fs::create_dir_all(run.config().fo4edit_data_dir()).unwrap();
        fs::write(run.config().plugin_archive_path(), b"ba2").unwrap();
        let executor = WorkflowOperationExecutor::new(RecordingAdapters::new(true));

        let err = executor
            .run_step(WorkflowStep::GeneratePrecombines, &run)
            .unwrap_err();

        assert!(matches!(err, Error::PluginAlreadyHasArchive));
        assert!(executor.adapters.ck_calls.borrow().is_empty());
    }

    #[test]
    fn step_one_rejects_vis_uvd_files_before_ck() {
        let (_dir, run) = prepared_run(BuildMode::Filtered, None, true);
        fs::create_dir_all(run.config().vis_dir()).unwrap();
        fs::write(run.config().vis_dir().join("cell.uvd"), b"uvd").unwrap();
        let executor = WorkflowOperationExecutor::new(RecordingAdapters::new(true));

        let err = executor
            .run_step(WorkflowStep::GeneratePrecombines, &run)
            .unwrap_err();

        assert!(matches!(err, Error::VisUvdFilesExist));
        assert!(executor.adapters.ck_calls.borrow().is_empty());
    }

    #[test]
    fn step_one_reports_missing_combined_objects_from_operation() {
        let (_dir, run) = prepared_run(BuildMode::Clean, None, true);
        let executor = WorkflowOperationExecutor::new(RecordingAdapters::new(false));

        let err = executor
            .run_step(WorkflowStep::GeneratePrecombines, &run)
            .unwrap_err();

        assert!(matches!(err, Error::MissingCombinedObjects));
    }

    #[test]
    fn step_one_reports_missing_geometry_psg_from_operation() {
        let (_dir, run) = prepared_run(BuildMode::Clean, None, true);
        let executor = WorkflowOperationExecutor::new(RecordingAdapters::new(true).without_psg());

        let err = executor
            .run_step(WorkflowStep::GeneratePrecombines, &run)
            .unwrap_err();

        assert!(matches!(err, Error::MissingGeometryPsg(name) if name == "MyMod"));
    }

    #[test]
    fn step_one_reports_no_precombined_meshes_from_operation() {
        let (_dir, run) = prepared_run(BuildMode::Filtered, None, true);
        let executor =
            WorkflowOperationExecutor::new(RecordingAdapters::new(true).without_precombined_mesh());

        let err = executor
            .run_step(WorkflowStep::GeneratePrecombines, &run)
            .unwrap_err();

        assert!(matches!(err, Error::NoPrecombinedMeshes));
    }

    #[test]
    fn step_one_reports_handle_array_log_error_from_operation() {
        let (_dir, run) = prepared_run(BuildMode::Filtered, None, true);
        let executor = WorkflowOperationExecutor::new(
            RecordingAdapters::new(true)
                .with_ck_log_contents(b"DEFAULT: OUT OF HANDLE ARRAY ENTRIES\n"),
        );

        let err = executor
            .run_step(WorkflowStep::GeneratePrecombines, &run)
            .unwrap_err();

        assert!(matches!(err, Error::HandleArrayLogError));
    }

    #[test]
    fn step_one_prompt_clears_precombined_meshes_when_interactive() {
        let (_dir, run) = prepared_run(BuildMode::Filtered, None, false);
        let existing_mesh = run.config().precombined_dir().join("old").join("mesh.nif");
        fs::create_dir_all(existing_mesh.parent().unwrap()).unwrap();
        fs::write(&existing_mesh, b"old").unwrap();

        let executor = WorkflowOperationExecutor::new(RecordingAdapters::new(true));

        executor
            .run_step(WorkflowStep::GeneratePrecombines, &run)
            .unwrap();

        assert_eq!(executor.adapters.clear_prompts.get(), 1);
        assert!(!existing_mesh.is_file());
    }

    #[test]
    fn unimplemented_operation_fails_fast() {
        let (_dir, run) = prepared_run(BuildMode::Filtered, None, true);
        let executor = WorkflowOperationExecutor::new(RecordingAdapters::new(true));

        let err = executor
            .run_step(WorkflowStep::MergePrecombineObjects, &run)
            .unwrap_err();

        assert!(matches!(err, Error::StepNotImplemented(2)));
    }
}
