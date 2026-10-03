//! Merge Previs Operation.
//!
//! This module owns Step 7 domain flow (batch `:PreVisMerge`, V2.99 lines 322–328): the `.uvd`
//! and `Previs.esp` preconditions, the FO4Edit merge, and the Step's own judgement of the
//! script's log. The artifact paths come from the Previs Workspace, which Step 6 fills; the
//! FO4Edit episode owns everything about running the script, the two fatals every script run
//! shares included.

use crate::config::WorkflowStep;
use crate::error::{Error, Result};
use crate::run::WorkflowRun;
use crate::text::contains_ignore_ascii_case;
use crate::toolchain::ToolchainRequirements;
use crate::warning::BuildWarning;

use super::previs_workspace::PrevisWorkspace;
use super::{OperationPorts, WorkflowOperationDefinition};

/// The Step 7 log text that means the merge script reported a clean run (batch 327).
///
/// Held in the batch's shape — `Findstr /I /M /C:"Completed: No Errors."` — a case-insensitive
/// substring, period included. Its *absence* is what earns the warning: the inverse of Step 2,
/// which looks for `Error: `.
const MERGE_CLEAN_MARKER: &str = "Completed: No Errors.";

pub(super) const DEFINITION: WorkflowOperationDefinition = WorkflowOperationDefinition::new(
    WorkflowStep::MergePrevis,
    ToolchainRequirements::fo4edit(),
    run,
);

/// Run the Step 7 Merge Previs Operation for a prepared Workflow Run.
///
/// Stops with [`Error::NoVisibilityFiles`] when `Data\vis` holds no `.uvd` (323), and then with
/// [`Error::NoPrevisPlugin`] when `Data\Previs.esp` is absent (324) — both before FO4Edit is
/// launched or any delay runs, and in the batch's order. Then merges `Previs.esp` into the run's
/// plugin, and raises the "Merge Previs had errors" Build Warning when the script's log lacks
/// `Completed: No Errors.` (327–328): the run goes on, as the batch's does.
///
/// Also propagates every stop of the FO4Edit episode, including its two shared fatals, which
/// stop interactive and non-interactive runs alike, and [`Error::Io`] from the session-log
/// append a Build Warning makes.
pub(super) fn run(run: &WorkflowRun, ports: &OperationPorts<'_>) -> Result<()> {
    // Reached first, so a run that prepared no FO4Edit (a preparation bug) stops before the
    // workspace is even looked at, as Step 2 does.
    let fo4edit = ports.fo4edit()?;
    let config = run.config();
    let workspace = PrevisWorkspace::new(config, ports.files);

    // Absent outputs stop; a partial shortfall of `.uvd` files is the script's to log and this
    // step's warning to raise. Both checks reuse the workspace's own query and path rather
    // than adding a Step 7 rule to it.
    if !workspace.has_vis_uvd_files() {
        return Err(Error::NoVisibilityFiles);
    }
    if !ports.files.is_file(&workspace.previs_plugin_path()) {
        return Err(Error::NoPrevisPlugin);
    }

    let log = fo4edit.merge_previs(&config.plugin.file_name)?;

    // Step 7's criterion, kept here rather than in the episode (ADR-0001/0002), with Step 2's
    // polarity inverted: no clean-run marker, not an error marker, is what warns.
    if !contains_ignore_ascii_case(&log, MERGE_CLEAN_MARKER) {
        ports.warnings.raise(BuildWarning::MergePrevisHadErrors)?;
    }

    Ok(())
}
