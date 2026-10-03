//! Compress PSG Operation.
//!
//! This module owns Step 4 domain flow (batch `:CompPSG`, V2.99 lines 295–301): the geometry
//! precondition every clean build shares, Xbox's skip, and Clean's compress-then-delete. There
//! is no Filtered branch: Workflow Plan membership keeps Filtered out of Step 4 entirely (296),
//! so the operation never re-checks for it.

use std::path::PathBuf;

use crate::config::{BuildMode, ProjectConfig, WorkflowStep};
use crate::error::{Error, Result};
use crate::run::WorkflowRun;
use crate::toolchain::ToolchainRequirements;

use super::{OperationPorts, WorkflowOperationDefinition};

pub(super) const DEFINITION: WorkflowOperationDefinition = WorkflowOperationDefinition::new(
    WorkflowStep::CompressPsg,
    ToolchainRequirements::creation_kit(),
    run,
);

/// Run the Step 4 Compress PSG Operation for a prepared Workflow Run.
///
/// Every clean build requires `Data\<base name> - Geometry.psg` and stops with
/// [`Error::MissingGeometryPsg`] before anything is spawned when it is absent (297). Xbox then
/// completes, keeping the `.psg` uncompressed (298). Clean compresses it with Creation Kit and
/// deletes it only once a fresh `.csg` exists (299–301), stopping with
/// [`Error::MissingCreationKitOutput`] when Creation Kit leaves none.
///
/// Also propagates [`Error::Io`] from the Creation Kit episode, the file deletes, and the
/// session-log append a Build Warning makes.
pub(super) fn run(run: &WorkflowRun, ports: &OperationPorts<'_>) -> Result<()> {
    let config = run.config();
    let psg = geometry_psg_path(config);

    if !ports.files.is_file(&psg) {
        return Err(Error::MissingGeometryPsg(config.plugin.base_name.clone()));
    }

    // The batch tests `NEQ "clean"` here (298). Filtered never reaches Step 4, so this is the
    // Xbox branch: its shipped geometry file is the uncompressed `.psg`, so it must be kept.
    // Console-only, and deliberately not a Build Warning — the skip is the intended outcome.
    if config.build_mode != BuildMode::Clean {
        tracing::info!(
            "Xbox build: keeping {} uncompressed, so CompressPSG is skipped.",
            geometry_psg_name(config)
        );
        return Ok(());
    }

    // A divergence that closes a data-loss path: the batch never clears an earlier run's
    // `.csg`, so a Creation Kit that silently wrote nothing would pass the output check on the
    // stale file, and the only `.psg` would then be deleted. Cleared before the spawn, the
    // `.csg` the check finds can only be this run's.
    let csg = geometry_csg_path(config);
    if ports.files.is_file(&csg) {
        ports.files.remove_file(&csg)?;
    }

    let ck_run = ports.ck.compress_psg(&config.plugin.file_name)?;

    if !ports.files.is_file(&csg) {
        return Err(ck_run.missing_output_error(&geometry_csg_name(config)));
    }

    // Only with the output confirmed: the batch checks it (471) before it calls a non-zero
    // exit harmless (472), so a run that produced nothing is never told it "seemed to finish".
    if let Some(warning) = ck_run.non_zero_exit_warning() {
        ports.warnings.raise(warning)?;
    }

    // Last, so a stop anywhere above — including a failed warning append — leaves the `.psg`
    // in place for the operator to resume from.
    ports.files.remove_file(&psg)?;

    Ok(())
}

/// `<base name> - Geometry.psg`, the uncompressed geometry Step 1 leaves in `Data`.
fn geometry_psg_name(config: &ProjectConfig) -> String {
    format!("{} - Geometry.psg", config.plugin.base_name)
}

/// `<base name> - Geometry.csg`, the compressed geometry `CompressPSG` writes into `Data`.
///
/// A bare name, because that is what the batch's missing-output line prints (`%~2`, 471).
fn geometry_csg_name(config: &ProjectConfig) -> String {
    format!("{} - Geometry.csg", config.plugin.base_name)
}

fn geometry_psg_path(config: &ProjectConfig) -> PathBuf {
    config.fo4edit_data_dir().join(geometry_psg_name(config))
}

fn geometry_csg_path(config: &ProjectConfig) -> PathBuf {
    config.fo4edit_data_dir().join(geometry_csg_name(config))
}
