//! Add Previs to Archive Operation.
//!
//! This module owns Step 8 domain flow (batch `:ArcPreVis`, V2.99 lines 331–335, and the
//! archive check at 424): deciding whether there is previs to add and an archive to add it to.
//! The Archive episode owns everything about rebuilding the archive — the tool, the extract or
//! unpack, the work folder, the swap into `Data` and the removal of the loose `vis` — so nothing
//! here names Archive2 or BSArch.

use crate::config::WorkflowStep;
use crate::error::{Error, Result};
use crate::run::WorkflowRun;
use crate::toolchain::ToolchainRequirements;
use crate::warning::BuildWarning;

use super::previs_workspace::PrevisWorkspace;
use super::{OperationPorts, WorkflowOperationDefinition};

pub(super) const DEFINITION: WorkflowOperationDefinition = WorkflowOperationDefinition::new(
    WorkflowStep::AddPrevisToArchive,
    ToolchainRequirements::archive(),
    run,
);

/// Run the Step 8 Add Previs to Archive Operation for a prepared Workflow Run.
///
/// With no `.uvd` under `Data\vis` it raises [`BuildWarning::NoVisibilityFilesToArchive`] and
/// completes without running any tool, whether or not the Plugin Archive exists. Otherwise it
/// stops with [`Error::PluginArchiveMissing`] before any tool runs when the Plugin Archive is
/// absent, and rebuilds it as its precombines plus the new previs when it is present.
///
/// Also propagates every stop of the Archive episode, which stops interactive and
/// non-interactive runs alike, [`Error::ArchiveNotPrepared`] when the run prepared no archive
/// tool, and [`Error::Io`] when the warning cannot be appended to the session log.
pub(super) fn run(run: &WorkflowRun, ports: &OperationPorts<'_>) -> Result<()> {
    // Reached first, so a run that prepared no archive tool (a preparation bug) stops before the
    // workspace is even looked at, as every other step does.
    let archive = ports.archive()?;
    let config = run.config();
    let archive_name = config.plugin.archive_name();

    // Checked before the archive, as in the batch (333 precedes 424): no `.uvd` is the normal
    // state after a Step 8 that succeeded, so a resume at 8 over a finished build warns and
    // completes rather than stopping on an archive question it never needed to ask.
    if !PrevisWorkspace::new(config, ports.files).has_vis_uvd_files() {
        return ports
            .warnings
            .raise(BuildWarning::NoVisibilityFilesToArchive);
    }

    // The batch falls back to `:ArchiveOnly` here (424), packing `vis` alone. That drops the
    // precombined meshes previs refers to, so the port stops instead (design item 8 on #29).
    if !ports.files.is_file(&config.plugin_archive_path()) {
        return Err(Error::PluginArchiveMissing { name: archive_name });
    }

    archive.add_previs(&archive_name)
}
