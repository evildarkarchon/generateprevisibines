//! Precombine Workspace artifact checks and cleanup for Step 1.

use std::path::PathBuf;

use crate::config::{BuildMode, ProjectConfig};
use crate::error::{Error, Result};
use crate::files::FileSpace;

/// The Creation Kit log text that means the precombine ran out of Reference Handles.
///
/// Held in the batch's own shape — `Findstr /I /M /C:"OUT OF HANDLE ARRAY ENTRIES"`
/// (`GeneratePrevisibines.bat:270`): a case-insensitive substring, carrying no prefix. The
/// Creation Kit usually emits it under `DEFAULT: `, but the batch never required that, so
/// neither do we; requiring it would let a differently-prefixed line pass a run the batch
/// would have failed.
const HANDLE_ARRAY_MARKER: &str = "OUT OF HANDLE ARRAY ENTRIES";

/// Artifact space prepared and validated by the Generate Precombines Operation.
///
/// Every artifact path is derived here from the resolved project configuration; only raw
/// filesystem access goes through the [`FileSpace`] seam.
#[derive(Debug)]
pub(super) struct PrecombineWorkspace<'a> {
    config: &'a ProjectConfig,
    files: &'a dyn FileSpace,
}

impl<'a> PrecombineWorkspace<'a> {
    /// Create a workspace view over the precombine artifacts for a prepared project config.
    ///
    /// `files` is the space the artifacts are observed in and cleaned up through; it is held
    /// for the lifetime of the operation because the answers legitimately change as the
    /// resume clear, the stale-artifact cleanup and Creation Kit itself mutate it.
    #[must_use]
    pub(super) const fn new(config: &'a ProjectConfig, files: &'a dyn FileSpace) -> Self {
        Self { config, files }
    }

    /// Return the precombined mesh directory used for prompt and cleanup decisions.
    #[must_use]
    pub(super) fn precombined_dir(&self) -> PathBuf {
        self.config.precombined_dir()
    }

    /// Whether any `.nif` exists under `meshes/precombined` recursively.
    #[must_use]
    pub(super) fn has_precombined_meshes(&self) -> bool {
        self.files
            .find_first_file_with_extension(&self.config.precombined_dir(), "nif")
            .is_some()
    }

    /// Delete the precombined mesh directory when it exists.
    pub(super) fn clear_precombined_meshes(&self) -> Result<()> {
        // Tree removal is idempotent, so an absent directory needs no separate check.
        self.files.remove_dir_all(&self.config.precombined_dir())
    }

    /// Run artifact preconditions and cleanup before launching Creation Kit.
    pub(super) fn prepare_for_generate(&self) -> Result<()> {
        if self.files.is_file(&self.config.plugin_archive_path()) {
            return Err(Error::PluginAlreadyHasArchive);
        }

        if self.has_precombined_meshes() {
            return Err(Error::PrecombinedMeshesExist);
        }

        if self.has_vis_uvd_files() {
            return Err(Error::VisUvdFilesExist);
        }

        let combined = self.combined_objects_path();
        if self.files.is_file(&combined) {
            self.files.remove_file(&combined)?;
        }

        let psg = self.geometry_psg_path();
        if self.files.is_file(&psg) {
            self.files.remove_file(&psg)?;
        }

        Ok(())
    }

