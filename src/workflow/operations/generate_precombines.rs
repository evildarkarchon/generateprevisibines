//! Generate Precombines Operation.
//!
//! This module owns Step 1 domain flow: preconditions, CK action selection,
//! postconditions, and cleanup rules before any external-tool adapter details.

use crate::config::{BuildMode, WorkflowStep};
use crate::error::{Error, Result};
use crate::run::WorkflowRun;
use crate::tools::CkOperation;

use super::OperationAdapters;
use super::precombine_workspace::PrecombineWorkspace;

/// Run the Step 1 Generate Precombines Operation for a prepared Workflow Run.
pub(super) fn run<A: OperationAdapters>(run: &WorkflowRun, adapters: &A) -> Result<()> {
    let config = run.config();
    let workspace = PrecombineWorkspace::new(config);
    maybe_clear_precombined_on_resume(run, adapters, &workspace)?;

    workspace.prepare_for_generate()?;

    let qualifiers = precombine_qualifiers(config.build_mode);
    adapters.run_creation_kit(
        run,
        CkOperation::GeneratePrecombined,
        &config.plugin.file_name,
        qualifiers,
    )?;

    let ck_log = run
        .tool_context()
        .ck_log_path
        .clone()
        .ok_or_else(|| Error::Other("CK log path not configured".into()))?;

    workspace.validate_generated(&ck_log)?;

    Ok(())
}

fn maybe_clear_precombined_on_resume<A: OperationAdapters>(
    run: &WorkflowRun,
    adapters: &A,
    workspace: &PrecombineWorkspace<'_>,
) -> Result<()> {
    let config = run.config();
    let resume_step1 = config
        .resume_from
        .is_none_or(|s| s == WorkflowStep::GeneratePrecombines);

    if !resume_step1 {
        return Ok(());
    }

    if !workspace.has_precombined_meshes() {
        return Ok(());
    }

    if config.non_interactive {
        return Err(Error::PrecombinedMeshesExist);
    }

    let precombined = workspace.precombined_dir();
    if !adapters.confirm_clear_precombined(&precombined)? {
        return Err(Error::Other(
            "precombined meshes not cleared - choose another resume step".into(),
        ));
    }

    workspace.clear_precombined_meshes()?;

    Ok(())
}

/// CK `-GeneratePrecombined` qualifier string (batch lines 249-253).
fn precombine_qualifiers(build_mode: BuildMode) -> &'static str {
    if build_mode == BuildMode::Clean {
        "clean all"
    } else {
        "filtered all"
    }
}
