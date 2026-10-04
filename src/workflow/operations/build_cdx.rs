//! Build CDX Operation.
//!
//! This module owns Step 5 domain flow (batch `:BldCDX`, V2.99 lines 304–307). It is the only
//! Creation Kit step with no pre-checks of its own: a resume at 5 enters at `:BldCDX` (231) and
//! skips Step 4's `.psg` check, so nothing here looks for the geometry file. There is no
//! Filtered branch: Workflow Plan membership keeps Filtered out of Step 5 entirely (305).

use std::path::PathBuf;

use crate::config::{ProjectConfig, WorkflowStep};
use crate::error::Result;
use crate::run::WorkflowRun;
use crate::toolchain::ToolchainRequirements;

use super::{OperationPorts, WorkflowOperationDefinition};

pub(super) const DEFINITION: WorkflowOperationDefinition = WorkflowOperationDefinition::new(
    WorkflowStep::BuildCdx,
    ToolchainRequirements::creation_kit(),
    run,
);

/// Run the Step 5 Build CDX Operation for a prepared Workflow Run.
///
/// Clears any `Data\<base name>.cdx` an earlier run left, runs `BuildCDX`, and stops with
/// [`Error::MissingCreationKitOutput`] when Creation Kit leaves no `.cdx` (471). A non-zero
/// exit that still produced the `.cdx` completes with a Build Warning (472).
///
/// Also propagates [`Error::Io`] from the Creation Kit episode, the stale-output delete, and
/// the session-log append a Build Warning makes.
///
/// [`Error::MissingCreationKitOutput`]: crate::error::Error::MissingCreationKitOutput
/// [`Error::Io`]: crate::error::Error::Io
pub(super) fn run(run: &WorkflowRun, ports: &OperationPorts<'_>) -> Result<()> {
    // Reached first, so a run that prepared no Creation Kit (a preparation bug) stops before
    // anything in the workspace is touched.
    let ck = ports.ck()?;
    let config = run.config();

    // A divergence, for the same reason as Step 4's stale `.csg`: the batch never clears an
    // earlier run's `.cdx`, so a Creation Kit that silently wrote nothing would pass the
    // output check on the stale file. Cleared before the spawn, the `.cdx` the check finds can
    // only be this run's.
    let cdx = cdx_path(config);
    if ports.files.is_file(&cdx) {
        ports.files.remove_file(&cdx)?;
    }

    let ck_run = ck.build_cdx(&config.plugin.file_name)?;

    if !ports.files.is_file(&cdx) {
        return Err(ck_run.missing_output_error(&cdx_name(config)));
    }

    // Only with the output confirmed: the batch checks it (471) before it calls a non-zero
    // exit harmless (472), so a run that produced nothing is never told it "seemed to finish".
    if let Some(warning) = ck_run.non_zero_exit_warning() {
        ports.warnings.raise(warning)?;
    }

    Ok(())
}

/// `<base name>.cdx`, the index `BuildCDX` writes into `Data`.
///
/// A bare name, because that is what the batch's missing-output line prints (`%~2`, 471).
pub(super) fn cdx_name(config: &ProjectConfig) -> String {
    format!("{}.cdx", config.plugin.base_name)
}

fn cdx_path(config: &ProjectConfig) -> PathBuf {
    config.fo4edit_data_dir().join(cdx_name(config))
}
