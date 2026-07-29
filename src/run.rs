//! Prepared workflow runs.
//!
//! This module turns user intent into a validated, executable workflow run while
//! leaving prompts and presentation to adapters such as `main`.

use std::path::{Path, PathBuf};

use crate::config::{ArchiveTool, BuildMode, PluginIdentity, ProjectConfig, WorkflowStep};
use crate::error::{Error, Result};
use crate::logging;
use crate::toolchain::{ToolchainDiagnostic, WorkflowToolchainProbe};
use crate::tools::ToolContext;
use crate::validation;
use crate::workflow::operations::{
    OperationAdapters, ProductionOperationAdapters, WorkflowOperationExecutor,
};
use crate::workflow::{OperationCapability, WorkflowPlan};

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
    /// Returns the fully validated run, or propagates request validation, unimplemented resume,
    /// toolchain readiness, log initialization, and Creation Kit context errors.
    pub fn prepare(
        request: &WorkflowRequest,
        exe_dir: &Path,
        probe: &WorkflowToolchainProbe,
    ) -> Result<Self> {
        let config = request.to_project_config(probe)?;
        let plan = WorkflowPlan::new(config.build_mode, config.resume_from)?;
        Self::prepare_with_plan(config, exe_dir, probe, plan)
    }

    /// Prepare a workflow run against synthetic capability during the ticket #8 migration.
    ///
    /// Returns the fully validated run, or propagates request validation, capability planning,
    /// toolchain readiness, log initialization, and Creation Kit context errors.
    pub fn prepare_with_capability(
        request: &WorkflowRequest,
        exe_dir: &Path,
        probe: &WorkflowToolchainProbe,
        capability: OperationCapability,
    ) -> Result<Self> {
        let config = request.to_project_config(probe)?;
        let plan =
            WorkflowPlan::new_with_capability(config.build_mode, config.resume_from, capability)?;
        Self::prepare_with_plan(config, exe_dir, probe, plan)
    }

    /// Complete preparation from a validated config and its already-resolved Workflow Plan.
    ///
    /// Returns the ready-to-execute run, or propagates toolchain readiness, log initialization,
    /// and Creation Kit context errors. Request and plan validation must happen before this helper.
    fn prepare_with_plan(
        config: ProjectConfig,
        exe_dir: &Path,
        probe: &WorkflowToolchainProbe,
        plan: WorkflowPlan,
    ) -> Result<Self> {
        let requirements =
            WorkflowOperationExecutor::<ProductionOperationAdapters>::toolchain_requirements_for_steps(
                plan.runnable_steps(),
            );
        let toolchain = probe.prepare(exe_dir, config.archive_tool, requirements)?;
        let mut diagnostics = toolchain
            .diagnostics()
            .iter()
            .cloned()
            .map(RunDiagnostic::Toolchain)
            .collect::<Vec<_>>();
        if plan.is_partial_due_to_capability() {
            diagnostics.push(RunDiagnostic::LaterStepsNotImplemented {
                skipped: plan.skipped_unrunnable_count(),
                planned: plan.planned_steps().len(),
            });
        }

        let log_path = logging::session_log_path(&config.plugin);
        logging::init_session_log(
            &log_path,
            config.build_mode.as_str(),
            &config.plugin.file_name,
        )?;

        let ctx = if requirements.needs_creation_kit() {
            toolchain.creation_kit_context(log_path.clone())?
        } else {
            ToolContext {
                session_log: Some(log_path.clone()),
                unattended_log: Some(logging::unattended_log_path()),
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
        self.execute_with(&WorkflowOperationExecutor::production())
    }

    /// Execute the runnable subset of the prepared workflow through supplied operations.
    pub fn execute_with<A: OperationAdapters>(
        &self,
        executor: &WorkflowOperationExecutor<A>,
    ) -> Result<()> {
        if executor.capability() != self.plan.capability() {
            return Err(Error::OperationCapabilityMismatch {
                planned: capability_step_numbers(self.plan.capability()),
                executor: capability_step_numbers(executor.capability()),
            });
        }

        executor.run_steps(self.plan.runnable_steps(), self)
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

    /// Steps that will be executed by the current operation capability.
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

fn capability_step_numbers(capability: OperationCapability) -> Vec<u8> {
    capability
        .runnable_steps()
        .iter()
        .map(|step| step.number())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::PluginIdentity;
    use crate::discovery::ToolPaths;
    use crate::tools::CkOperation;
    use std::fs;
    use tempfile::{TempDir, tempdir};

    #[derive(Debug)]
    struct NoopAdapters;

    impl OperationAdapters for NoopAdapters {
        fn run_creation_kit(
            &self,
            _run: &WorkflowRun,
            _operation: CkOperation,
            _plugin_file: &str,
            _qualifiers: &str,
        ) -> Result<()> {
            unreachable!("capability mismatch should stop before execution")
        }

        fn confirm_clear_precombined(&self, _precombined_dir: &Path) -> Result<bool> {
            unreachable!("capability mismatch should stop before execution")
        }
    }

    struct ReadyWorkflowFixture {
        directory: TempDir,
        fallout4_directory: PathBuf,
        probe: WorkflowToolchainProbe,
        request: WorkflowRequest,
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

        let run = WorkflowRun::prepare(&fixture.request, fixture.directory.path(), &fixture.probe)
            .unwrap();

        assert_eq!(run.runnable_steps(), &[WorkflowStep::GeneratePrecombines]);
        assert_eq!(
            run.plan().runnable_steps(),
            &[WorkflowStep::GeneratePrecombines]
        );
        assert!(run.planned_steps().len() > run.runnable_steps().len());
        assert!(run.diagnostics().iter().any(|diagnostic| matches!(
            diagnostic,
            RunDiagnostic::LaterStepsNotImplemented { .. }
        )));
        assert_eq!(
            run.tool_context().ck_log_path,
            fixture.fallout4_directory.join("CK.log")
        );
    }

    #[test]
    fn prepare_with_capability_stops_at_first_unavailable_operation() {
        let fixture = ready_workflow_fixture();
        let capability = OperationCapability::new(&[
            WorkflowStep::GeneratePrecombines,
            WorkflowStep::CreateBa2FromPrecombines,
        ]);

        let run = WorkflowRun::prepare_with_capability(
            &fixture.request,
            fixture.directory.path(),
            &fixture.probe,
            capability,
        )
        .unwrap();

        assert_eq!(run.runnable_steps(), &[WorkflowStep::GeneratePrecombines]);
    }

    #[test]
    fn execute_with_rejects_executor_capability_mismatch() {
        let fixture = ready_workflow_fixture();
        let run = WorkflowRun::prepare(&fixture.request, fixture.directory.path(), &fixture.probe)
            .unwrap();
        let executor = WorkflowOperationExecutor::new_with_capability(
            NoopAdapters,
            OperationCapability::new(&[]),
        );

        let err = run.execute_with(&executor).unwrap_err();

        assert!(matches!(
            err,
            Error::OperationCapabilityMismatch { planned, executor }
                if planned == vec![1] && executor.is_empty()
        ));
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

        let run = WorkflowRun::prepare(&request, dir.path(), &probe).unwrap();

        assert_eq!(run.tool_context().ck_log_path, fo4.join("CK.log"));
    }
}
