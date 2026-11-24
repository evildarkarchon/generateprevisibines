//! Generate Precombines Operation.
//!
//! This module owns Step 1 domain flow: preconditions, CK action selection,
//! postconditions, and cleanup rules before any external-tool adapter details.

use crate::config::WorkflowStep;
use crate::error::{Error, Result};
use crate::run::WorkflowRun;
use crate::toolchain::ToolchainRequirements;

use super::precombine_workspace::PrecombineWorkspace;
use super::{Confirmation, OperationPorts, WorkflowOperationDefinition};

pub(super) const DEFINITION: WorkflowOperationDefinition = WorkflowOperationDefinition::new(
    WorkflowStep::GeneratePrecombines,
    ToolchainRequirements::creation_kit(),
    run,
);

/// Run the Step 1 Generate Precombines Operation for a prepared Workflow Run.
pub(super) fn run(run: &WorkflowRun, ports: &OperationPorts<'_>) -> Result<()> {
    // Reached first, so a run that prepared no Creation Kit (a preparation bug) stops before
    // anything in the workspace is touched.
    let ck = ports.ck()?;
    let config = run.config();
    let workspace = PrecombineWorkspace::new(config, ports.files);
    clear_existing_outputs(run, ports, &workspace)?;

    workspace.prepare_for_generate()?;

    let ck_run = ck.generate_precombined(&config.plugin.file_name, config.build_mode)?;

    // The log arrives as content, from the adapter that owns its lifecycle, rather than being
    // re-found by path: the Creation Kit episode deletes the stale log, reads this run's once,
    // and hands it here. Success criteria stay with the operation, per ADR-0001.
    workspace.validate_generated(ck_run.log.as_deref())?;

    // Only now, with every postcondition passed: the batch checks the output (471) before it
    // calls a non-zero exit harmless (472), so a run that produced nothing is never told it
    // "seemed to finish".
    if let Some(warning) = ck_run.non_zero_exit_warning() {
        ports.warnings.raise(warning)?;
    }

    Ok(())
}

/// Find earlier build output that conflicts with Step 1, and offer to clear all of it at once.
///
/// Every conflict (precombined meshes, the plugin archive, `Datais`) is found before
/// anything is asked or deleted. A non-interactive run stops on the first one in the batch's
/// check order and deletes nothing. An interactive run gets one Yes/No for the whole set:
/// **Y** deletes all of it, and **N** stops without deleting anything.
///
/// Deliberate deviation from the batch. Its fresh-run `:Precomb` stops on each of these
/// (253–260), and its resume-only `:RePrecomb` (237–242) offers to clear only the meshes,
/// deleting them before it has looked at the archive or `Datais`. Intake lets an existing
/// archive through to here only on an interactive run, so the prompt is what decides it.
fn clear_existing_outputs(
    run: &WorkflowRun,
    ports: &OperationPorts<'_>,
    workspace: &PrecombineWorkspace<'_>,
) -> Result<()> {
    let existing = workspace.find_existing_outputs();
    let Some(stop) = existing.stop_error() else {
        return Ok(());
    };

    if run.config().non_interactive {
        return Err(stop);
    }

    if !ports
        .prompts
        .confirm(&Confirmation::ClearExistingOutputs(existing.paths()))?
    {
        return Err(Error::Other(
            "existing build output not cleared - choose another resume step".into(),
        ));
    }

    workspace.clear_existing_outputs(&existing)
}
