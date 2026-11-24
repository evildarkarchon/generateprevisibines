//! Precombine Workspace artifact checks and cleanup for Step 1, and the artifacts Step 2
//! requires before it merges them.

use std::path::PathBuf;

use crate::config::ProjectConfig;
use crate::error::{Error, Result};
use crate::files::FileSpace;

use super::previs_workspace::PrevisWorkspace;

/// The Creation Kit log text that means the precombine ran out of Reference Handles.
///
/// Held in the batch's own shape — `Findstr /I /M /C:"OUT OF HANDLE ARRAY ENTRIES"`
/// (`GeneratePrevisibines.bat:276`): a case-insensitive substring, carrying no prefix. The
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

    /// Observe every earlier build output that would conflict with a fresh Step 1, deleting
    /// nothing: precombined meshes, the plugin archive, and previs in `Data\vis`.
    ///
    /// All three are found before anything is offered or deleted. The operator then answers
    /// once for the whole set, and a run that cannot proceed never loses part of it first.
    #[must_use]
    pub(super) fn find_existing_outputs(&self) -> ExistingOutputs {
        let archive = self.config.plugin_archive_path();
        let vis = PrevisWorkspace::new(self.config, self.files);
        ExistingOutputs {
            precombined: self
                .has_precombined_meshes()
                .then(|| self.config.precombined_dir()),
            archive: self.files.is_file(&archive).then_some(archive),
            vis: vis.has_vis_uvd_files().then(|| vis.vis_dir()),
        }
    }

    /// Delete everything `existing` names: the precombined mesh tree, the plugin archive,
    /// and the whole `Data\vis` tree, in that order.
    ///
    /// Only call this after the operator has consented to the whole set. A failed delete
    /// stops at once, and anything already deleted stays deleted.
    pub(super) fn clear_existing_outputs(&self, existing: &ExistingOutputs) -> Result<()> {
        if existing.precombined.is_some() {
            self.clear_precombined_meshes()?;
        }
        if let Some(archive) = &existing.archive {
            self.files.remove_file(archive)?;
        }
        if existing.vis.is_some() {
            PrevisWorkspace::new(self.config, self.files).clear_vis()?;
        }
        Ok(())
    }

    /// Remove stale `CombinedObjects.esp` and `- Geometry.psg` before launching Creation Kit
    /// (batch 263–264).
    ///
    /// Run this only once [`Self::find_existing_outputs`] reports nothing, or after that set
    /// has been cleared, so a run that stops on a conflict leaves these files alone.
    pub(super) fn prepare_for_generate(&self) -> Result<()> {
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
    ///
    /// A missing `<plugin> - Geometry.psg` is fatal for every clean build — Clean and, since
    /// V2.99, Xbox — because both ran `clean all` (V2.99 batch line 270).
    pub(super) fn validate_generated(&self, ck_log: Option<&str>) -> Result<()> {
        if !self.files.is_file(&self.combined_objects_path()) {
            return Err(Error::MissingCombinedObjects);
        }

        if self.config.build_mode.is_clean_build() && !self.files.is_file(&self.geometry_psg_path())
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

    /// Require what the Step 2 merge reads, before FO4Edit is launched.
    ///
    /// Stops with [`Error::NoPrecombinedMeshesFound`] when `meshes\precombined` holds no `.nif`
    /// (batch 281), then with [`Error::MissingCombinedObjects`] when `Data\CombinedObjects.esp`
    /// is absent. The second is a divergence: the batch never checks it, and learns of it only
    /// through FO4Edit's "missing modules" fatal, after about 50 seconds of MO2 delays.
    pub(super) fn validate_ready_to_merge(&self) -> Result<()> {
        if !self.has_precombined_meshes() {
            return Err(Error::NoPrecombinedMeshesFound);
        }

        if !self.files.is_file(&self.combined_objects_path()) {
            return Err(Error::MissingCombinedObjects);
        }

        Ok(())
    }

    /// `Data\CombinedObjects.esp`, which Step 1 writes, Step 2 merges and Finish removes.
    pub(super) fn combined_objects_path(&self) -> PathBuf {
        self.config.fo4edit_data_dir().join("CombinedObjects.esp")
    }

    fn geometry_psg_path(&self) -> PathBuf {
        self.config
            .fo4edit_data_dir()
            .join(format!("{} - Geometry.psg", self.config.plugin.base_name))
    }
}

/// Earlier build output that conflicts with a fresh Step 1, as found by
/// [`PrecombineWorkspace::find_existing_outputs`]. Each field holds the path to show and
/// clear, or `None` when that output is absent.
///
/// `Data\vis` is detected through the Previs Workspace's own definition, so Step 1 and Step 6
/// can never disagree about whether previs exists.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct ExistingOutputs {
    precombined: Option<PathBuf>,
    archive: Option<PathBuf>,
    vis: Option<PathBuf>,
}

