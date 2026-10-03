//! Test doubles shared by the Workflow Operation and Workflow Run test modules.
//!
//! Crate-visible rather than nested in a private `tests` module so the Workflow Run module's
//! tests can drive the same Step 1 simulation the operation's own tests do.
//!
//! There is no recording Creation Kit here, and that is the point. Substitution happens
//! *underneath* the Creation Kit episode, at `RecordingProcessRunner` and `RecordingWait`, so
//! the real `CreationKitOps` runs and a Step 1 test observes the DLL guard, the mandated MO2
//! delay and the log lifecycle rather than a fake's claim about them. What this module supplies
//! is the other half: the [`Prompts`] the operation asks, and the files Creation Kit would have
//! left behind — recorded into the space the operation reads back, so a test states an outcome
//! instead of writing bytes into a temporary directory and hoping the workspace finds them.
//!
//! The same holds for FO4Edit: there is no recording FO4Edit either. The real `Fo4EditOps` runs
//! over a `RecordingDesktopWindows` scripted by [`behaving_fo4edit_windows`], and the merge's
//! log is recorded as an effect of dismissing Module Selection.
//!
//! Nor is there a recording archive episode. The real `ArchiveOps` runs over a
//! `RecordingProcessRunner` whose calls are scripted with the helpers below, each of which
//! records what one Archive2 or BSArch call leaves behind as an effect of that call, written
//! where the call's own arguments say. [`FaultyFileSpace`] refuses chosen operations, for the
//! swap, move-back and cleanup failures.

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use crate::config::ProjectConfig;
use crate::error::Result;
use crate::files::{FileSpace, InMemoryFileSpace};
use crate::logging;
use crate::tools::desktop::{DesktopAction, RecordingDesktopWindows, ScriptedWindow, WindowHandle};
use crate::tools::process::{
    ExitFlag, RECORDED_PROCESS_ID, RecordingProcessRunner, ScriptedCall, ScriptedExit,
};
use crate::tools::wait::{
    FO4EDIT_CLOSE_DELAYS_SECS, FO4EDIT_POLL_INTERVAL_SECS, FO4EDIT_STARTUP_DELAY_SECS,
    MO2_DELAY_BEFORE_FO4EDIT_SECS,
};

use super::{Confirmation, Prompts};

/// The delays of one FO4Edit merge whose Module Selection is dismissed on the first poll: the
/// MO2 delay before the launch, the startup delay after it, one poll wait, then the close
/// sequence's three.
pub(crate) const ONE_POLL_MERGE_DELAYS: [u64; 6] = [
    MO2_DELAY_BEFORE_FO4EDIT_SECS,
    FO4EDIT_STARTUP_DELAY_SECS,
    FO4EDIT_POLL_INTERVAL_SECS,
    FO4EDIT_CLOSE_DELAYS_SECS[0],
    FO4EDIT_CLOSE_DELAYS_SECS[1],
    FO4EDIT_CLOSE_DELAYS_SECS[2],
];

/// A quiet Creation Kit log: present, readable, and free of the handle-array marker.
pub(crate) const QUIET_CK_LOG: &str = "Masterfile: Fallout4.esm\n";

/// A clean Step 2 merge script log: it carries the `Completed: ` the FO4Edit episode's shared
/// fatal looks for, and no `Error: ` for Step 2 to warn about.
pub(crate) const COMPLETED_COMBINED_OBJECTS_MERGE_LOG: &str =
    "Merging CombinedObjects.esp into MyMod.esp\nCompleted: No Errors.\n";

/// A clean Step 7 merge script log: it carries the `Completed: ` the FO4Edit episode's shared
/// fatal looks for, and the `Completed: No Errors.` Step 7 requires to raise no warning.
pub(crate) const COMPLETED_PREVIS_MERGE_LOG: &str =
    "Merging Previs.esp into MyMod.esp\nCompleted: No Errors.\n";

/// The main form of the FO4Edit a recording process runner spawned.
pub(crate) const FO4EDIT_MAIN_FORM: WindowHandle =
    WindowHandle::from_raw(0x100, RECORDED_PROCESS_ID);

/// The Module Selection dialog of the FO4Edit a recording process runner spawned.
pub(crate) const FO4EDIT_MODULE_SELECTION: WindowHandle =
    WindowHandle::from_raw(0x200, RECORDED_PROCESS_ID);

