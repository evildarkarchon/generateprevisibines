//! Precombine / CK output checks shared by Step 1 preamble and post-run validation.

use std::path::{Path, PathBuf};

use crate::config::{BuildMode, ProjectConfig};
use crate::error::{Error, Result};

const HANDLE_ARRAY_MARKER: &str = "DEFAULT: OUT OF HANDLE ARRAY ENTRIES";

/// CK `-GeneratePrecombined` qualifier string (batch lines 249–253).
#[must_use]
pub fn precombine_qualifiers(build_mode: BuildMode) -> &'static str {
    if build_mode == BuildMode::Clean {
        "clean all"
    } else {
        "filtered all"
    }
}

/// Whether any `.nif` exists under `meshes/precombined` (recursive).
#[must_use]
pub fn has_precombined_nifs(precombined_dir: &Path) -> bool {
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
        } else if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("nif")) {
            return Some(path);
        }
    }
    None
}

/// Whether any `.uvd` exists in `Data/vis`.
#[must_use]
pub fn has_vis_uvd_files(vis_dir: &Path) -> bool {
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
#[must_use]
pub fn ck_log_has_handle_array_error(log_path: &Path) -> bool {
    let Ok(contents) = std::fs::read_to_string(log_path) else {
        return false;
    };
    contents.contains(HANDLE_ARRAY_MARKER)
}

/// Preamble checks before launching CK for Step 1 (`:Precomb`).
pub fn run_precomb_preamble(config: &ProjectConfig) -> Result<()> {
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
pub fn run_post_precomb_checks(config: &ProjectConfig, ck_log_path: &Path) -> Result<()> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn qualifiers_match_build_mode() {
        assert_eq!(precombine_qualifiers(BuildMode::Clean), "clean all");
        assert_eq!(precombine_qualifiers(BuildMode::Filtered), "filtered all");
        assert_eq!(precombine_qualifiers(BuildMode::Xbox), "filtered all");
    }

    #[test]
    fn detects_handle_array_marker() {
        let dir = tempdir().unwrap();
        let log = dir.path().join("CK.log");
        fs::write(&log, "ok\n").unwrap();
        assert!(!ck_log_has_handle_array_error(&log));

        fs::write(&log, format!("line\n{HANDLE_ARRAY_MARKER}\n")).unwrap();
        assert!(ck_log_has_handle_array_error(&log));
    }

    #[test]
    fn finds_nested_precombined_nif() {
        let dir = tempdir().unwrap();
        let mesh = dir.path().join("sub").join("test.nif");
        fs::create_dir_all(mesh.parent().unwrap()).unwrap();
        fs::write(mesh, b"").unwrap();
        assert!(has_precombined_nifs(dir.path()));
    }

    #[test]
    fn detects_uvd_in_vis() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("cell.uvd"), b"").unwrap();
        assert!(has_vis_uvd_files(dir.path()));
    }
}
