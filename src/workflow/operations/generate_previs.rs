//! Generate Previs Operation.
//!
//! This module owns Step 6 domain flow (batch `:PreVis`/`:RePreVis`, V2.99 lines 244–249 and
//! 309–320): the `Data\vis` precondition, the stale-`Previs.esp` delete, the `GeneratePreVisData`
//! run, and its postconditions. The artifact rules themselves live in the Previs Workspace,
//! which Steps 1, 7 and 8 share.

use crate::config::WorkflowStep;
use crate::error::{Error, Result};
use crate::run::WorkflowRun;
use crate::toolchain::ToolchainRequirements;

use super::previs_workspace::PrevisWorkspace;
use super::{Confirmation, OperationPorts, WorkflowOperationDefinition};

pub(super) const DEFINITION: WorkflowOperationDefinition = WorkflowOperationDefinition::new(
    WorkflowStep::GeneratePrevis,
    ToolchainRequirements::creation_kit(),
    run,
);

/// Run the Step 6 Generate Previs Operation for a prepared Workflow Run.
///
/// Stops with [`Error::VisUvdFilesExist`] before anything is deleted or spawned when `Data\vis`
/// is not empty, unless an interactive run resumed at Step 6 and the operator agrees to clear
/// it. Then deletes a stale `Previs.esp`, runs `GeneratePreVisData` with `clean all` for every
/// Build Mode, and stops with [`Error::MissingCreationKitOutput`] when Creation Kit leaves no
/// `Previs.esp` (471). Raises the visibility-task Build Warning (320) and then the non-zero-exit
/// Build Warning (472) only once `Previs.esp` is confirmed.
///
/// Also propagates [`Error::Io`] from the Creation Kit episode, the deletes, and the
/// session-log append a Build Warning makes, and [`Error::Prompt`] from the console.
pub(super) fn run(run: &WorkflowRun, ports: &OperationPorts<'_>) -> Result<()> {
    // Reached first, so a run that prepared no Creation Kit (a preparation bug) stops before
    // anything in the workspace is touched.
    let ck = ports.ck()?;
    let config = run.config();
    let workspace = PrevisWorkspace::new(config, ports.files);

    ensure_vis_is_empty(run, ports, &workspace)?;

    // The batch deletes it here too (315): cleared before the spawn, the `Previs.esp` the
    // output check finds can only be this run's, so a silent Creation Kit cannot pass on an
    // earlier run's plugin and have Step 7 merge it.
    workspace.remove_stale_previs_plugin()?;

    let ck_run = ck.generate_previs_data(&config.plugin.file_name)?;

    let visibility_warning = workspace.validate_generated(ck_run.log.as_deref(), |file| {
        ck_run.missing_output_error(file)
    })?;

    // Both only with `Previs.esp` confirmed, so a run that produced nothing is never told it
    // "seemed to finish". Visibility first, then the exit (docs/episodes.md, Step 6 note);
    // the batch prints them the other way round only because its exit warning is raised inside
    // `:RunCK` (472) before the call-site scan (319–320).
    if let Some(warning) = visibility_warning {
        ports.warnings.raise(warning)?;
    }
    if let Some(warning) = ck_run.non_zero_exit_warning() {
        ports.warnings.raise(warning)?;
    }

    Ok(())
}

/// Require an empty `Data\vis`, offering to clear it only on an interactive resume at Step 6.
///
/// Only `:RePreVis` — the `:GetStep` target for a resume at 6 (244–249) — asks. Every other
/// way into Step 6 is `:PreVis`, which hard-stops on a non-empty `vis` (311–313): a fresh run
/// reaching 6, and a resume at 1–5, including a Filtered resume at 4 or 5 that is forwarded to
/// `:PreVis` (296, 305). Hence `== Some(GeneratePrevis)`, and deliberately not Step 1's
/// `is_none_or`: a fresh run must never be offered to have its previs silently overwritten.
fn ensure_vis_is_empty(
    run: &WorkflowRun,
    ports: &OperationPorts<'_>,
    workspace: &PrevisWorkspace<'_>,
) -> Result<()> {
    if !workspace.has_vis_uvd_files() {
        return Ok(());
    }

    let config = run.config();
    let resumed_at_step_six = config.resume_from == Some(WorkflowStep::GeneratePrevis);

    // Non-interactive runs never block on a prompt; they stop for the operator to clear `vis`.
    if !resumed_at_step_six || config.non_interactive {
        return Err(Error::VisUvdFilesExist);
    }

    if !ports
        .prompts
        .confirm(&Confirmation::ClearVis(workspace.vis_dir()))?
    {
        return Err(Error::Other(
            "previs directory not cleared - choose another resume step".into(),
        ));
    }

    workspace.clear_vis()
}
