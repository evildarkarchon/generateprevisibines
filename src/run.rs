//! Prepared workflow runs.
//!
//! This module turns user intent into a validated, executable workflow run while
//! leaving prompts and presentation to adapters such as `main`.

use std::path::{Path, PathBuf};

use crate::config::{ArchiveTool, BuildMode, PluginIdentity, ProjectConfig, WorkflowStep};
use crate::discovery::{self, ToolPaths};
use crate::error::{Error, Result};
use crate::logging;
use crate::tools::ToolContext;
use crate::validation;
use crate::workflow::WorkflowPlan;
use crate::workflow::operations::{
    OperationAdapters, ProductionOperationAdapters, WorkflowOperationExecutor,
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

    /// Build the resolved project config once environment facts have been prepared.
    pub fn to_project_config(
        &self,
        fallout4_dir: PathBuf,
        fo4edit_path: Option<PathBuf>,
        ck_log_path: Option<PathBuf>,
    ) -> Result<ProjectConfig> {
        validation::validate_plugin(&self.plugin, self.build_mode)?;

        Ok(ProjectConfig {
            build_mode: self.build_mode,
            archive_tool: self.archive_tool,
            fallout4_dir,
            plugin: self.plugin.clone(),
            non_interactive: self.non_interactive,
            resume_from: self.resume_from,
            fo4edit_path,
            xedit_data_dir: self.fallout4_override.as_ref().map(|d| d.join("Data")),
            ck_log_path,
        })
    }
}

/// Structured information discovered while preparing a workflow run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunDiagnostic {
    Fo4EditDiscovered(PathBuf),
    Fallout4Directory(PathBuf),
    CkpeConfig { file_name: String, log_file: String },
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
    /// Discover external tools using the same lookup rules as the batch-compatible entrypoint.
    pub fn discover_tools(exe_dir: &Path, fallout4_override: Option<PathBuf>) -> Result<ToolPaths> {
        discovery::discover_tools(exe_dir, fallout4_override)
    }

    /// Extract the resolved Fallout 4 directory from discovered tool paths.
    pub fn fallout4_dir(tools: &ToolPaths) -> Result<PathBuf> {
        tools.fallout4_dir.clone().ok_or_else(|| {
            Error::Other("Fallout 4 directory could not be determined. Use --FO4 <DIR>.".into())
        })
    }

    /// Create diagnostics for tool paths that were already discovered.
    #[must_use]
    pub fn tool_diagnostics(tools: &ToolPaths) -> Vec<RunDiagnostic> {
        let mut diagnostics = Vec::new();
        if let Some(ref fo4edit) = tools.fo4edit {
            diagnostics.push(RunDiagnostic::Fo4EditDiscovered(fo4edit.clone()));
        }
        if let Some(ref fallout4_dir) = tools.fallout4_dir {
            diagnostics.push(RunDiagnostic::Fallout4Directory(fallout4_dir.clone()));
        }
        diagnostics
    }

    /// Prepare a workflow run by validating environment facts and building executable state.
    pub fn prepare(request: &WorkflowRequest, exe_dir: &Path, tools: ToolPaths) -> Result<Self> {
        let mut diagnostics = Self::tool_diagnostics(&tools);
        let fallout4_dir = Self::fallout4_dir(&tools)?;
        let ckpe = load_ckpe_installation(&fallout4_dir)?;

        diagnostics.push(RunDiagnostic::CkpeConfig {
            file_name: ckpe.file_name,
            log_file: ckpe.log_file,
        });

        let config = request.to_project_config(
            fallout4_dir.clone(),
            tools.fo4edit.clone(),
            Some(ckpe.log_path.clone()),
        )?;

        validate_xedit_scripts_when_available(exe_dir, &tools)?;

        let plan = WorkflowPlan::new(
            &config,
            WorkflowOperationExecutor::<ProductionOperationAdapters>::capability(),
        )?;
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

        let creation_kit = tools.creation_kit.ok_or_else(|| {
            Error::Other(format!(
                "CreationKit.exe not found in {}",
                config.fallout4_dir.display()
            ))
        })?;

        let ctx = ToolContext {
            session_log: Some(log_path.clone()),
            unattended_log: Some(logging::unattended_log_path()),
            fallout4_dir: config.fallout4_dir.clone(),
            creation_kit,
            ck_log_path: config.ck_log_path.clone(),
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

#[derive(Debug, Clone)]
struct CkpeInstallation {
    file_name: String,
    log_file: String,
    log_path: PathBuf,
}

fn load_ckpe_installation(fallout4_dir: &Path) -> Result<CkpeInstallation> {
    let kind = validation::detect_ckpe_config_kind(fallout4_dir);
    let file_name = kind.file_name().to_string();
    let ckpe_path = fallout4_dir.join(&file_name);

    if !ckpe_path.is_file() {
        return Err(Error::CkpeConfig(format!(
            "CKPE not configured. File {file_name} missing"
        )));
    }

    let contents = std::fs::read_to_string(&ckpe_path)?;
    let (kind, log_file) = validation::validate_ckpe_config(fallout4_dir, &contents)?;

    Ok(CkpeInstallation {
        file_name: kind.file_name().to_string(),
        log_path: resolve_ck_log_path(fallout4_dir, &log_file),
        log_file,
    })
}

fn resolve_ck_log_path(fallout4_dir: &Path, log_setting: &str) -> PathBuf {
    let path = Path::new(log_setting);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        fallout4_dir.join(path)
    }
}

fn validate_xedit_scripts_when_available(exe_dir: &Path, tools: &ToolPaths) -> Result<()> {
    if let Some(ref fo4edit) = tools.fo4edit {
        let scripts_dir = fo4edit.parent().unwrap_or(exe_dir).join("Edit Scripts");
        validation::validate_required_xedit_scripts(&scripts_dir)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::PluginIdentity;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn request_to_project_config_uses_fo4_override_for_xedit_data() {
        let request = WorkflowRequest::new(
            BuildMode::Filtered,
            ArchiveTool::BSArch,
            PluginIdentity::parse("MyMod"),
            true,
            None,
            Some(PathBuf::from(r"D:\Games\Fallout4")),
        );

        let config = request
            .to_project_config(PathBuf::from(r"C:\DetectedFallout4"), None, None)
            .unwrap();

        assert_eq!(config.build_mode, BuildMode::Filtered);
        assert_eq!(config.archive_tool, ArchiveTool::BSArch);
        assert_eq!(
            config.xedit_data_dir.as_deref(),
            Some(Path::new(r"D:\Games\Fallout4\Data"))
        );
    }

    #[test]
    fn prepare_creates_runnable_step_one_run() {
        let dir = tempdir().unwrap();
        let fo4 = dir.path().join("Fallout4");
        fs::create_dir_all(&fo4).unwrap();
        fs::write(fo4.join("CreationKit.exe"), b"").unwrap();
        fs::write(
            fo4.join("fallout4_test.ini"),
            "[CreationKit]\nBSHandleRefObjectPatch=true\n[CreationKit_Log]\nOutputFile=CK.log\n",
        )
        .unwrap();

        let tools = ToolPaths {
            fallout4_dir: Some(fo4.clone()),
            creation_kit: Some(fo4.join("CreationKit.exe")),
            ..ToolPaths::default()
        };
        let request = WorkflowRequest::new(
            BuildMode::Clean,
            ArchiveTool::Archive2,
            PluginIdentity::parse("MyMod"),
            true,
            None,
            None,
        );

        let run = WorkflowRun::prepare(&request, dir.path(), tools).unwrap();

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
        assert_eq!(run.config().ck_log_path, Some(fo4.join("CK.log")));
    }
}
