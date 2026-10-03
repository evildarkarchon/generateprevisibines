//! Merge CombinedObjects Operation.
//!
//! This module owns Step 2 domain flow (batch `:PrecombMerge`, V2.99 lines 279–285): the
//! precombined-mesh and `CombinedObjects.esp` preconditions, the FO4Edit merge, and the
//! Step's own judgement of the script's log. The FO4Edit episode owns everything about running
//! the script, the two fatals every script run shares included.

use crate::config::WorkflowStep;
use crate::error::Result;
use crate::run::WorkflowRun;
use crate::text::contains_ignore_ascii_case;
use crate::toolchain::ToolchainRequirements;
use crate::warning::BuildWarning;

use super::precombine_workspace::PrecombineWorkspace;
use super::{OperationPorts, WorkflowOperationDefinition};

/// The Step 2 log text that means the merge script reported an error (batch 284).
///
/// Held in the batch's shape — `Findstr /I /M /C:"Error: "` — a case-insensitive substring with
/// its trailing space, which is what keeps a successful log's `Completed: No Errors.` from
/// matching.
const MERGE_ERROR_MARKER: &str = "Error: ";

pub(super) const DEFINITION: WorkflowOperationDefinition = WorkflowOperationDefinition::new(
    WorkflowStep::MergePrecombineObjects,
    ToolchainRequirements::fo4edit(),
    run,
);

/// Run the Step 2 Merge CombinedObjects Operation for a prepared Workflow Run.
///
/// Stops with [`crate::error::Error::NoPrecombinedMeshesFound`] when `meshes\precombined` holds
/// no `.nif` (281), and with [`crate::error::Error::MissingCombinedObjects`] when
/// `Data\CombinedObjects.esp` is absent — both before FO4Edit is launched or any delay runs.
/// Then merges `CombinedObjects.esp` into the run's plugin, and raises the "Merge Precombines
/// had errors" Build Warning when the script's log contains `Error: ` (284–285): the run goes
/// on, as the batch's does.
///
/// Also propagates every stop of the FO4Edit episode, including its two shared fatals, which
/// stop interactive and non-interactive runs alike, and [`crate::error::Error::Io`] from the
/// session-log append a Build Warning makes.
pub(super) fn run(run: &WorkflowRun, ports: &OperationPorts<'_>) -> Result<()> {
    // Reached first, so a run that prepared no FO4Edit (a preparation bug) stops before the
    // workspace is even looked at, as Steps 1 and 6 do for Creation Kit.
    let fo4edit = ports.fo4edit()?;
    let config = run.config();

    PrecombineWorkspace::new(config, ports.files).validate_ready_to_merge()?;

    let log = fo4edit.merge_combined_objects(&config.plugin.file_name)?;

    // Step 2's criterion, kept here rather than in the episode (ADR-0001/0002): Step 7 judges
    // its own log with the opposite polarity, so the episode only returns the text.
    if contains_ignore_ascii_case(&log, MERGE_ERROR_MARKER) {
        ports
            .warnings
            .raise(BuildWarning::MergePrecombinesHadErrors)?;
    }

    Ok(())
}
