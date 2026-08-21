//! Generate Precombines Operation.
//!
//! This module owns Step 1 domain flow: preconditions, CK action selection,
//! postconditions, and cleanup rules before any external-tool adapter details.

use crate::config::WorkflowStep;
use crate::error::{Error, Result};
use crate::run::WorkflowRun;
use crate::toolchain::ToolchainRequirements;

use super::precombine_workspace::PrecombineWorkspace;
use super::{OperationPorts, WorkflowOperationDefinition};

pub(super) const DEFINITION: WorkflowOperationDefinition = WorkflowOperationDefinition::new(
    WorkflowStep::GeneratePrecombines,
    ToolchainRequirements::creation_kit(),
    run,
);

/// Run the Step 1 Generate Precombines Operation for a prepared Workflow Run.
pub(super) fn run(run: &WorkflowRun, ports: &OperationPorts<'_>) -> Result<()> {
    let config = run.config();
    let workspace = PrecombineWorkspace::new(config, ports.files);
    maybe_clear_precombined_on_resume(run, ports, &workspace)?;

    workspace.prepare_for_generate()?;

    let ck_run = ports
        .ck
        .generate_precombined(&config.plugin.file_name, config.build_mode)?;

    // The log arrives as content, from the adapter that owns its lifecycle, rather than being
    // re-found by path: the Creation Kit episode deletes the stale log, reads this run's once,
    // and hands it here. Success criteria stay with the operation, per ADR-0001.
    workspace.validate_generated(ck_run.log.as_deref())?;

    Ok(())
}

fn maybe_clear_precombined_on_resume(
    run: &WorkflowRun,
    ports: &OperationPorts<'_>,
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
    if !ports.prompts.confirm_clear_precombined(&precombined)? {
        return Err(Error::Other(
            "precombined meshes not cleared - choose another resume step".into(),
        ));
    }

    workspace.clear_precombined_meshes()?;

    Ok(())
}