    /// Validate Creation Kit outputs after the Generate Precombines command finishes.
    ///
    /// `ck_log` is the log *content* the Creation Kit adapter read back, not a path: that
    /// adapter owns the log's lifecycle, and `None` is its "Creation Kit wrote no log" state.
    pub(super) fn validate_generated(&self, ck_log: Option<&str>) -> Result<()> {
        if !self.files.is_file(&self.combined_objects_path()) {
            return Err(Error::MissingCombinedObjects);
        }

        if self.config.build_mode == BuildMode::Clean
            && !self.files.is_file(&self.geometry_psg_path())
        {
            return Err(Error::MissingGeometryPsg(
                self.config.plugin.base_name.clone(),
            ));
        }

        if !self.has_precombined_meshes() {
            return Err(Error::NoPrecombinedMeshes);
        }

        if ck_log.is_some_and(has_handle_array_error) {
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
        self.files
            .find_first_file_with_extension(&self.config.vis_dir(), "uvd")
            .is_some()
    }
}

/// Whether Creation Kit's log reports the handle-array exhaustion marker.
///
/// An absent log means "no handle-array error": the marker's absence is the only thing that
/// clears the run, and a log Creation Kit never wrote cannot contain it. A log that *is* there
/// but cannot be read never reaches here — the Creation Kit adapter raises that as an error
/// rather than folding it into "no log", precisely so it cannot pass as a clear run.
fn has_handle_array_error(contents: &str) -> bool {
    // `Findstr /I` is case-insensitive, and the marker is pure ASCII, so folding both
    // sides the same way is enough — no Unicode case mapping is in play on either side.
    // The marker is folded too rather than assumed uppercase: a constant respelled in
    // mixed case would otherwise silently stop matching anything.
    contents
        .to_ascii_uppercase()
        .contains(&HANDLE_ARRAY_MARKER.to_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ArchiveTool, PluginIdentity};
    use crate::files::InMemoryFileSpace;

    /// A resolved project configuration rooted at plain relative paths.
    ///
    /// Nothing here needs to exist on disk: the rules only ever ask the space about paths.
    /// The filesystem-shaped behaviour these tests used to carry — recursive descent,
    /// case-insensitive extension matching, a directory named like an artifact, a non-UTF-8
    /// log — is asserted against the standard-library adapter in the `files` module, where
    /// it actually lives.
    fn project_config(mode: BuildMode) -> ProjectConfig {
        ProjectConfig {
            build_mode: mode,
            archive_tool: ArchiveTool::Archive2,
            fallout4_dir: PathBuf::from("Fallout4"),
            data_dir: PathBuf::from("Data"),
            plugin: PluginIdentity::parse("MyMod"),
            non_interactive: true,
            resume_from: None,
        }
    }

    /// The `meshes\precombined` layout, spelled out rather than asked for.
    ///
    /// The literals in these helpers are the point: a test that derived its paths from the
    /// same accessors it exercises would agree with a changed derivation instead of
    /// catching it, so the layout and the `<plugin base name> - Geometry.psg` naming are
    /// pinned here by hand.
    fn precombined_dir() -> PathBuf {
        PathBuf::from("Data").join("meshes").join("precombined")
    }

    fn precombined_mesh() -> PathBuf {
        precombined_dir().join("cell").join("mesh.nif")
    }

    fn combined_objects() -> PathBuf {
        PathBuf::from("Data").join("CombinedObjects.esp")
    }

    fn geometry_psg() -> PathBuf {
        PathBuf::from("Data").join("MyMod - Geometry.psg")
    }

    fn plugin_archive() -> PathBuf {
        PathBuf::from("Data").join("MyMod - Main.ba2")
    }

    fn vis_uvd() -> PathBuf {
        PathBuf::from("Data").join("vis").join("cell.uvd")
    }

    /// A Creation Kit log carrying the handle-array marker, for the ordering cases below.
    ///
    /// Every check that must win over the marker is asserted against a log that *does* carry
    /// it, so an ordering regression shows up as the wrong error rather than as no error.
    fn failing_ck_log() -> String {
        format!("{HANDLE_ARRAY_MARKER}\n")
    }

    /// Record what a successful Creation Kit run leaves behind.
    ///
    /// `include_psg` distinguishes a Clean-mode run, which emits the geometry PSG, from a
    /// Filtered-mode run, which does not.
    fn record_generated_outputs(space: &InMemoryFileSpace, include_psg: bool) {
        space.add_file(precombined_mesh());
        space.add_file(combined_objects());

        if include_psg {
            space.add_file(geometry_psg());
        }
    }

    #[test]
    fn precombined_dir_is_the_meshes_precombined_layout_under_the_data_dir() {
        let config = project_config(BuildMode::Filtered);
        let space = InMemoryFileSpace::new();
        let workspace = PrecombineWorkspace::new(&config, &space);

        assert_eq!(workspace.precombined_dir(), precombined_dir());
    }

    #[test]
    fn detects_precombined_meshes_beneath_the_precombined_dir() {
        let config = project_config(BuildMode::Filtered);
        let space = InMemoryFileSpace::new();
        let workspace = PrecombineWorkspace::new(&config, &space);
        assert!(!workspace.has_precombined_meshes());

        space.add_file(precombined_mesh());

        assert!(workspace.has_precombined_meshes());
    }

    #[test]
    fn clear_precombined_meshes_empties_the_precombined_dir() {
        let config = project_config(BuildMode::Filtered);
        let space = InMemoryFileSpace::new();
        let workspace = PrecombineWorkspace::new(&config, &space);
        // Two meshes at different depths: clearing takes the whole tree, not one file.
        let shallow = precombined_dir().join("mesh.nif");
        space.add_file(precombined_mesh());
        space.add_file(&shallow);

        workspace.clear_precombined_meshes().unwrap();

        assert!(!space.is_file(&precombined_mesh()));
        assert!(!space.is_file(&shallow));
        assert!(!workspace.has_precombined_meshes());
        // Tree removal is idempotent, which is why the caller needs no existence check.
        workspace.clear_precombined_meshes().unwrap();
    }

    #[test]
    fn prepare_accepts_a_workspace_with_no_prior_artifacts() {
        let config = project_config(BuildMode::Clean);
        let space = InMemoryFileSpace::new();
        let workspace = PrecombineWorkspace::new(&config, &space);

        workspace.prepare_for_generate().unwrap();
    }

    #[test]
    fn prepare_rejects_an_existing_plugin_archive_before_any_other_check() {
        let config = project_config(BuildMode::Clean);
        let space = InMemoryFileSpace::new();
        space.add_file(plugin_archive());
        space.add_file(precombined_mesh());
        space.add_file(vis_uvd());
        space.add_file(combined_objects());
        let workspace = PrecombineWorkspace::new(&config, &space);

        let err = workspace.prepare_for_generate().unwrap_err();

        assert!(matches!(err, Error::PluginAlreadyHasArchive));
        // The archive check wins, so the stale-artifact cleanup never ran.
        assert!(space.is_file(&combined_objects()));
    }

    #[test]
    fn prepare_rejects_existing_precombined_meshes_before_vis_uvd_files() {
        let config = project_config(BuildMode::Clean);
        let space = InMemoryFileSpace::new();
        space.add_file(precombined_mesh());
        space.add_file(vis_uvd());
        let workspace = PrecombineWorkspace::new(&config, &space);

        let err = workspace.prepare_for_generate().unwrap_err();

        assert!(matches!(err, Error::PrecombinedMeshesExist));
    }

    #[test]
    fn prepare_rejects_vis_uvd_files_before_cleaning_stale_artifacts() {
        let config = project_config(BuildMode::Clean);
        let space = InMemoryFileSpace::new();
        space.add_file(vis_uvd());
        space.add_file(combined_objects());
        space.add_file(geometry_psg());
        let workspace = PrecombineWorkspace::new(&config, &space);

        let err = workspace.prepare_for_generate().unwrap_err();

        assert!(matches!(err, Error::VisUvdFilesExist));
        assert!(space.is_file(&combined_objects()));
        assert!(space.is_file(&geometry_psg()));
    }

    #[test]
    fn prepare_removes_stale_combined_objects_and_geometry_psg() {
        let config = project_config(BuildMode::Clean);
        let space = InMemoryFileSpace::new();
        space.add_file(combined_objects());
        space.add_file(geometry_psg());
        let workspace = PrecombineWorkspace::new(&config, &space);

        workspace.prepare_for_generate().unwrap();

        assert!(!space.is_file(&combined_objects()));
        assert!(!space.is_file(&geometry_psg()));
    }

    #[test]
    fn validate_reports_missing_combined_objects_before_every_other_failure() {
        let config = project_config(BuildMode::Clean);
        let space = InMemoryFileSpace::new();
        let workspace = PrecombineWorkspace::new(&config, &space);

        let err = workspace
            .validate_generated(Some(&failing_ck_log()))
            .unwrap_err();

        assert!(matches!(err, Error::MissingCombinedObjects));
    }

    #[test]
    fn validate_reports_a_missing_geometry_psg_before_missing_meshes_in_clean_mode() {
        let config = project_config(BuildMode::Clean);
        let space = InMemoryFileSpace::new();
        space.add_file(combined_objects());
        let workspace = PrecombineWorkspace::new(&config, &space);

        let err = workspace
            .validate_generated(Some(&failing_ck_log()))
            .unwrap_err();

        assert!(matches!(err, Error::MissingGeometryPsg(name) if name == "MyMod"));
    }

    #[test]
    fn validate_accepts_a_missing_geometry_psg_in_filtered_mode() {
        let config = project_config(BuildMode::Filtered);
        let space = InMemoryFileSpace::new();
        record_generated_outputs(&space, false);
        let workspace = PrecombineWorkspace::new(&config, &space);

        workspace.validate_generated(None).unwrap();
    }

    #[test]
    fn validate_reports_no_precombined_meshes_before_the_handle_array_marker() {
        let config = project_config(BuildMode::Clean);
        let space = InMemoryFileSpace::new();
        space.add_file(combined_objects());
        space.add_file(geometry_psg());
        let workspace = PrecombineWorkspace::new(&config, &space);

        let err = workspace
            .validate_generated(Some(&failing_ck_log()))
            .unwrap_err();

        assert!(matches!(err, Error::NoPrecombinedMeshes));
    }

    /// Drive `validate_generated` over a Filtered-mode run whose only variable is the log.
    ///
    /// The marker cases differ in nothing but the log text, so the outputs and the mode are
    /// factored out here to keep each case down to the line it is actually pinning.
    fn validate_with_ck_log(contents: &str) -> Result<()> {
        let config = project_config(BuildMode::Filtered);
        let space = InMemoryFileSpace::new();
        record_generated_outputs(&space, false);
        let workspace = PrecombineWorkspace::new(&config, &space);

        workspace.validate_generated(Some(contents))
    }

    // The four marker cases below spell the Creation Kit text out by hand rather than reusing
    // `HANDLE_ARRAY_MARKER`: a test built from the constant would follow a narrowed constant
    // instead of catching it, which is exactly the divergence from `Findstr /I /C:` these
    // cases exist to pin.

    #[test]
    fn validate_reports_the_handle_array_marker_under_the_default_prefix() {
        let err = validate_with_ck_log(
            "Masterfile: Fallout4.esm\nDEFAULT: OUT OF HANDLE ARRAY ENTRIES\n",
        )
        .unwrap_err();

        assert!(matches!(err, Error::HandleArrayLogError));
    }

    #[test]
    fn validate_reports_the_handle_array_marker_without_a_prefix() {
        let err = validate_with_ck_log("Masterfile: Fallout4.esm\nOUT OF HANDLE ARRAY ENTRIES\n")
            .unwrap_err();

        assert!(matches!(err, Error::HandleArrayLogError));
    }

    #[test]
    fn validate_reports_the_handle_array_marker_in_lowercase() {
        // Case is the only thing varied against the prefixed case above, so a failure here
        // points at the comparison rather than at the prefix.
        let err = validate_with_ck_log(
            "Masterfile: Fallout4.esm\ndefault: out of handle array entries\n",
        )
        .unwrap_err();

        assert!(matches!(err, Error::HandleArrayLogError));
    }

    #[test]
    fn validate_accepts_a_ck_log_without_the_handle_array_marker() {
        validate_with_ck_log("Masterfile: Fallout4.esm\nDEFAULT: exporting cell\n").unwrap();
    }

    #[test]
    fn validate_accepts_complete_clean_mode_outputs_with_a_quiet_log() {
        let config = project_config(BuildMode::Clean);
        let space = InMemoryFileSpace::new();
        record_generated_outputs(&space, true);
        let workspace = PrecombineWorkspace::new(&config, &space);

        workspace
            .validate_generated(Some("Masterfile: Fallout4.esm\n"))
            .unwrap();
    }

    #[test]
    fn validate_accepts_outputs_when_creation_kit_wrote_no_log() {
        let config = project_config(BuildMode::Clean);
        let space = InMemoryFileSpace::new();
        record_generated_outputs(&space, true);
        // The batch's "Unable to find log" state. An absent log means "no handle-array error":
        // the marker's absence is the only thing that clears the run, and a log Creation Kit
        // never wrote cannot contain it. A log that exists but cannot be read never gets here
        // — `CreationKitOps` raises that as an error instead of reporting `None`.
        let workspace = PrecombineWorkspace::new(&config, &space);

        workspace.validate_generated(None).unwrap();
    }
}
