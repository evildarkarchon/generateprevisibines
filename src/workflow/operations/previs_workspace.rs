//! Previs Workspace: `Data\vis\*.uvd` and `Data\Previs.esp`.
//!
//! The previs artifacts are read by Steps 1, 6, 7 and 8, so the rules for them live here
//! rather than in any one step: Step 1 refuses to start over a non-empty `vis`, Step 6 clears
//! and validates both, Step 7 requires them, and Step 8 looks for a `.uvd` before archiving.

use std::path::PathBuf;

use crate::config::ProjectConfig;
use crate::error::{Error, Result};
use crate::files::FileSpace;
use crate::warning::BuildWarning;

/// The plugin `GeneratePreVisData` writes into `Data`, under a fixed name (batch 315–317).
///
/// A bare name, because that is what the batch's missing-output line prints (`%~2`, 471).
const PREVIS_PLUGIN: &str = "Previs.esp";

/// The Creation Kit log text that means at least one cluster's `.uvd` was not built.
///
/// Held in the batch's own shape — `Findstr /I /M /C:"ERROR: visibility task did not
/// complete."` (`GeneratePrevisibines.bat:319`): a case-insensitive substring anywhere on a
/// line, following the same convention as the precombine handle-array marker.
const VISIBILITY_TASK_MARKER: &str = "ERROR: visibility task did not complete.";

/// Artifact space cleared and validated by the Generate Previs Operation, required by the
/// Merge Previs Operation, and checked for `.uvd` files by the Add Previs to Archive Operation.
///
/// Every artifact path is derived here from the resolved project configuration; only raw
/// filesystem access goes through the [`FileSpace`] seam.
#[derive(Debug)]
pub(super) struct PrevisWorkspace<'a> {
    config: &'a ProjectConfig,
    files: &'a dyn FileSpace,
}

impl<'a> PrevisWorkspace<'a> {
    /// Create a workspace view over the previs artifacts for a prepared project config.
    ///
    /// `files` is held for the workspace's lifetime because its answers legitimately change as
    /// the resume clear, the stale-plugin delete and Creation Kit itself mutate the space.
    #[must_use]
    pub(super) const fn new(config: &'a ProjectConfig, files: &'a dyn FileSpace) -> Self {
        Self { config, files }
    }

    /// `Data\vis`, the directory Creation Kit writes each cluster's `.uvd` into.
    #[must_use]
    pub(super) fn vis_dir(&self) -> PathBuf {
        self.config.vis_dir()
    }

    /// Whether any `.uvd` exists under `Data\vis`, recursively (`dir /a-d /s /b`, 245/258/311).
    #[must_use]
    pub(super) fn has_vis_uvd_files(&self) -> bool {
        self.files
            .find_first_file_with_extension(&self.vis_dir(), "uvd")
            .is_some()
    }

    /// Delete the whole `Data\vis` directory, as `RD /S /Q` does at batch 248.
    ///
    /// The whole tree rather than only its `.uvd` files, because that is what the operator
    /// consented to clearing. Tree removal is idempotent, so an absent directory needs no check.
    pub(super) fn clear_vis(&self) -> Result<()> {
        self.files.remove_dir_all(&self.vis_dir())
    }

    /// Delete a `Data\Previs.esp` an earlier run left (batch 315).
    ///
    /// Cleared before the spawn so that the `Previs.esp` [`Self::validate_generated`] finds can
    /// only be this run's.
    pub(super) fn remove_stale_previs_plugin(&self) -> Result<()> {
        let previs = self.previs_plugin_path();
        if self.files.is_file(&previs) {
            self.files.remove_file(&previs)?;
        }

        Ok(())
    }

    /// Validate what `GeneratePreVisData` left behind, returning the visibility-task Build
    /// Warning when the Creation Kit log reports one.
    ///
    /// `ck_log` is the log *content* the Creation Kit episode read back; `None` is its
    /// "Creation Kit wrote no log" state, which the batch skips the scan for (318), so it earns
    /// no warning. `missing_output` builds the stop for a missing `Previs.esp` from the bare
    /// file name: the episode supplies it, because the message names the batch verb and only
    /// the episode knows that.
    ///
    /// Returns the error `missing_output` builds when there is no `Previs.esp`. The warning is
    /// returned rather than raised so the operation decides the order it is raised in.
    pub(super) fn validate_generated(
        &self,
        ck_log: Option<&str>,
        missing_output: impl FnOnce(&str) -> Error,
    ) -> Result<Option<BuildWarning>> {
        if !self.files.is_file(&self.previs_plugin_path()) {
            return Err(missing_output(PREVIS_PLUGIN));
        }

        Ok(ck_log
            .is_some_and(has_visibility_task_error)
            .then_some(BuildWarning::VisibilityTaskIncomplete))
    }

    /// `Data\Previs.esp`, the plugin `GeneratePreVisData` writes and Step 7 merges.
    #[must_use]
    pub(super) fn previs_plugin_path(&self) -> PathBuf {
        self.config.fo4edit_data_dir().join(PREVIS_PLUGIN)
    }
}