impl ExistingOutputs {
    /// The paths found, in the order the batch checks them (253, 257, 258), for the operator
    /// to review before consenting.
    #[must_use]
    pub(super) fn paths(&self) -> Vec<PathBuf> {
        [&self.precombined, &self.archive, &self.vis]
            .into_iter()
            .flatten()
            .cloned()
            .collect()
    }

    /// The stop for a run that cannot ask, using the first conflict in the batch's check
    /// order: precombined meshes (253), then the archive (257), then `Data\vis` (258).
    ///
    /// Returns `None` when nothing conflicts.
    #[must_use]
    pub(super) fn stop_error(&self) -> Option<Error> {
        if self.precombined.is_some() {
            Some(Error::PrecombinedMeshesExist)
        } else if self.archive.is_some() {
            Some(Error::PluginAlreadyHasArchive)
        } else if self.vis.is_some() {
            Some(Error::VisUvdFilesExist)
        } else {
            None
        }
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
    use crate::config::{ArchiveTool, BuildMode, PluginIdentity};
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

    fn vis_dir() -> PathBuf {
        PathBuf::from("Data").join("vis")
    }

    fn vis_uvd() -> PathBuf {
        vis_dir().join("cell.uvd")
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
    /// `include_psg` distinguishes a clean-build run (Clean or Xbox), which emits the geometry
    /// PSG, from a Filtered-mode run, which does not.
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
        space.add_file(precombined_mesh());
        let workspace = PrecombineWorkspace::new(&config, &space);

        // Observed through what Step 1 shows the operator and clears.
        assert_eq!(
            workspace.find_existing_outputs().paths(),
            vec![precombined_dir()]
        );
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
    fn a_workspace_with_no_prior_artifacts_has_no_existing_outputs() {
        let config = project_config(BuildMode::Clean);
        let space = InMemoryFileSpace::new();
        let workspace = PrecombineWorkspace::new(&config, &space);

        let existing = workspace.find_existing_outputs();

        assert_eq!(existing.paths(), Vec::<PathBuf>::new());
        assert!(existing.stop_error().is_none());
        workspace.prepare_for_generate().unwrap();
    }

    #[test]
    fn finds_every_existing_output_in_batch_order_and_touches_nothing() {
        let config = project_config(BuildMode::Clean);
        let space = InMemoryFileSpace::new();
        space.add_file(vis_uvd());
        space.add_file(plugin_archive());
        space.add_file(precombined_mesh());
        space.add_file(combined_objects());
        let workspace = PrecombineWorkspace::new(&config, &space);

        let existing = workspace.find_existing_outputs();

        assert_eq!(
            existing.paths(),
            vec![precombined_dir(), plugin_archive(), vis_dir()]
        );
        assert!(space.is_file(&precombined_mesh()));
        assert!(space.is_file(&plugin_archive()));
        assert!(space.is_file(&vis_uvd()));
        assert!(space.is_file(&combined_objects()));
    }

    /// A run that cannot ask stops on the first conflict in the batch's own check order:
    /// meshes (253), then the archive (257), then `Data\vis` (258).
    #[test]
    fn the_stop_error_follows_the_batch_check_order() {
        let config = project_config(BuildMode::Clean);
        let space = InMemoryFileSpace::new();
        let workspace = PrecombineWorkspace::new(&config, &space);

        space.add_file(vis_uvd());
        assert!(matches!(
            workspace.find_existing_outputs().stop_error(),
            Some(Error::VisUvdFilesExist)
        ));

        space.add_file(plugin_archive());
        assert!(matches!(
            workspace.find_existing_outputs().stop_error(),
            Some(Error::PluginAlreadyHasArchive)
        ));

        space.add_file(precombined_mesh());
        assert!(matches!(
            workspace.find_existing_outputs().stop_error(),
            Some(Error::PrecombinedMeshesExist)
        ));
    }

    #[test]
    fn clearing_existing_outputs_removes_exactly_what_was_found() {
        let config = project_config(BuildMode::Clean);
        let space = InMemoryFileSpace::new();
        space.add_file(precombined_mesh());
        space.add_file(plugin_archive());
        space.add_file(vis_uvd());
        space.add_file(combined_objects());
        let workspace = PrecombineWorkspace::new(&config, &space);
        let existing = workspace.find_existing_outputs();

        workspace.clear_existing_outputs(&existing).unwrap();

        assert_eq!(
            workspace.find_existing_outputs(),
            ExistingOutputs::default()
        );
        // Stale working files are prepare's to remove, not part of the confirmed set.
        assert!(space.is_file(&combined_objects()));
    }

    /// Only what was found is deleted, so an absent archive is never a failed `remove_file`.
    #[test]
    fn clearing_skips_outputs_that_were_not_found() {
        let config = project_config(BuildMode::Clean);
        let space = InMemoryFileSpace::new();
        space.add_file(precombined_mesh());
        let workspace = PrecombineWorkspace::new(&config, &space);
        let existing = workspace.find_existing_outputs();

        workspace.clear_existing_outputs(&existing).unwrap();

        assert!(!space.is_file(&precombined_mesh()));
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

    /// Clean and V2.99 Xbox both run `clean all`, so both require the PSG (batch line 270).
    #[test]
    fn validate_reports_a_missing_geometry_psg_before_missing_meshes_in_clean_builds() {
        for mode in [BuildMode::Clean, BuildMode::Xbox] {
            let config = project_config(mode);
            let space = InMemoryFileSpace::new();
            space.add_file(combined_objects());
            let workspace = PrecombineWorkspace::new(&config, &space);

            let err = workspace
                .validate_generated(Some(&failing_ck_log()))
                .unwrap_err();

            assert!(
                matches!(&err, Error::MissingGeometryPsg(name) if name == "MyMod"),
                "mode: {mode:?}, err: {err:?}"
            );
        }
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
    fn a_merge_needs_a_precombined_mesh_before_anything_else() {
        let config = project_config(BuildMode::Clean);
        let space = InMemoryFileSpace::new();
        // No mesh and no `CombinedObjects.esp`: the batch's own check (281) speaks first.
        let workspace = PrecombineWorkspace::new(&config, &space);

        let err = workspace.validate_ready_to_merge().unwrap_err();

        assert!(matches!(err, Error::NoPrecombinedMeshesFound), "{err:?}");
    }

    #[test]
    fn a_merge_needs_combined_objects() {
        let config = project_config(BuildMode::Clean);
        let space = InMemoryFileSpace::new();
        space.add_file(precombined_mesh());
        let workspace = PrecombineWorkspace::new(&config, &space);

        let err = workspace.validate_ready_to_merge().unwrap_err();

        assert!(matches!(err, Error::MissingCombinedObjects), "{err:?}");
    }

    /// Step 2 asks only for what it merges: no geometry PSG, in any mode.
    #[test]
    fn a_merge_is_ready_with_a_mesh_and_combined_objects() {
        let config = project_config(BuildMode::Clean);
        let space = InMemoryFileSpace::new();
        record_generated_outputs(&space, false);
        let workspace = PrecombineWorkspace::new(&config, &space);

        workspace.validate_ready_to_merge().unwrap();
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
