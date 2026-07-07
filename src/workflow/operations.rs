//! Workflow Operations for the planned 8-step build.
//!
//! A Workflow Operation owns the domain flow for a step. External tools stay behind
//! adapters so tests can exercise ordering and postconditions without launching CK.

use std::path::Path;

use crate::checks::{self, has_precombined_nifs};
use crate::config::WorkflowStep;
use crate::error::{Error, Result};
use crate::interactive;
use crate::run::WorkflowRun;
use crate::tools::{CkOperation, CreationKitOps};
use crate::workflow::OperationCapability;

const PRODUCTION_RUNNABLE_STEPS: &[WorkflowStep] = &[WorkflowStep::GeneratePrecombines];

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
        Self { adapters }
    }

    /// Steps implemented by the production operation executor.
    #[must_use]
    pub const fn capability() -> OperationCapability {
        OperationCapability::new(PRODUCTION_RUNNABLE_STEPS)
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
        tracing::info!(step = step.number(), "{}", step.label());
        match step {
            WorkflowStep::GeneratePrecombines => self.run_generate_precombines(run),
            _ => Err(Error::StepNotImplemented(step.number())),
        }
    }

    fn run_generate_precombines(&self, run: &WorkflowRun) -> Result<()> {
        let config = run.config();
        self.maybe_clear_precombined_on_resume(run)?;

        checks::run_precomb_preamble(config)?;

        let qualifiers = checks::precombine_qualifiers(config.build_mode);
        self.adapters.run_creation_kit(
            run,
            CkOperation::GeneratePrecombined,
            &config.plugin.file_name,
            qualifiers,
        )?;

        let combined = config.fo4edit_data_dir().join("CombinedObjects.esp");
        if !combined.is_file() {
            return Err(Error::MissingCombinedObjects);
        }

        let ck_log = run
            .tool_context()
            .ck_log_path
            .clone()
            .ok_or_else(|| Error::Other("CK log path not configured".into()))?;

        checks::run_post_precomb_checks(config, &ck_log)?;

        Ok(())
    }

    fn maybe_clear_precombined_on_resume(&self, run: &WorkflowRun) -> Result<()> {
        let config = run.config();
        let resume_step1 = config
            .resume_from
            .is_none_or(|s| s == WorkflowStep::GeneratePrecombines);

        if !resume_step1 {
            return Ok(());
        }

        let precombined = config.precombined_dir();
        if !has_precombined_nifs(&precombined) {
            return Ok(());
        }

        if config.non_interactive {
            return Err(Error::PrecombinedMeshesExist);
        }

        if !self.adapters.confirm_clear_precombined(&precombined)? {
            return Err(Error::Other(
                "precombined meshes not cleared - choose another resume step".into(),
            ));
        }

        if precombined.is_dir() {
            std::fs::remove_dir_all(precombined)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::fs;
    use tempfile::{TempDir, tempdir};

    use super::*;
    use crate::config::{ArchiveTool, BuildMode, PluginIdentity};
    use crate::discovery::ToolPaths;
    use crate::run::WorkflowRequest;

    #[derive(Debug)]
    struct RecordingAdapters {
        ck_calls: RefCell<Vec<(CkOperation, String, String)>>,
        clear_prompts: Cell<usize>,
        clear_response: bool,
        create_combined: bool,
    }

    impl RecordingAdapters {
        fn new(create_combined: bool) -> Self {
            Self {
                ck_calls: RefCell::new(Vec::new()),
                clear_prompts: Cell::new(0),
                clear_response: true,
                create_combined,
            }
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
            if self.create_combined {
                fs::write(data.join("CombinedObjects.esp"), b"combined")?;
            }

            let precombined_mesh = run.config().precombined_dir().join("test").join("mesh.nif");
            fs::create_dir_all(precombined_mesh.parent().unwrap())?;
            fs::write(precombined_mesh, b"nif")?;

            if run.config().build_mode == BuildMode::Clean {
                fs::write(
                    data.join(format!("{} - Geometry.psg", run.config().plugin.base_name)),
                    b"psg",
                )?;
            }

            if let Some(ck_log) = &run.tool_context().ck_log_path {
                fs::write(ck_log, b"ok\n")?;
            }

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

        let tools = ToolPaths {
            fallout4_dir: Some(fallout4_dir.clone()),
            creation_kit: Some(fallout4_dir.join("CreationKit.exe")),
            ..ToolPaths::default()
        };

        let request = WorkflowRequest::new(
            mode,
            ArchiveTool::Archive2,
            PluginIdentity::parse("MyMod"),
            non_interactive,
            resume_from,
            None,
        );

        let run = WorkflowRun::prepare(&request, dir.path(), tools).unwrap();
        (dir, run)
    }

    #[test]
    fn production_capability_starts_with_step_one_only() {
        let capability = WorkflowOperationExecutor::<ProductionOperationAdapters>::capability();
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
    fn step_one_reports_missing_combined_objects_from_operation() {
        let (_dir, run) = prepared_run(BuildMode::Clean, None, true);
        let executor = WorkflowOperationExecutor::new(RecordingAdapters::new(false));

        let err = executor
            .run_step(WorkflowStep::GeneratePrecombines, &run)
            .unwrap_err();

        assert!(matches!(err, Error::MissingCombinedObjects));
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