/// Test [`Prompts`] that record the confirmations they were asked and answer from a script.
///
/// The script is one answer per [`Confirmation`] variant, so a test that refuses one question
/// still consents to every other — a refusal pins exactly the branch it names.
#[derive(Debug)]
pub(crate) struct RecordingPrompts {
    asked: RefCell<Vec<Confirmation>>,
    clear_precombined_answer: bool,
    clear_vis_answer: bool,
}

impl Default for RecordingPrompts {
    fn default() -> Self {
        Self {
            asked: RefCell::new(Vec::new()),
            // Consent by default, so a refusal is something a test has to ask for by name.
            clear_precombined_answer: true,
            clear_vis_answer: true,
        }
    }
}

impl RecordingPrompts {
    /// Prompts that have been asked nothing yet and consent to what they are asked.
    #[must_use]
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Answer [`Confirmation::ClearPrecombined`] with a refusal instead of consent.
    #[must_use]
    pub(crate) const fn refusing_clear_precombined(mut self) -> Self {
        self.clear_precombined_answer = false;
        self
    }

    /// Answer [`Confirmation::ClearVis`] with a refusal instead of consent.
    #[must_use]
    pub(crate) const fn refusing_clear_vis(mut self) -> Self {
        self.clear_vis_answer = false;
        self
    }

    /// Every confirmation asked so far, in the order it was asked.
    #[must_use]
    pub(crate) fn asked(&self) -> Vec<Confirmation> {
        self.asked.borrow().clone()
    }
}

impl Prompts for RecordingPrompts {
    fn confirm(&self, confirmation: &Confirmation) -> Result<bool> {
        self.asked.borrow_mut().push(confirmation.clone());
        Ok(match confirmation {
            Confirmation::ClearPrecombined(_) => self.clear_precombined_answer,
            Confirmation::ClearVis(_) => self.clear_vis_answer,
        })
    }
}

/// Record what a successful Creation Kit precombine run leaves in a Workflow Run's space.
///
/// Meant as a `RecordingProcessRunner` effects callback body, so the simulated tool writes into
/// the very space the Precombine Workspace reads back — the same relationship the real pair
/// has. One definition, not one per test module: a change to what Creation Kit produces belongs
/// here and nowhere else.
///
/// `ck_log` is the log CKPE configured Creation Kit to write. The episode deletes it before the
/// spawn and reads it back after, so it has to appear as an effect of the spawn rather than as
/// something the test seeded beforehand.
pub(crate) fn record_successful_precombine_outputs(
    space: &InMemoryFileSpace,
    config: &ProjectConfig,
    ck_log: &Path,
) {
    space.add_file(config.fo4edit_data_dir().join("CombinedObjects.esp"));
    // Nested, because that is the shape Creation Kit writes precombines in.
    space.add_file(config.precombined_dir().join("cell").join("mesh.nif"));

    // Only a `clean all` run (Clean, and Xbox since V2.99) emits the geometry PSG; a
    // Filtered one never does.
    if config.build_mode.is_clean_build() {
        space.add_file(
            config
                .fo4edit_data_dir()
                .join(format!("{} - Geometry.psg", config.plugin.base_name)),
        );
    }

    space.add_file_with_contents(ck_log, QUIET_CK_LOG);
}

/// Record what a successful Creation Kit `CompressPSG` run leaves in a Workflow Run's space.
///
/// The `.csg` beside the `.psg` it was compressed from, plus this run's log. The `.psg` is left
/// alone: Creation Kit does not remove it, the Compress PSG Operation does (batch 301), and a
/// simulation that removed it would hide whether the operation still does.
///
/// Like [`record_successful_precombine_outputs`], meant as an effects callback body, so the
/// `.csg` appears *because of* the spawn — which is what lets a test seed a stale one first
/// and still tell it apart from a fresh one.
pub(crate) fn record_successful_compress_outputs(
    space: &InMemoryFileSpace,
    config: &ProjectConfig,
    ck_log: &Path,
) {
    space.add_file(
        config
            .fo4edit_data_dir()
            .join(format!("{} - Geometry.csg", config.plugin.base_name)),
    );
    space.add_file_with_contents(ck_log, QUIET_CK_LOG);
}

