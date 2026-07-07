//! Generate Precombines Operation.
//!
//! This module owns Step 1 domain flow: preconditions, CK action selection,
//! postconditions, and cleanup rules before any external-tool adapter details.

use std::path::{Path, PathBuf};

use crate::config::{BuildMode, WorkflowStep};
use crate::error::{Error, Result};
use crate::run::WorkflowRun;
use crate::tools::CkOperation;

use super::OperationAdapters;

const HANDLE_ARRAY_MARKER: &str = "DEFAULT: OUT OF HANDLE ARRAY ENTRIES";

/// Run the Step 1 Generate Precombines Operation for a prepared Workflow Run.
pub(super) fn run<A: OperationAdapters>(run: &WorkflowRun, adapters: &A) -> Result<()> {
    let config = run.config();
    maybe_clear_precombined_on_resume(run, adapters)?;

    run_precomb_preamble(run)?;

    let qualifiers = precombine_qualifiers(config.build_mode);
    adapters.run_creation_kit(
        run,
        CkOperation::GeneratePrecombined,
        &config.plugin.file_name,
        qualifiers,
    )?;

    let combined = config.fo4edit_data_dir().join("CombinedObjects.esp");
    if !combined.is_file() {
        return Err(Error::MissingCombinedObjects);
    }

    let ck_log = run
        .tool_context()
        .ck_log_path
        .clone()
        .ok_or_else(|| Error::Other("CK log path not configured".into()))?;

    run_post_precomb_checks(run, &ck_log)?;

    Ok(())
}

fn maybe_clear_precombined_on_resume<A: OperationAdapters>(
    run: &WorkflowRun,
    adapters: &A,
) -> Result<()> {
    let config = run.config();
    let resume_step1 = config
        .resume_from
        .is_none_or(|s| s == WorkflowStep::GeneratePrecombines);

    if !resume_step1 {
        return Ok(());
    }

    let precombined = config.precombined_dir();
    if !has_precombined_nifs(&precombined) {
        return Ok(());
    }

    if config.non_interactive {
        return Err(Error::PrecombinedMeshesExist);
    }

    if !adapters.confirm_clear_precombined(&precombined)? {
        return Err(Error::Other(
            "precombined meshes not cleared - choose another resume step".into(),
        ));
    }

    if precombined.is_dir() {
        std::fs::remove_dir_all(precombined)?;
    }

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

/// Whether any `.nif` exists under `meshes/precombined` (recursive).
fn has_precombined_nifs(precombined_dir: &Path) -> bool {
    find_first_precombined_nif(precombined_dir).is_some()
}

fn find_first_precombined_nif(dir: &Path) -> Option<PathBuf> {
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_first_precombined_nif(&path) {
                return Some(found);
            }
        } else if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("nif"))
        {
            return Some(path);
        }
    }
    None
}

/// Whether any `.uvd` exists in `Data/vis`.
fn has_vis_uvd_files(vis_dir: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(vis_dir) else {
        return false;
    };
    entries.flatten().any(|entry| {
        entry
            .path()
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("uvd"))
    })
}

/// Scan CK log for handle-array exhaustion (batch `findstr` on line 258).
fn ck_log_has_handle_array_error(log_path: &Path) -> bool {
    let Ok(contents) = std::fs::read_to_string(log_path) else {
        return false;
    };
    contents.contains(HANDLE_ARRAY_MARKER)
}

/// Preamble checks before launching CK for Step 1 (`:Precomb`).
fn run_precomb_preamble(run: &WorkflowRun) -> Result<()> {
    let config = run.config();
    let data = config.fo4edit_data_dir();

    if config.plugin_archive_path().is_file() {
        return Err(Error::PluginAlreadyHasArchive);
    }

    if has_precombined_nifs(&config.precombined_dir()) {
        return Err(Error::PrecombinedMeshesExist);
    }

    if has_vis_uvd_files(&config.vis_dir()) {
        return Err(Error::VisUvdFilesExist);
    }

    let combined = data.join("CombinedObjects.esp");
    if combined.is_file() {
        std::fs::remove_file(combined)?;
    }

    let psg = data.join(format!("{} - Geometry.psg", config.plugin.base_name));
    if psg.is_file() {
        std::fs::remove_file(psg)?;
    }

    Ok(())
}

/// Post-CK validation (`:Precomb2`).
fn run_post_precomb_checks(run: &WorkflowRun, ck_log_path: &Path) -> Result<()> {
    let config = run.config();
    let data = config.fo4edit_data_dir();

    if config.build_mode == BuildMode::Clean {
        let psg = data.join(format!("{} - Geometry.psg", config.plugin.base_name));
        if !psg.is_file() {
            return Err(Error::MissingGeometryPsg(config.plugin.base_name.clone()));
        }
    }

    if !has_precombined_nifs(&config.precombined_dir()) {
        return Err(Error::NoPrecombinedMeshes);
    }

    if ck_log_has_handle_array_error(ck_log_path) {
        return Err(Error::HandleArrayLogError);
    }

    Ok(())
}
