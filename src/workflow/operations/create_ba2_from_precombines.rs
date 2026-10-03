//! Create BA2 from Precombines Operation.
//!
//! This module owns Step 3 domain flow (batch `:ArcPrecomb`, V2.99 lines 287–292): deciding
//! whether there is anything to archive. The Archive episode owns everything about building the
//! archive — the tool, its command line, the work folder, the swap into `Data` and the removal of
//! the loose meshes — so nothing here names Archive2 or BSArch.

use crate::config::WorkflowStep;
use crate::error::{Error, Result};
use crate::run::WorkflowRun;
use crate::toolchain::ToolchainRequirements;

use super::precombine_workspace::PrecombineWorkspace;
use super::{OperationPorts, WorkflowOperationDefinition};

pub(super) const DEFINITION: WorkflowOperationDefinition = WorkflowOperationDefinition::new(
    WorkflowStep::CreateBa2FromPrecombines,
    ToolchainRequirements::archive(),
    run,
);

/// Run the Step 3 Create BA2 from Precombines Operation for a prepared Workflow Run.
///
/// Packs the loose meshes under `meshes\precombined` into the Plugin Archive whenever any `.nif`
/// is there, even over an existing archive. With no loose meshes it completes with nothing to
/// do when the Plugin Archive already exists, and otherwise stops with
/// [`Error::NoPrecombinesToArchive`] before any tool runs.
///
/// Also propagates every stop of the Archive episode, which stops interactive and
/// non-interactive runs alike, and [`Error::ArchiveNotPrepared`] when the run prepared no
/// archive tool.
pub(super) fn run(run: &WorkflowRun, ports: &OperationPorts<'_>) -> Result<()> {
    // Reached first, so a run that prepared no archive tool (a preparation bug) stops before the
    // workspace is even looked at, as the Creation Kit and FO4Edit steps do.
    let archive = ports.archive()?;
    let config = run.config();
    let archive_name = config.plugin.archive_name();

    // Packed even when the archive already exists, for parity with the batch: a resume at 3
    // repacks over an earlier attempt's archive (design item 2 on #29). The episode replaces the
    // old archive only once the new one has been built and checked, so a failed repack keeps it.
    if PrecombineWorkspace::new(config, ports.files).has_precombined_meshes() {
        return archive.archive_precombines(&archive_name);
    }

    // The batch skips silently whenever there are no loose meshes (289). The port skips only
    // when an earlier attempt already packed them, and says so on the console: nothing was
    // degraded, so it is not a Build Warning.
    if ports.files.is_file(&config.plugin_archive_path()) {
        tracing::info!("No loose precombined meshes; {archive_name} already holds them");
        return Ok(());
    }

    Err(Error::NoPrecombinesToArchive { name: archive_name })
}