/// Record what a successful Creation Kit `BuildCDX` run leaves in a Workflow Run's space.
///
/// `Data\<base name>.cdx`, plus this run's log. Like the other helpers here, meant as an
/// effects callback body, so the `.cdx` appears *because of* the spawn and a stale one seeded
/// beforehand can be told apart from it.
pub(crate) fn record_successful_cdx_outputs(
    space: &InMemoryFileSpace,
    config: &ProjectConfig,
    ck_log: &Path,
) {
    space.add_file(
        config
            .fo4edit_data_dir()
            .join(format!("{}.cdx", config.plugin.base_name)),
    );
    space.add_file_with_contents(ck_log, QUIET_CK_LOG);
}

/// Record what a successful Creation Kit `GeneratePreVisData` run leaves in a Workflow Run's
/// space.
///
/// `Data\Previs.esp`, one cluster's `.uvd` under `Data\vis`, and this run's quiet log. Like the
/// other helpers here, meant as an effects callback body, so `Previs.esp` appears *because of*
/// the spawn and a stale one seeded beforehand can be told apart from it. A test that needs a
/// different log overwrites it after calling this.
pub(crate) fn record_successful_previs_outputs(
    space: &InMemoryFileSpace,
    config: &ProjectConfig,
    ck_log: &Path,
) {
    space.add_file(config.fo4edit_data_dir().join("Previs.esp"));
    space.add_file(config.vis_dir().join("cluster.uvd"));
    space.add_file_with_contents(ck_log, QUIET_CK_LOG);
}

/// Record what a successful Step 2 merge script run leaves in a Workflow Run's space: its
/// unattended log, reading [`COMPLETED_COMBINED_OBJECTS_MERGE_LOG`].
///
/// Meant as the `on_dismissed` body of [`behaving_fo4edit_windows`], so the log appears *because
/// of* the dismissal rather than being seeded beforehand: the episode deletes a stale log before
/// launching FO4Edit and polls for this run's, so a seeded one would be gone before it was read.
pub(crate) fn record_successful_combined_objects_merge(space: &InMemoryFileSpace) {
    space.add_file_with_contents(
        logging::unattended_log_path(space),
        COMPLETED_COMBINED_OBJECTS_MERGE_LOG,
    );
}

/// Record what a successful Step 7 merge script run leaves in a Workflow Run's space: its
/// unattended log, reading [`COMPLETED_PREVIS_MERGE_LOG`].
///
/// Meant as the `on_dismissed` body of [`behaving_fo4edit_windows`], for the reason given on
/// [`record_successful_combined_objects_merge`]: a log seeded beforehand would be deleted as
/// stale before it was read.
pub(crate) fn record_successful_previs_merge(space: &InMemoryFileSpace) {
    space.add_file_with_contents(
        logging::unattended_log_path(space),
        COMPLETED_PREVIS_MERGE_LOG,
    );
}

/// A process runner whose FO4Edit spawns exit once `exit` is set, which
/// [`behaving_fo4edit_windows`] does when the episode asks FO4Edit to close.
pub(crate) fn fo4edit_exiting_when<'a>(exit: &ExitFlag) -> RecordingProcessRunner<'a> {
    RecordingProcessRunner::new().spawning(ScriptedExit::WhenFlagged {
        flag: exit.clone(),
        code: 0,
    })
}

/// The windows of a FO4Edit that behaves: its main form, and Module Selection with its `OK`.
///
/// Clicking that `OK` takes Module Selection away and runs `on_dismissed`, which is where a test
/// records what the script leaves behind, since xEdit runs the script once its plugins are
/// chosen. Any close request sets `exit`, so a process spawned with
/// `ScriptedExit::WhenFlagged` on the same flag exits when the episode asks it to close.
pub(crate) fn behaving_fo4edit_windows<'a>(
    exit: &ExitFlag,
    on_dismissed: impl Fn() + 'a,
) -> RecordingDesktopWindows<'a> {
    let exit = exit.clone();
    RecordingDesktopWindows::new()
        .with_window(
            ScriptedWindow::new(FO4EDIT_MAIN_FORM, "FO4Script 4.1.5q x64")
                .with_class("TfrmMain")
                .disabled(),
        )
        .with_window(
            ScriptedWindow::new(FO4EDIT_MODULE_SELECTION, "Module Selection")
                .with_class("TfrmModuleSelect")
                .with_button("OK"),
        )
        .on_action(move |action, script| match action {
            DesktopAction::ClickButton { window, caption }
                if *window == FO4EDIT_MODULE_SELECTION && caption == "OK" =>
            {
                script.remove_window(FO4EDIT_MODULE_SELECTION);
                on_dismissed();
            }
            DesktopAction::RequestClose { .. } => exit.set(),
            _ => {}
        })
}