/// Whether Creation Kit's log reports the incomplete-visibility-task marker.
fn has_visibility_task_error(contents: &str) -> bool {
    // `Findstr /I` is case-insensitive and the marker is pure ASCII, so folding both sides the
    // same way is enough. The marker is folded too rather than assumed to be in one case: a
    // constant respelled in mixed case would otherwise silently stop matching anything.
    contents
        .to_ascii_uppercase()
        .contains(&VISIBILITY_TASK_MARKER.to_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ArchiveTool, BuildMode, PluginIdentity};
    use crate::files::InMemoryFileSpace;

    /// A resolved project configuration rooted at plain relative paths; nothing needs to exist
    /// on disk, because the rules only ever ask the space about paths.
    fn project_config() -> ProjectConfig {
        ProjectConfig {
            build_mode: BuildMode::Clean,
            archive_tool: ArchiveTool::Archive2,
            fallout4_dir: PathBuf::from("Fallout4"),
            data_dir: PathBuf::from("Data"),
            plugin: PluginIdentity::parse("MyMod"),
            non_interactive: true,
            resume_from: None,
        }
    }

    // The layout is spelled out by hand rather than asked for: a test that derived its paths
    // from the accessors it exercises would agree with a changed derivation instead of
    // catching it.

    fn vis_dir() -> PathBuf {
        PathBuf::from("Data").join("vis")
    }

    fn previs_plugin() -> PathBuf {
        PathBuf::from("Data").join("Previs.esp")
    }

    /// A stand-in for the episode's missing-output error, so these tests need no Creation Kit.
    fn missing(file: &str) -> Error {
        Error::Other(format!("missing {file}"))
    }

    #[test]
    fn vis_dir_is_the_vis_layout_under_the_data_dir() {
        let config = project_config();
        let space = InMemoryFileSpace::new();

        assert_eq!(PrevisWorkspace::new(&config, &space).vis_dir(), vis_dir());
    }

    #[test]
    fn detects_uvd_files_at_any_depth_beneath_vis() {
        let config = project_config();
        let space = InMemoryFileSpace::new();
        let workspace = PrevisWorkspace::new(&config, &space);
        // A non-`.uvd` file is not previs, so it does not count.
        space.add_file(vis_dir().join("notes.txt"));
        assert!(!workspace.has_vis_uvd_files());

        space.add_file(vis_dir().join("cell").join("cluster.uvd"));

        assert!(workspace.has_vis_uvd_files());
    }

    #[test]
    fn clear_vis_removes_the_whole_vis_directory() {
        let config = project_config();
        let space = InMemoryFileSpace::new();
        let workspace = PrevisWorkspace::new(&config, &space);
        let nested = vis_dir().join("cell").join("cluster.uvd");
        let other = vis_dir().join("notes.txt");
        space.add_file(&nested);
        space.add_file(&other);

        workspace.clear_vis().unwrap();

        // `RD /S /Q` takes everything, not only the `.uvd` files.
        assert!(!space.is_file(&nested));
        assert!(!space.is_file(&other));
        // Idempotent, so a caller needs no existence check.
        workspace.clear_vis().unwrap();
    }

    #[test]
    fn removes_a_stale_previs_plugin_and_tolerates_none() {
        let config = project_config();
        let space = InMemoryFileSpace::new();
        let workspace = PrevisWorkspace::new(&config, &space);
        space.add_file(previs_plugin());

        workspace.remove_stale_previs_plugin().unwrap();

        assert!(!space.is_file(&previs_plugin()));
        workspace.remove_stale_previs_plugin().unwrap();
    }

    #[test]
    fn validate_stops_with_the_supplied_error_when_previs_plugin_is_missing() {
        let config = project_config();
        let space = InMemoryFileSpace::new();
        let workspace = PrevisWorkspace::new(&config, &space);

        // A marker in the log must not win over the missing output.
        let err = workspace
            .validate_generated(Some("ERROR: visibility task did not complete.\n"), missing)
            .unwrap_err();

        assert!(matches!(err, Error::Other(message) if message == "missing Previs.esp"));
    }

    /// Drive `validate_generated` over a run whose `Previs.esp` exists and whose only variable
    /// is the log.
    fn validate_with_ck_log(ck_log: Option<&str>) -> Option<BuildWarning> {
        let config = project_config();
        let space = InMemoryFileSpace::new();
        space.add_file(previs_plugin());

        PrevisWorkspace::new(&config, &space)
            .validate_generated(ck_log, missing)
            .unwrap()
    }

    // The marker cases spell the Creation Kit text out by hand rather than reusing
    // `VISIBILITY_TASK_MARKER`: a test built from the constant would follow a narrowed
    // constant instead of catching it.

    #[test]
    fn validate_warns_on_the_visibility_marker_under_a_prefix() {
        assert_eq!(
            validate_with_ck_log(Some(
                "Masterfile: Fallout4.esm\nDEFAULT: ERROR: visibility task did not complete.\n"
            )),
            Some(BuildWarning::VisibilityTaskIncomplete)
        );
    }

    #[test]
    fn validate_warns_on_the_bare_visibility_marker() {
        assert_eq!(
            validate_with_ck_log(Some(
                "Masterfile: Fallout4.esm\nERROR: visibility task did not complete.\n"
            )),
            Some(BuildWarning::VisibilityTaskIncomplete)
        );
    }

    #[test]
    fn validate_warns_on_the_visibility_marker_in_lowercase() {
        assert_eq!(
            validate_with_ck_log(Some(
                "Masterfile: Fallout4.esm\nerror: visibility task did not complete.\n"
            )),
            Some(BuildWarning::VisibilityTaskIncomplete)
        );
    }

    #[test]
    fn validate_raises_no_warning_for_a_quiet_log() {
        assert_eq!(
            validate_with_ck_log(Some("Masterfile: Fallout4.esm\nDEFAULT: exporting cell\n")),
            None
        );
    }

    /// The batch skips the scan when Creation Kit wrote no log (318), so no log, no warning.
    #[test]
    fn validate_raises_no_warning_when_creation_kit_wrote_no_log() {
        assert_eq!(validate_with_ck_log(None), None);
    }
}
