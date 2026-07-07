//! Precombine Workspace artifact checks and cleanup for Step 1.

use std::path::{Path, PathBuf};

use crate::config::{BuildMode, ProjectConfig};
use crate::error::{Error, Result};

const HANDLE_ARRAY_MARKER: &str = "DEFAULT: OUT OF HANDLE ARRAY ENTRIES";

/// Artifact space prepared and validated by the Generate Precombines Operation.
#[derive(Debug)]
pub(super) struct PrecombineWorkspace<'a> {
    config: &'a ProjectConfig,
}

impl<'a> PrecombineWorkspace<'a> {
    /// Create a workspace view over the precombine artifacts for a prepared project config.
    #[must_use]
    pub(super) const fn new(config: &'a ProjectConfig) -> Self {
        Self { config }
    }

    /// Return the precombined mesh directory used for prompt and cleanup decisions.
    #[must_use]
    pub(super) fn precombined_dir(&self) -> PathBuf {
        self.config.precombined_dir()
    }

    /// Whether any `.nif` exists under `meshes/precombined` recursively.
    #[must_use]
    pub(super) fn has_precombined_meshes(&self) -> bool {
        find_first_precombined_nif(&self.config.precombined_dir()).is_some()
    }

    /// Delete the precombined mesh directory when it exists.
    pub(super) fn clear_precombined_meshes(&self) -> Result<()> {
        let precombined = self.config.precombined_dir();
        if precombined.is_dir() {
            std::fs::remove_dir_all(precombined)?;
        }

        Ok(())
    }

    /// Run artifact preconditions and cleanup before launching Creation Kit.
    pub(super) fn prepare_for_generate(&self) -> Result<()> {
        let data = self.config.fo4edit_data_dir();

        if self.config.plugin_archive_path().is_file() {
            return Err(Error::PluginAlreadyHasArchive);
        }

        if self.has_precombined_meshes() {
            return Err(Error::PrecombinedMeshesExist);
        }

        if self.has_vis_uvd_files() {
            return Err(Error::VisUvdFilesExist);
        }

        let combined = data.join("CombinedObjects.esp");
        if combined.is_file() {
            std::fs::remove_file(combined)?;
        }

        let psg = self.geometry_psg_path();
        if psg.is_file() {
            std::fs::remove_file(psg)?;
        }

        Ok(())
    }

    /// Validate Creation Kit outputs after the Generate Precombines command finishes.
    pub(super) fn validate_generated(&self, ck_log_path: &Path) -> Result<()> {
        if !self.combined_objects_path().is_file() {
            return Err(Error::MissingCombinedObjects);
        }

        if self.config.build_mode == BuildMode::Clean && !self.geometry_psg_path().is_file() {
            return Err(Error::MissingGeometryPsg(
                self.config.plugin.base_name.clone(),
            ));
        }

        if !self.has_precombined_meshes() {
            return Err(Error::NoPrecombinedMeshes);
        }

        if ck_log_has_handle_array_error(ck_log_path) {
            return Err(Error::HandleArrayLogError);
        }

        Ok(())
    }

    fn combined_objects_path(&self) -> PathBuf {
        self.config.fo4edit_data_dir().join("CombinedObjects.esp")
    }

    fn geometry_psg_path(&self) -> PathBuf {
        self.config
            .fo4edit_data_dir()
            .join(format!("{} - Geometry.psg", self.config.plugin.base_name))
    }

    fn has_vis_uvd_files(&self) -> bool {
        let Ok(entries) = std::fs::read_dir(self.config.vis_dir()) else {
            return false;
        };
        entries.flatten().any(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("uvd"))
        })
    }
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