/// What a simulated archive pack writes as the built Plugin Archive.
pub(crate) const PACKED_ARCHIVE: &str = "packed archive";

/// The precombined mesh a simulated extract or unpack writes beneath `root`.
pub(crate) fn extracted_mesh(root: &Path) -> PathBuf {
    root.join("meshes")
        .join("precombined")
        .join("0000ABCD_OC.nif")
}

/// The old cluster file a simulated extract or unpack writes beneath `root` when the archive
/// already holds `vis`.
pub(crate) fn extracted_old_uvd(root: &Path) -> PathBuf {
    root.join("vis").join("old.uvd")
}

/// An Archive2 pack that writes [`PACKED_ARCHIVE`] where its `-c=` argument says.
///
/// The output path is read from the call's own arguments rather than passed in, so a pack that
/// named the wrong output leaves the archive somewhere the episode does not look.
pub(crate) fn archive2_pack_writing_archive(space: &InMemoryFileSpace) -> ScriptedCall<'_> {
    ScriptedCall::new().with_effect(space, |space, _exe, args| {
        let built = args
            .iter()
            .find_map(|arg| arg.to_str()?.strip_prefix("-c="))
            .expect("an Archive2 pack names its output with -c=");
        space.add_file_with_contents(built, PACKED_ARCHIVE);
    })
}

/// A BSArch pack (`pack <folder> <archive> …`) that writes [`PACKED_ARCHIVE`] at `<archive>`.
pub(crate) fn bsarch_pack_writing_archive(space: &InMemoryFileSpace) -> ScriptedCall<'_> {
    ScriptedCall::new().with_effect(space, |space, _exe, args| {
        space.add_file_with_contents(&args[2], PACKED_ARCHIVE);
    })
}

/// An Archive2 extract (`<archive> -e=. -q`, cwd `Data`) that writes the archive's precombined
/// mesh into `data_dir`, and its old `vis` too when `with_vis` is set.
///
/// Archive2 extracts relative to its working directory, which the call's arguments do not name,
/// so `data_dir` is passed in.
pub(crate) fn archive2_extract_writing_precombines(
    space: &InMemoryFileSpace,
    data_dir: PathBuf,
    with_vis: bool,
) -> ScriptedCall<'_> {
    ScriptedCall::new().with_effect(space, move |space, _exe, _args| {
        space.add_file_with_contents(extracted_mesh(&data_dir), "extracted mesh");
        if with_vis {
            space.add_file_with_contents(extracted_old_uvd(&data_dir), "old previs");
        }
    })
}

/// A BSArch unpack (`unpack <archive> <folder>`) that writes the archive's precombined mesh
/// beneath `<folder>`, and its old `vis` too when `with_vis` is set.
pub(crate) fn bsarch_unpack_writing_precombines(
    space: &InMemoryFileSpace,
    with_vis: bool,
) -> ScriptedCall<'_> {
    ScriptedCall::new().with_effect(space, move |space, _exe, args| {
        let target = PathBuf::from(&args[2]);
        space.add_file_with_contents(extracted_mesh(&target), "unpacked mesh");
        if with_vis {
            space.add_file_with_contents(extracted_old_uvd(&target), "old previs");
        }
    })
}

/// A [`FileSpace`] over an [`InMemoryFileSpace`] that refuses chosen operations on chosen paths,
/// standing in for a usvfs or antivirus lock.
///
/// Every other call is forwarded, so the code under test still sees one consistent space.
#[derive(Debug)]
pub(crate) struct FaultyFileSpace<'a> {
    inner: &'a InMemoryFileSpace,
    refused_dir_removals: Vec<PathBuf>,
    refused_dir_moves_to: Vec<PathBuf>,
    refused_renames_to: Vec<PathBuf>,
    refused_file_removals: Vec<PathBuf>,
}

impl<'a> FaultyFileSpace<'a> {
    /// A wrapper that refuses nothing yet.
    #[must_use]
    pub(crate) const fn over(inner: &'a InMemoryFileSpace) -> Self {
        Self {
            inner,
            refused_dir_removals: Vec::new(),
            refused_dir_moves_to: Vec::new(),
            refused_renames_to: Vec::new(),
            refused_file_removals: Vec::new(),
        }
    }

    /// Fail `remove_dir_all(path)`, leaving everything beneath `path` in place.
    #[must_use]
    pub(crate) fn refusing_dir_removal(mut self, path: impl Into<PathBuf>) -> Self {
        self.refused_dir_removals.push(path.into());
        self
    }

    /// Fail every `move_dir` whose destination is `to`.
    #[must_use]
    pub(crate) fn refusing_dir_move_to(mut self, to: impl Into<PathBuf>) -> Self {
        self.refused_dir_moves_to.push(to.into());
        self
    }

    /// Fail every `rename` whose destination is `to`.
    #[must_use]
    pub(crate) fn refusing_rename_to(mut self, to: impl Into<PathBuf>) -> Self {
        self.refused_renames_to.push(to.into());
        self
    }

    /// Fail `remove_file(path)`, leaving the file in place.
    #[must_use]
    pub(crate) fn refusing_file_removal(mut self, path: impl Into<PathBuf>) -> Self {
        self.refused_file_removals.push(path.into());
        self
    }

    /// The error a refused operation reports, as a locked path would.
    fn refusal(operation: &str, path: &Path) -> crate::error::Error {
        std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            format!("{operation} refused in space: {}", path.display()),
        )
        .into()
    }
}

impl FileSpace for FaultyFileSpace<'_> {
    fn is_file(&self, path: &Path) -> bool {
        self.inner.is_file(path)
    }

    fn find_first_file_with_extension(&self, directory: &Path, extension: &str) -> Option<PathBuf> {
        self.inner
            .find_first_file_with_extension(directory, extension)
    }

    fn remove_file(&self, path: &Path) -> Result<()> {
        if self
            .refused_file_removals
            .iter()
            .any(|refused| refused == path)
        {
            return Err(Self::refusal("remove_file", path));
        }
        self.inner.remove_file(path)
    }

    fn remove_dir_all(&self, directory: &Path) -> Result<()> {
        if self
            .refused_dir_removals
            .iter()
            .any(|refused| refused == directory)
        {
            return Err(Self::refusal("remove_dir_all", directory));
        }
        self.inner.remove_dir_all(directory)
    }

    fn read_lossy(&self, path: &Path) -> Result<String> {
        self.inner.read_lossy(path)
    }

    fn copy(&self, from: &Path, to: &Path) -> Result<()> {
        self.inner.copy(from, to)
    }

    fn rename(&self, from: &Path, to: &Path) -> Result<()> {
        if self.refused_renames_to.iter().any(|refused| refused == to) {
            return Err(Self::refusal("rename", to));
        }
        self.inner.rename(from, to)
    }

    fn exists(&self, path: &Path) -> bool {
        self.inner.exists(path)
    }

    fn create_dir_all(&self, path: &Path) -> Result<()> {
        self.inner.create_dir_all(path)
    }

    fn is_dir(&self, path: &Path) -> bool {
        self.inner.is_dir(path)
    }

    fn child_dirs(&self, directory: &Path) -> Vec<PathBuf> {
        self.inner.child_dirs(directory)
    }

    fn move_dir(&self, from: &Path, to: &Path) -> Result<()> {
        if self
            .refused_dir_moves_to
            .iter()
            .any(|refused| refused == to)
        {
            return Err(Self::refusal("move_dir", to));
        }
        self.inner.move_dir(from, to)
    }

    fn write(&self, path: &Path, contents: &str) -> Result<()> {
        self.inner.write(path, contents)
    }

    fn append(&self, path: &Path, contents: &str) -> Result<()> {
        self.inner.append(path, contents)
    }

    fn temp_dir(&self) -> PathBuf {
        self.inner.temp_dir()
    }
}