fn ck_log_has_handle_array_error(log_path: &Path) -> bool {
    let Ok(contents) = std::fs::read_to_string(log_path) else {
        return false;
    };
    contents.contains(HANDLE_ARRAY_MARKER)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::{TempDir, tempdir};

    use super::*;
    use crate::config::{ArchiveTool, PluginIdentity};

    fn workspace(mode: BuildMode) -> (TempDir, ProjectConfig) {
        let dir = tempdir().unwrap();
        let config = ProjectConfig {
            build_mode: mode,
            archive_tool: ArchiveTool::Archive2,
            fallout4_dir: dir.path().join("Fallout4"),
            plugin: PluginIdentity::parse("MyMod"),
            non_interactive: true,
            resume_from: None,
            fo4edit_path: None,
            xedit_data_dir: Some(dir.path().join("Data")),
            ck_log_path: None,
        };

        (dir, config)
    }

    fn create_generated_outputs(config: &ProjectConfig, include_psg: bool) {
        let precombined_mesh = config.precombined_dir().join("cell").join("mesh.nif");
        fs::create_dir_all(precombined_mesh.parent().unwrap()).unwrap();
        fs::write(precombined_mesh, b"nif").unwrap();

        fs::create_dir_all(config.fo4edit_data_dir()).unwrap();
        fs::write(
            config.fo4edit_data_dir().join("CombinedObjects.esp"),
            b"esp",
        )
        .unwrap();

        if include_psg {
            fs::write(
                config.fo4edit_data_dir().join("MyMod - Geometry.psg"),
                b"psg",
            )
            .unwrap();
        }
    }

    #[test]
    fn detects_precombined_meshes_recursively() {
        let (_dir, config) = workspace(BuildMode::Filtered);
        let workspace = PrecombineWorkspace::new(&config);
        assert!(!workspace.has_precombined_meshes());

        let mesh = config
            .precombined_dir()
            .join("a")
            .join("b")
            .join("mesh.NIF");
        fs::create_dir_all(mesh.parent().unwrap()).unwrap();
        fs::write(mesh, b"nif").unwrap();

        assert!(workspace.has_precombined_meshes());
    }

    #[test]
    fn clear_precombined_meshes_removes_directory_when_present() {
        let (_dir, config) = workspace(BuildMode::Filtered);
        let workspace = PrecombineWorkspace::new(&config);
        let mesh = config.precombined_dir().join("old").join("mesh.nif");
        fs::create_dir_all(mesh.parent().unwrap()).unwrap();
        fs::write(&mesh, b"nif").unwrap();

        workspace.clear_precombined_meshes().unwrap();

        assert!(!config.precombined_dir().exists());
    }

    #[test]
    fn prepare_rejects_existing_archive() {
        let (_dir, config) = workspace(BuildMode::Filtered);
        let workspace = PrecombineWorkspace::new(&config);
        fs::create_dir_all(config.fo4edit_data_dir()).unwrap();
        fs::write(config.plugin_archive_path(), b"ba2").unwrap();

        let err = workspace.prepare_for_generate().unwrap_err();

        assert!(matches!(err, Error::PluginAlreadyHasArchive));
    }

    #[test]
    fn prepare_rejects_vis_uvd_files() {
        let (_dir, config) = workspace(BuildMode::Filtered);
        let workspace = PrecombineWorkspace::new(&config);
        fs::create_dir_all(config.vis_dir()).unwrap();
        fs::write(config.vis_dir().join("cell.UVD"), b"uvd").unwrap();

        let err = workspace.prepare_for_generate().unwrap_err();

        assert!(matches!(err, Error::VisUvdFilesExist));
    }

    #[test]
    fn prepare_removes_stale_combined_objects_and_psg() {
        let (_dir, config) = workspace(BuildMode::Clean);
        let workspace = PrecombineWorkspace::new(&config);
        fs::create_dir_all(config.fo4edit_data_dir()).unwrap();
        let combined = config.fo4edit_data_dir().join("CombinedObjects.esp");
        let psg = config.fo4edit_data_dir().join("MyMod - Geometry.psg");
        fs::write(&combined, b"old").unwrap();
        fs::write(&psg, b"old").unwrap();

        workspace.prepare_for_generate().unwrap();

        assert!(!combined.exists());
        assert!(!psg.exists());
    }

    #[test]
    fn validate_generated_reports_missing_combined_objects() {
        let (_dir, config) = workspace(BuildMode::Filtered);
        let workspace = PrecombineWorkspace::new(&config);
        let ck_log = config.fo4edit_data_dir().join("CK.log");

        let err = workspace.validate_generated(&ck_log).unwrap_err();

        assert!(matches!(err, Error::MissingCombinedObjects));
    }

    #[test]
    fn validate_generated_requires_psg_for_clean_mode() {
        let (_dir, config) = workspace(BuildMode::Clean);
        create_generated_outputs(&config, false);
        let workspace = PrecombineWorkspace::new(&config);
        let ck_log = config.fo4edit_data_dir().join("CK.log");

        let err = workspace.validate_generated(&ck_log).unwrap_err();

        assert!(matches!(err, Error::MissingGeometryPsg(name) if name == "MyMod"));
    }

    #[test]
    fn validate_generated_accepts_missing_psg_for_filtered_mode() {
        let (_dir, config) = workspace(BuildMode::Filtered);
        create_generated_outputs(&config, false);
        let workspace = PrecombineWorkspace::new(&config);
        let ck_log = config.fo4edit_data_dir().join("CK.log");

        workspace.validate_generated(&ck_log).unwrap();
    }

    #[test]
    fn validate_generated_reports_no_precombined_meshes() {
        let (_dir, config) = workspace(BuildMode::Filtered);
        fs::create_dir_all(config.fo4edit_data_dir()).unwrap();
        fs::write(
            config.fo4edit_data_dir().join("CombinedObjects.esp"),
            b"esp",
        )
        .unwrap();
        let workspace = PrecombineWorkspace::new(&config);
        let ck_log = config.fo4edit_data_dir().join("CK.log");

        let err = workspace.validate_generated(&ck_log).unwrap_err();

        assert!(matches!(err, Error::NoPrecombinedMeshes));
    }

    #[test]
    fn validate_generated_reports_handle_array_marker() {
        let (_dir, config) = workspace(BuildMode::Filtered);
        create_generated_outputs(&config, false);
        let ck_log = config.fo4edit_data_dir().join("CK.log");
        fs::write(&ck_log, b"DEFAULT: OUT OF HANDLE ARRAY ENTRIES\n").unwrap();
        let workspace = PrecombineWorkspace::new(&config);

        let err = workspace.validate_generated(&ck_log).unwrap_err();

        assert!(matches!(err, Error::HandleArrayLogError));
    }
}
