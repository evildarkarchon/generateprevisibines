//! Archive invocation (`:Archive`, `:Extract` and `:AddToArchive` in batch, V2.99 388–447).
//!
//! [`ArchiveOps`] owns the whole Archive episode for both Archive2 and BSArch: choosing and
//! creating the work folder, its restore list, the session-log headers and the output fold, the
//! tool calls and their exit-code policy, the MO2 waits, the archive-exists check, the swap into
//! `Data`, the BSArch move-backs, and the cleanup. Callers state build meaning — "pack the
//! precombines into this archive", "add previs to it" — and never a tool's command grammar, which
//! is why the command lines are private to this module (design item 12 on #29).
//!
//! The Plugin Archive is the only state carried across the archive steps (ADR-0004). Each step
//! builds the new archive in a work folder of its own (`<fo4>\ArchiveWork`, or the first free
//! `ArchiveWork.<n>`), checks it, and only then replaces the archive in `Data`. The work folder
//! keeps a restore list naming anything it may hold the only copy of, so that a crashed run's
//! files can be put back: [`restore_archive_work_folders`] does that at the start of every run.
//! The work-folder naming rule and the restore-list format live here and nowhere else.
//!
//! See `docs/episodes.md` § Archive for the command lines, and `docs/workarounds.md` §2 and §4
//! for the waits and the Archive2 extract-repack, which are required workarounds.

use std::ffi::{OsStr, OsString};
use std::path::{Component, Path, PathBuf};

use crate::config::ArchiveTool;
use crate::error::{Error, Result};
use crate::files::FileSpace;
use crate::logging;
use crate::tools::process::{ProcessOutput, ProcessRunner};
use crate::tools::wait::{
    MO2_DELAY_AFTER_ARCHIVE2_EXTRACT_SECS, MO2_DELAY_BEFORE_BSARCH_PACK_SECS, Wait,
};
use crate::warning::{BuildWarning, BuildWarnings, LeftoverItems};

/// The work folder's first-choice name, in the Fallout 4 directory.
const WORK_FOLDER_NAME: &str = "ArchiveWork";

/// The infix of a set-aside work folder's name: `ArchiveWork.orphaned.<n>`.
const ORPHANED_INFIX: &str = "orphaned";

/// The work folder's staging tree: the only tree BSArch packs.
const STAGING_FOLDER_NAME: &str = "staging";

/// The work folder's restore list.
const RESTORE_LIST_FILE_NAME: &str = "restore.txt";

/// What Step 3 packs, as Archive2's source argument and the session-log header name it.
const PRECOMBINED_SOURCES: &str = r"meshes\precombined";

/// What the Step 8 rebuild packs: Archive2's comma-separated source list (batch 442).
const PRECOMBINED_AND_VIS_SOURCES: &str = r"meshes\precombined,vis";

/// The resolved paths one Workflow Run's Archive episodes run against.
///
/// Preparation resolves these once, from the Workflow Toolchain; execution binds them to the
/// ports an episode runs through. Like [`crate::tools::CreationKitPaths`], it has no `Default`:
/// "no archive tool was prepared" is an absent value.
#[derive(Debug, Clone)]
pub(crate) struct ArchivePaths {
    /// Which tool [`Self::exe`] is, because the command lines differ by tool.
    pub(crate) tool: ArchiveTool,
    /// The resolved `Archive2.exe` or `BSArch.exe`.
    pub(crate) exe: PathBuf,
    /// The Fallout 4 install directory: the parent of every work folder.
    pub(crate) fallout4_dir: PathBuf,
    /// `Data`: the tools' working directory, and the home of the archive and the loose folders.
    pub(crate) data_dir: PathBuf,
    /// The Workflow Run's session log, which each episode folds the tool's output into.
    pub(crate) session_log: PathBuf,
}

impl ArchivePaths {
    /// Bind the resolved paths to the ports one Archive episode runs through.
    ///
    /// The ports are chosen at execution time rather than stored here, so the paths a Workflow
    /// Run carries stay plain data and a test can drive the real episode over recording ports.
    pub(crate) const fn bind<'a>(&'a self, ports: ArchivePorts<'a>) -> ArchiveOps<'a> {
        ArchiveOps { paths: self, ports }
    }
}

/// The internal seams one Archive episode runs through.
///
/// Not part of what a Workflow Operation is handed (ADR-0002); crate-visible only so the tests
/// in `src/workflow/` can assemble recording ones. There is no clock: the batch writes no timing
/// bracket around the archive tools.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ArchivePorts<'a> {
    /// Runs Archive2 or BSArch, waits for it and captures its output.
    pub(crate) process: &'a dyn ProcessRunner,
    /// The MO2 waits: after the Archive2 extract, and before every BSArch pack.
    pub(crate) wait: &'a dyn Wait,
    /// The work folder, the moves, the swap, the cleanup and the session-log appends.
    pub(crate) files: &'a dyn FileSpace,
    /// Where the episode raises its own cleanup warnings.
    ///
    /// The episode raises them itself, rather than returning them, because they are
    /// unconditional facts of the episode and must survive a step that then returns `Err`.
    pub(crate) warnings: &'a BuildWarnings<'a>,
}

/// The Archive episode: Archive2 or BSArch, with its work folder, swap, waits and cleanup.
#[derive(Debug)]
pub(crate) struct ArchiveOps<'a> {
    paths: &'a ArchivePaths,
    ports: ArchivePorts<'a>,
}

impl ArchiveOps<'_> {
    /// Pack `<data>\meshes\precombined` into the Plugin Archive `archive_name` and remove the
    /// loose meshes (Step 3; batch 286–292 for Archive2, 394–401 for BSArch).
    ///
    /// Packs over an existing archive, which is replaced only once the new one has been built
    /// and checked. Returns the tool's failure ([`Error::Archive2Failed`],
    /// [`Error::BsarchFailed`]), [`Error::NoPluginArchiveCreated`], [`Error::ArchiveSwapFailed`],
    /// [`Error::ArchiveMoveBackFailed`], [`Error::ArchiveRestoreListNotRemoved`], or
    /// [`Error::Io`] when the work folder, its restore list, a move or the session log cannot be
    /// written.
    pub(crate) fn archive_precombines(&self, archive_name: &str) -> Result<()> {
        let work = WorkFolder::choose(&self.paths.fallout4_dir, self.ports.files);
        let result = work
            .create(self.ports.files)
            .and_then(|()| match self.paths.tool {
                ArchiveTool::Archive2 => self.archive2_precombines(&work, archive_name),
                ArchiveTool::BSArch => self.bsarch_precombines(&work, archive_name),
            });
        self.finish(&work, result)
    }

    /// Rebuild the Plugin Archive `archive_name` as its own precombines plus `<data>\vis`, then
    /// remove the loose `vis` (Step 8; batch 437–444 for Archive2, ADR-0004's unpack rebuild for
    /// BSArch).
    ///
    /// The operation has already checked that the archive exists and that `vis` holds a `.uvd`.
    /// Returns [`Error::PluginArchiveHasNoPrecombines`], before the old archive is touched, when
    /// the extract or unpack yields no precombined mesh. Otherwise returns the errors
    /// [`Self::archive_precombines`] does, plus [`Error::Archive2ExtractFailed`] and
    /// [`Error::BsarchUnpackFailed`].
    pub(crate) fn add_previs(&self, archive_name: &str) -> Result<()> {
        let work = WorkFolder::choose(&self.paths.fallout4_dir, self.ports.files);
        let result = work
            .create(self.ports.files)
            .and_then(|()| match self.paths.tool {
                ArchiveTool::Archive2 => self.archive2_add_previs(&work, archive_name),
                ArchiveTool::BSArch => self.bsarch_add_previs(&work, archive_name),
            });
        self.finish(&work, result)
    }

    /// Step 3 with Archive2: pack the loose meshes from `Data`, swap, then remove them.
    fn archive2_precombines(&self, work: &WorkFolder, archive_name: &str) -> Result<()> {
        self.pack_header(archive_name, PRECOMBINED_SOURCES)?;
        self.archive2_pack(work, archive_name, PRECOMBINED_SOURCES)?;
        self.list_built_archive(work, archive_name)?;
        self.swap(work, archive_name)?;
        self.clean_up(&precombined_dir(&self.paths.data_dir))
    }

    /// Step 3 with BSArch: move the loose meshes into staging, pack it, then swap.
    ///
    /// The meshes are moved rather than copied, as the batch moves them (396), so the work
    /// folder holds their only copy until the swap; that is what the restore list is for.
    fn bsarch_precombines(&self, work: &WorkFolder, archive_name: &str) -> Result<()> {
        self.pack_header(archive_name, PRECOMBINED_SOURCES)?;

        let loose = precombined_dir(&self.paths.data_dir);
        let staged = precombined_dir(&work.staging());
        self.ports
            .files
            .create_dir_all(&work.staging().join("meshes"))?;
        self.stage(work, &loose, &staged)?;
        self.bsarch_pack_staged(work, archive_name, &staged, &loose)?;
        self.swap(work, archive_name)
        // The staged meshes are now in the archive too, and go with the work folder.
    }

    /// Step 8 with Archive2: the batch's extract → wait → repack (docs/workarounds.md §4).
    ///
    /// The batch's delete of the archive before the repack (440) is gone: the swap replaces
    /// it, so a failed repack leaves the old archive in place. On a failure after the extract,
    /// the extracted precombines stay loose in `Data`, as in the batch, and a rerun extracts
    /// over them.
    fn archive2_add_previs(&self, work: &WorkFolder, archive_name: &str) -> Result<()> {
        logging::append_archive_extract_header(
            &self.paths.session_log,
            archive_name,
            self.ports.files,
        )?;
        let extract = self.run_tool(&[
            OsString::from(archive_name),
            OsString::from("-e=."),
            OsString::from("-q"),
        ])?;
        if !extract.status.success() {
            return Err(Error::Archive2ExtractFailed {
                code: extract.status.code(),
            });
        }

        // Required workaround (batch 439, docs/workarounds.md §2): under MO2 the extracted files
        // are not visible in the virtual `Data` until it has synced.
        self.ports
            .wait
            .sync_delay(MO2_DELAY_AFTER_ARCHIVE2_EXTRACT_SECS);

        let loose_precombined = precombined_dir(&self.paths.data_dir);
        self.require_precombines(&loose_precombined, archive_name)?;

        self.pack_header(archive_name, PRECOMBINED_AND_VIS_SOURCES)?;
        self.archive2_pack(work, archive_name, PRECOMBINED_AND_VIS_SOURCES)?;
        self.list_built_archive(work, archive_name)?;
        self.swap(work, archive_name)?;
        self.clean_up(&loose_precombined)?;
        self.clean_up(&vis_dir(&self.paths.data_dir))
    }

    /// Step 8 with BSArch: unpack the archive into staging, move `vis` in, pack, then swap.
    fn bsarch_add_previs(&self, work: &WorkFolder, archive_name: &str) -> Result<()> {
        // `:Extract`'s header, although the batch never unpacks: the unpack takes its place.
        logging::append_archive_extract_header(
            &self.paths.session_log,
            archive_name,
            self.ports.files,
        )?;

        let staging = work.staging();
        // BSArch's `unpack` needs its target folder to exist already.
        self.ports.files.create_dir_all(&staging)?;
        let unpack = self.run_tool(&[
            OsString::from("unpack"),
            self.paths.data_dir.join(archive_name).into_os_string(),
            staging.clone().into_os_string(),
        ])?;
        if !unpack.status.success() {
            return Err(Error::BsarchUnpackFailed {
                code: unpack.status.code(),
            });
        }

        // What the unpack wrote is never added to the restore list: a crash can leave it half
        // written, and the old archive it came from is still in `Data`, so the next run's
        // restore discards it with the folder rather than moving a partial copy into `Data`.
        self.require_precombines(&precombined_dir(&staging), archive_name)?;

        // The archive's *old* previs, unpacked by this run as a copy, so deleting it is safe.
        // Without this the move of the new `vis` below would fail, or stale visibility data
        // would be packed. Not cleanup: it decides what the archive contains, so a failure here
        // stops the step.
        let staged_vis = vis_dir(&staging);
        if is_present(self.ports.files, &staged_vis) {
            self.ports.files.remove_dir_all(&staged_vis)?;
        }

        self.pack_header(archive_name, PRECOMBINED_AND_VIS_SOURCES)?;
        let loose_vis = vis_dir(&self.paths.data_dir);
        self.stage(work, &loose_vis, &staged_vis)?;
        // On a failed pack only `vis` goes back. The unpacked precombines are a copy (the old
        // archive is still in `Data`), so they are discarded with the work folder.
        self.bsarch_pack_staged(work, archive_name, &staged_vis, &loose_vis)?;
        self.swap(work, archive_name)
    }

    /// Open a pack's session-log entry.
    fn pack_header(&self, archive_name: &str, sources: &str) -> Result<()> {
        logging::append_archive_pack_header(
            &self.paths.session_log,
            archive_name,
            sources,
            self.ports.files,
        )
    }

    /// Run Archive2's pack of `sources` into the work folder, and check it built the archive.
    ///
    /// `sources` stay relative under cwd `Data`, so the archive's internal paths are what the
    /// batch's were. Each argument is one argv token: Rust's own quoting of a `-c=` path with
    /// spaces works with Archive2 (probe, #44), so the batch's `-c="…"` form is not reproduced.
    fn archive2_pack(&self, work: &WorkFolder, archive_name: &str, sources: &str) -> Result<()> {
        let built = work.archive(archive_name);
        let mut output_arg = OsString::from("-c=");
        output_arg.push(built.as_os_str());

        let pack = self.run_tool(&[
            OsString::from(sources),
            output_arg,
            OsString::from("-f=General"),
            OsString::from("-q"),
        ])?;
        if !pack.status.success() {
            return Err(Error::Archive2Failed {
                code: pack.status.code(),
            });
        }

        self.check_built(&built)
    }

    /// Wait, run BSArch's pack of the staging tree into the work folder, and check it built the
    /// archive.
    ///
    /// The output sits beside `staging\`, never inside it, so neither the built archive nor the
    /// restore list can be packed.
    fn bsarch_pack(&self, work: &WorkFolder, archive_name: &str) -> Result<()> {
        // A port addition (docs/workarounds.md §2): files just moved out of MO2's virtual `Data`
        // may not have settled when BSArch reads staging, which yields an incomplete archive.
        self.ports
            .wait
            .sync_delay(MO2_DELAY_BEFORE_BSARCH_PACK_SECS);

        let built = work.archive(archive_name);
        let pack = self.run_tool(&[
            OsString::from("pack"),
            work.staging().into_os_string(),
            built.clone().into_os_string(),
            OsString::from("-mt"),
            OsString::from("-fo4"),
            OsString::from("-z"),
        ])?;
        if !pack.status.success() {
            return Err(Error::BsarchFailed {
                code: pack.status.code(),
            });
        }

        self.check_built(&built)
    }

    /// Pack the staging tree with BSArch and list the built archive, moving `staged` back to
    /// `loose` first when anything before the swap fails.
    ///
    /// Any failure, not only a non-zero exit or a missing archive: until the swap, `staged` is
    /// the only copy of what was moved out of `Data`, and the work folder is removed on the way
    /// out. A failed move-back replaces the error with [`Error::ArchiveMoveBackFailed`], which
    /// keeps the work folder for the next run's restore.
    fn bsarch_pack_staged(
        &self,
        work: &WorkFolder,
        archive_name: &str,
        staged: &Path,
        loose: &Path,
    ) -> Result<()> {
        let packed = self
            .bsarch_pack(work, archive_name)
            .and_then(|()| self.list_built_archive(work, archive_name));
        if let Err(error) = packed {
            // The `source` entry stays listed: the restore skips an entry whose item has left
            // the folder, and a failed move-back needs it.
            if let Err(cause) = self.ports.files.move_dir(staged, loose) {
                tracing::warn!("Moving {} back failed: {cause}", staged.display());
                return Err(Error::ArchiveMoveBackFailed {
                    from: staged.to_path_buf(),
                    to: loose.to_path_buf(),
                    work: work.root.clone(),
                });
            }
            return Err(error);
        }

        Ok(())
    }

    /// Move `loose` out of `Data` to `staged` in the work folder, listing it first.
    ///
    /// The entry is written before the move, so a crash can leave an entry whose item never
    /// moved (which the restore skips), but never a moved item without an entry. A failed
    /// append stops the step before the move, so nothing is at risk.
    fn stage(&self, work: &WorkFolder, loose: &Path, staged: &Path) -> Result<()> {
        work.list(
            self.ports.files,
            &RestoreEntry {
                kind: RestoreKind::Source,
                item: work.relative(staged),
                target: loose.to_path_buf(),
            },
        )?;
        self.ports.files.move_dir(loose, staged)
    }

    /// List the built archive before the swap deletes the old one.
    ///
    /// Only here, after the exists check: an archive that never reached the swap may be half
    /// written, and `Data` still holds what it was made from, so it is never listed.
    fn list_built_archive(&self, work: &WorkFolder, archive_name: &str) -> Result<()> {
        work.list(
            self.ports.files,
            &RestoreEntry {
                kind: RestoreKind::Archive,
                item: PathBuf::from(archive_name),
                target: self.paths.data_dir.join(archive_name),
            },
        )
    }

    /// Replace the archive in `Data` with the built one, then remove the restore list.
    ///
    /// The old archive is replaced only now, after the built one passed the exists check, so a
    /// failed pack never leaves the user without an archive. The list is removed right after a
    /// successful swap: the staged sources (BSArch) are still in the work folder and now in the
    /// archive too, and a list left behind would make the next run move them back into `Data` as
    /// duplicates. Either failure keeps the work folder; see [`Self::finish`].
    fn swap(&self, work: &WorkFolder, archive_name: &str) -> Result<()> {
        let built = work.archive(archive_name);
        let target = self.paths.data_dir.join(archive_name);

        // Delete-then-rename, exactly as the probe (#44) tested inside and outside MO2. Inside
        // MO2 the archive lands in `overwrite`.
        let replaced = (|| {
            if self.ports.files.is_file(&target) {
                self.ports.files.remove_file(&target)?;
            }
            self.ports.files.rename(&built, &target)
        })();
        if let Err(cause) = replaced {
            tracing::warn!("Replacing {} failed: {cause}", target.display());
            return Err(Error::ArchiveSwapFailed {
                built,
                target,
                work: work.root.clone(),
            });
        }

        let restore_list = work.restore_list();
        if let Err(cause) = self.ports.files.remove_file(&restore_list) {
            tracing::warn!("Removing {} failed: {cause}", restore_list.display());
            return Err(Error::ArchiveRestoreListNotRemoved { path: restore_list });
        }

        Ok(())
    }

    /// Run the archive tool with `args` in `Data`, and fold its output into the session log.
    ///
    /// Stderr is folded in after stdout, a divergence from the batch's `>>` (design item 13), so
    /// that Archive2's `-1` stack traces reach the log. A non-zero exit is the caller's to judge.
    fn run_tool(&self, args: &[OsString]) -> Result<ProcessOutput> {
        let output =
            self.ports
                .process
                .run_capturing(&self.paths.exe, args, &self.paths.data_dir)?;
        logging::append_archive_tool_output(
            &self.paths.session_log,
            &output.stdout,
            &output.stderr,
            self.ports.files,
        )?;
        Ok(output)
    }

    /// The archive-exists check after every pack (batch 408).
    ///
    /// Load-bearing: `Archive2 -c` with no sources exits 0 and creates nothing.
    fn check_built(&self, built: &Path) -> Result<()> {
        if self.ports.files.is_file(built) {
            Ok(())
        } else {
            Err(Error::NoPluginArchiveCreated)
        }
    }

    /// Stop with [`Error::PluginArchiveHasNoPrecombines`] when `precombined` holds no `.nif`.
    ///
    /// Design item 9, enforced here as a rule of the rebuild because it sits mid-rebuild and
    /// needs tool-specific cleanup. The batch rebuilds from `vis` alone instead (441 → 445).
    fn require_precombines(&self, precombined: &Path, archive_name: &str) -> Result<()> {
        if self
            .ports
            .files
            .find_first_file_with_extension(precombined, "nif")
            .is_none()
        {
            return Err(Error::PluginArchiveHasNoPrecombines {
                name: archive_name.to_string(),
            });
        }
        Ok(())
    }

    /// Remove `path`, which the archive no longer depends on, raising
    /// [`BuildWarning::ArchiveCleanupFailed`] instead of failing when it cannot be removed.
    ///
    /// Returns `Err` only when the warning cannot be appended to the session log.
    fn clean_up(&self, path: &Path) -> Result<()> {
        if self.ports.files.remove_dir_all(path).is_err() {
            return self
                .ports
                .warnings
                .raise(BuildWarning::ArchiveCleanupFailed {
                    path: path.to_path_buf(),
                });
        }
        Ok(())
    }

    /// End a step: remove the work folder unless the step's outcome keeps it, then return the
    /// outcome.
    ///
    /// The work folder is kept after a failed move-back, a failed swap or a failed restore-list
    /// removal: it may hold the only copy of the new archive, and for BSArch of the staged
    /// precombines or `vis`, and the next run's restore puts them back. On every other path
    /// nothing of the user's remains in it. A failed removal is cleanup: a Build Warning, and on
    /// a failure path the step's own error is still what is returned, so a cleanup problem never
    /// hides why the step stopped.
    fn finish(&self, work: &WorkFolder, result: Result<()>) -> Result<()> {
        if let Err(error) = &result
            && keeps_work_folder(error)
        {
            return result;
        }

        if self.ports.files.remove_dir_all(&work.root).is_ok() {
            return result;
        }
        let raised = self
            .ports
            .warnings
            .raise(BuildWarning::ArchiveCleanupFailed {
                path: work.root.clone(),
            });
        // The step's own error wins over a failed append of the warning; the collector has kept
        // the warning either way.
        result.and(raised)
    }
}

/// Whether a step that stopped with `error` must leave its work folder for the next run.
const fn keeps_work_folder(error: &Error) -> bool {
    matches!(
        error,
        Error::ArchiveMoveBackFailed { .. }
            | Error::ArchiveSwapFailed { .. }
            | Error::ArchiveRestoreListNotRemoved { .. }
    )
}

/// One step's work folder: `staging\`, the built archive beside it, and `restore.txt`.
#[derive(Debug, Clone)]
struct WorkFolder {
    root: PathBuf,
}

impl WorkFolder {
    /// Take the first of `ArchiveWork`, `ArchiveWork.1`, `ArchiveWork.2`, … that is absent.
    ///
    /// A step skips a present name for a fresh sibling rather than building in it or removing
    /// it. A name still present here is one the run-start restore could not remove or set
    /// aside, or one an earlier step of this run failed to clean up, and either has already
    /// been warned about; it may hold the only copy of something, so a step never removes it.
    /// Building in a fresh folder means nothing stale is ever packed (BSArch packs the whole
    /// staging tree) or swapped in. The search always ends, at the first absent name.
    fn choose(fallout4_dir: &Path, files: &dyn FileSpace) -> Self {
        let root = first_absent(work_folder_candidates(fallout4_dir), files);
        let first_choice = fallout4_dir.join(WORK_FOLDER_NAME);
        if root != first_choice {
            tracing::info!(
                "Building in {} because {} could not be removed",
                root.display(),
                first_choice.display()
            );
        }
        Self { root }
    }

    /// Create the folder and its empty restore list.
    ///
    /// A failure stops the run, because there is nowhere safe to build. Nothing has been moved
    /// yet at that point.
    fn create(&self, files: &dyn FileSpace) -> Result<()> {
        files.create_dir_all(&self.root)?;
        files.write(&self.restore_list(), "")
    }

    /// The staging tree BSArch packs and unpacks into.
    ///
    /// A subfolder rather than the work folder itself, so that the built archive and the
    /// restore list, which sit beside it, can never be packed.
    fn staging(&self) -> PathBuf {
        self.root.join(STAGING_FOLDER_NAME)
    }

    /// Where the new archive is built, under its final name.
    fn archive(&self, archive_name: &str) -> PathBuf {
        self.root.join(archive_name)
    }

    fn restore_list(&self) -> PathBuf {
        self.root.join(RESTORE_LIST_FILE_NAME)
    }

    /// `path`, which lies in this folder, relative to it.
    fn relative(&self, path: &Path) -> PathBuf {
        path.strip_prefix(&self.root)
            .expect("only paths inside the work folder are listed")
            .to_path_buf()
    }

    /// Append `entry` to the restore list.
    fn list(&self, files: &dyn FileSpace, entry: &RestoreEntry) -> Result<()> {
        files.append(&self.restore_list(), &entry.line())
    }
}

/// What a restore-list entry's item is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RestoreKind {
    /// A folder moved out of `Data` into staging.
    Source,
    /// The built archive, listed at the swap.
    Archive,
}

impl RestoreKind {
    const fn label(self) -> &'static str {
        match self {
            Self::Source => "source",
            Self::Archive => "archive",
        }
    }

    fn parse(label: &str) -> Option<Self> {
        match label {
            "source" => Some(Self::Source),
            "archive" => Some(Self::Archive),
            _ => None,
        }
    }
}

/// One line of a work folder's restore list: what it may hold the only copy of, and where that
/// goes back.
///
/// The format is UTF-8 text, one entry per line, three TAB-separated fields (`<kind>`, `<item>`,
/// `<target>`), each line ended by `\n`. `item` is relative to the work folder; `target` is the
/// path the item goes back to. Windows paths cannot contain a TAB or a line break, so neither
/// needs escaping.
#[derive(Debug, Clone, PartialEq, Eq)]
struct RestoreEntry {
    kind: RestoreKind,
    item: PathBuf,
    target: PathBuf,
}

impl RestoreEntry {
    /// The entry as one restore-list line, terminator included.
    fn line(&self) -> String {
        format!(
            "{}\t{}\t{}\n",
            self.kind.label(),
            self.item.display(),
            self.target.display()
        )
    }

    /// Parse one non-blank line, or `None` when it is not a valid entry.
    ///
    /// An item that is absolute, or that climbs out of the work folder, is invalid: the restore
    /// would otherwise move something that is not the work folder's.
    fn parse(line: &str) -> Option<Self> {
        let mut fields = line.split('\t');
        let (Some(kind), Some(item), Some(target), None) =
            (fields.next(), fields.next(), fields.next(), fields.next())
        else {
            return None;
        };

        let kind = RestoreKind::parse(kind)?;
        let item = PathBuf::from(item);
        let inside_the_folder = item
            .components()
            .all(|component| matches!(component, Component::Normal(_)));
        if item.as_os_str().is_empty() || !inside_the_folder || target.is_empty() {
            return None;
        }

        Some(Self {
            kind,
            item,
            target: PathBuf::from(target),
        })
    }
}

/// Parse a whole restore list, or `None` when any line is invalid. Blank lines are ignored.
fn parse_restore_list(text: &str) -> Option<Vec<RestoreEntry>> {
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(RestoreEntry::parse)
        .collect()
}

/// Restore, then clear, every leftover archive work folder in `fallout4_dir` (ADR-0004).
///
/// Called by the Workflow Run once, after preparation and before the first step, on **every**
/// run, whatever its resume point and whether or not it reaches an archive step: a crash can
/// leave a work folder holding the only copy of the staged precombines, `vis` or a new archive,
/// and a run that stopped after Step 3 may next be resumed at Step 6. It replaces the batch's
/// Step 1-only `RD` of its staging folder (262).
///
/// The installation lock (ADR-0005) is held, so every folder found belongs to a run that has
/// ended, and the restore may act on it without an ownership check of its own.
///
/// For each folder named `ArchiveWork` or `ArchiveWork.<digits>`, in name order: listed items
/// still in it go back to `Data` ([`BuildWarning::ArchiveWorkRestored`]); a folder that cannot
/// be fully restored is set aside as `ArchiveWork.orphaned.<n>`
/// ([`BuildWarning::ArchiveWorkSetAside`], or [`BuildWarning::ArchiveWorkNotRestored`] when
/// even that fails) and never removed; any other is removed, with a console line, or
/// [`BuildWarning::ArchiveWorkFolderNotCleared`] when it cannot be.
///
/// Nothing found here ever stops the run. Returns `Err` only when a warning cannot be appended
/// to the session log, like every other raise.
pub(crate) fn restore_archive_work_folders(
    fallout4_dir: &Path,
    files: &dyn FileSpace,
    warnings: &BuildWarnings<'_>,
) -> Result<()> {
    // Listed rather than probed name by name, so a gap (an `ArchiveWork.3` with no `.1` or `.2`)
    // is still found. Sorted so that the handling and its log lines are deterministic.
    let mut leftovers = files
        .child_dirs(fallout4_dir)
        .into_iter()
        .filter(|folder| {
            folder
                .file_name()
                .and_then(OsStr::to_str)
                .is_some_and(is_work_folder_name)
        })
        .collect::<Vec<_>>();
    leftovers.sort();

    for leftover in &leftovers {
        restore_leftover(fallout4_dir, leftover, files, warnings)?;
    }
    Ok(())
}

/// How restoring one leftover folder's list ended.
enum Restoration {
    /// Every listed item still in the folder went back, so the folder holds only copies.
    Complete,
    /// Something listed could not go back, so the folder must be set aside.
    Blocked,
}

/// Restore one leftover work folder, then remove it or set it aside.
fn restore_leftover(
    fallout4_dir: &Path,
    leftover: &Path,
    files: &dyn FileSpace,
    warnings: &BuildWarnings<'_>,
) -> Result<()> {
    // The whole list is parsed before anything moves, so an invalid line sets the folder aside
    // untouched rather than half restored.
    let unrestored = match read_restore_list(leftover, files) {
        None => Some(LeftoverItems::UnreadableList),
        Some(entries) => match restore_entries(leftover, &entries, files, warnings)? {
            Restoration::Complete => None,
            Restoration::Blocked => {
                Some(LeftoverItems::Listed(still_held(leftover, &entries, files)))
            }
        },
    };

    match unrestored {
        Some(items) => set_aside(fallout4_dir, leftover, items, files, warnings),
        None => remove_leftover(leftover, files, warnings),
    }
}

/// The folder's restore-list entries: none when it has no list, `None` when the list cannot be
/// read or holds an invalid line.
fn read_restore_list(leftover: &Path, files: &dyn FileSpace) -> Option<Vec<RestoreEntry>> {
    let list = leftover.join(RESTORE_LIST_FILE_NAME);
    if !files.is_file(&list) {
        return Some(Vec::new());
    }
    parse_restore_list(&files.read_lossy(&list).ok()?)
}

/// Move each listed item still in `leftover` back to its target, in list order.
///
/// An entry whose item has left the folder is skipped: the crash came before that move, or a
/// move-back or the swap already took it. An `archive` entry whose target exists is skipped
/// too, because the swap never deleted the old archive, and its sources are in `Data` or are
/// restored with it. A `source` entry whose target exists blocks the folder instead: merging
/// into it could mix two runs' files.
fn restore_entries(
    leftover: &Path,
    entries: &[RestoreEntry],
    files: &dyn FileSpace,
    warnings: &BuildWarnings<'_>,
) -> Result<Restoration> {
    for entry in entries {
        let item = leftover.join(&entry.item);
        if !is_present(files, &item) {
            continue;
        }

        let moved = match entry.kind {
            RestoreKind::Archive => {
                if is_present(files, &entry.target) {
                    continue;
                }
                files.rename(&item, &entry.target)
            }
            RestoreKind::Source => {
                if is_present(files, &entry.target) {
                    return Ok(Restoration::Blocked);
                }
                entry
                    .target
                    .parent()
                    .map_or(Ok(()), |parent| files.create_dir_all(parent))
                    .and_then(|()| files.move_dir(&item, &entry.target))
            }
        };
        if moved.is_err() {
            return Ok(Restoration::Blocked);
        }

        warnings.raise(BuildWarning::ArchiveWorkRestored {
            item,
            target: entry.target.clone(),
        })?;
    }

    Ok(Restoration::Complete)
}

/// The listed items still in `leftover`, for the set-aside warning.
fn still_held(leftover: &Path, entries: &[RestoreEntry], files: &dyn FileSpace) -> Vec<PathBuf> {
    entries
        .iter()
        .filter(|entry| is_present(files, &leftover.join(&entry.item)))
        .map(|entry| entry.item.clone())
        .collect()
}

/// Rename `leftover` to the first absent `ArchiveWork.orphaned.<n>`.
///
/// Set aside rather than removed, because it holds the only copy of something; renamed out of
/// the `ArchiveWork*` names so that no later run builds in it or handles it again. It keeps its
/// `restore.txt`, so the user can see where each item belongs. If the rename fails too, the
/// folder is left where it is, and still never removed.
fn set_aside(
    fallout4_dir: &Path,
    leftover: &Path,
    items: LeftoverItems,
    files: &dyn FileSpace,
    warnings: &BuildWarnings<'_>,
) -> Result<()> {
    let to = first_absent(orphaned_folder_candidates(fallout4_dir), files);
    let warning = match files.move_dir(leftover, &to) {
        Ok(()) => BuildWarning::ArchiveWorkSetAside {
            from: leftover.to_path_buf(),
            to,
            items,
        },
        Err(_) => BuildWarning::ArchiveWorkNotRestored {
            path: leftover.to_path_buf(),
            items,
        },
    };
    warnings.raise(warning)
}

/// Remove a leftover that holds only copies and unfinished output.
///
/// A console line rather than a Build Warning, because nothing was degraded; a folder that
/// cannot be removed is a warning, and each archive step then builds in the next free name.
fn remove_leftover(
    leftover: &Path,
    files: &dyn FileSpace,
    warnings: &BuildWarnings<'_>,
) -> Result<()> {
    tracing::info!(
        "Removing archive work folder {} left over from an earlier run",
        leftover.display()
    );
    if files.remove_dir_all(leftover).is_err() {
        return warnings.raise(BuildWarning::ArchiveWorkFolderNotCleared {
            path: leftover.to_path_buf(),
        });
    }
    Ok(())
}

/// Whether `name` is a work folder's: `ArchiveWork` or `ArchiveWork.<digits>`, compared ASCII
/// case-insensitively because Windows names are.
fn is_work_folder_name(name: &str) -> bool {
    let Some(prefix) = name.get(..WORK_FOLDER_NAME.len()) else {
        return false;
    };
    if !prefix.eq_ignore_ascii_case(WORK_FOLDER_NAME) {
        return false;
    }

    let suffix = &name[WORK_FOLDER_NAME.len()..];
    suffix.is_empty()
        || suffix.strip_prefix('.').is_some_and(|digits| {
            !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
        })
}

/// `ArchiveWork`, `ArchiveWork.1`, `ArchiveWork.2`, … in `fallout4_dir`, without end.
fn work_folder_candidates(fallout4_dir: &Path) -> impl Iterator<Item = PathBuf> + '_ {
    std::iter::once(fallout4_dir.join(WORK_FOLDER_NAME))
        .chain((1_u64..).map(move |n| fallout4_dir.join(format!("{WORK_FOLDER_NAME}.{n}"))))
}

/// `ArchiveWork.orphaned.1`, `ArchiveWork.orphaned.2`, … in `fallout4_dir`, without end.
fn orphaned_folder_candidates(fallout4_dir: &Path) -> impl Iterator<Item = PathBuf> + '_ {
    (1_u64..).map(move |n| fallout4_dir.join(format!("{WORK_FOLDER_NAME}.{ORPHANED_INFIX}.{n}")))
}

/// The first of `candidates` with nothing at it.
fn first_absent(mut candidates: impl Iterator<Item = PathBuf>, files: &dyn FileSpace) -> PathBuf {
    candidates
        .find(|candidate| !is_present(files, candidate))
        .expect("the candidate names never run out")
}

/// Whether anything — a file or a directory — is at `path`.
///
/// Both questions are asked because the in-memory space answers `exists` for files only.
fn is_present(files: &dyn FileSpace, path: &Path) -> bool {
    files.exists(path) || files.is_dir(path)
}

fn precombined_dir(root: &Path) -> PathBuf {
    root.join("meshes").join("precombined")
}

fn vis_dir(root: &Path) -> PathBuf {
    root.join("vis")
}

/// Seeding helpers for tests outside this module that start a run over leftover work folders.
///
/// They write the restore list in this module's own format, so the format still lives only
/// here.
#[cfg(test)]
pub(crate) mod leftovers {
    use std::path::{Path, PathBuf};

    use super::{RESTORE_LIST_FILE_NAME, RestoreEntry, RestoreKind};

    /// The restore list of the work folder `folder`.
    pub(crate) fn restore_list(folder: &Path) -> PathBuf {
        folder.join(RESTORE_LIST_FILE_NAME)
    }

    /// A `source` line: `item`, relative to the work folder, was moved out of `target`.
    pub(crate) fn source_line(item: &Path, target: &Path) -> String {
        RestoreEntry {
            kind: RestoreKind::Source,
            item: item.to_path_buf(),
            target: target.to_path_buf(),
        }
        .line()
    }

    /// An `archive` line: `item`, the built archive, was about to replace `target`.
    pub(crate) fn archive_line(item: &Path, target: &Path) -> String {
        RestoreEntry {
            kind: RestoreKind::Archive,
            item: item.to_path_buf(),
            target: target.to_path_buf(),
        }
        .line()
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::ffi::OsStr;

    use super::*;
    use crate::files::{InMemoryFileSpace, SystemFileSpace};
    use crate::tools::process::{
        ProcessCallKind, RecordedProcessCall, RecordingProcessRunner, ScriptedCall,
    };
    use crate::tools::wait::RecordingWait;
    use crate::workflow::operations::recording_adapters::{
        FaultyFileSpace, PACKED_ARCHIVE, archive2_extract_writing_precombines,
        archive2_pack_writing_archive, bsarch_pack_writing_archive,
        bsarch_unpack_writing_precombines, extracted_mesh, extracted_old_uvd,
    };

    const NAME: &str = "MyMod - Main.ba2";
    const OLD_ARCHIVE: &str = "old archive";
    const SEPARATOR: &str = "====================================";

    fn fallout4_dir() -> PathBuf {
        PathBuf::from("Fallout4")
    }

    fn data() -> PathBuf {
        fallout4_dir().join("Data")
    }

    fn work() -> PathBuf {
        fallout4_dir().join(WORK_FOLDER_NAME)
    }

    fn staging() -> PathBuf {
        work().join(STAGING_FOLDER_NAME)
    }

    fn session_log() -> PathBuf {
        PathBuf::from("in-memory-temp").join("MyMod.log")
    }

    fn data_archive() -> PathBuf {
        data().join(NAME)
    }

    fn loose_precombined() -> PathBuf {
        precombined_dir(&data())
    }

    fn loose_vis() -> PathBuf {
        vis_dir(&data())
    }

    /// A loose mesh Step 1 left, nested as Creation Kit writes them.
    fn loose_mesh() -> PathBuf {
        loose_precombined().join("cell").join("mesh.nif")
    }

    /// A cluster file Step 6 left.
    fn new_uvd() -> PathBuf {
        loose_vis().join("new.uvd")
    }

    fn exe(tool: ArchiveTool) -> PathBuf {
        match tool {
            ArchiveTool::Archive2 => PathBuf::from("Tools").join("Archive2.exe"),
            ArchiveTool::BSArch => PathBuf::from("FO4Edit").join("BSArch.exe"),
        }
    }

    fn paths(tool: ArchiveTool) -> ArchivePaths {
        ArchivePaths {
            tool,
            exe: exe(tool),
            fallout4_dir: fallout4_dir(),
            data_dir: data(),
            session_log: session_log(),
        }
    }

    /// One capturing call of `tool`, as the process port should have received it.
    fn call(tool: ArchiveTool, args: &[&OsStr]) -> RecordedProcessCall {
        RecordedProcessCall {
            exe: exe(tool),
            args: args.iter().map(OsString::from).collect(),
            cwd: data(),
            kind: ProcessCallKind::RunCapturing,
        }
    }

    /// Archive2's `-c=<work>\<name>` argument.
    fn archive2_output_arg(work: &Path) -> OsString {
        let mut arg = OsString::from("-c=");
        arg.push(work.join(NAME));
        arg
    }

    /// What one episode did, for the assertions.
    struct Episode {
        result: Result<()>,
        calls: Vec<RecordedProcessCall>,
        delays: Vec<u64>,
        warnings: Vec<BuildWarning>,
    }

    /// Run `verb` through the real episode for `tool`, over `files` and `process`.
    ///
    /// `wait` is the caller's, so a tool call's effect can read the delays recorded before it.
    fn run_episode(
        tool: ArchiveTool,
        files: &dyn FileSpace,
        process: &RecordingProcessRunner<'_>,
        wait: &RecordingWait,
        verb: impl FnOnce(&ArchiveOps<'_>) -> Result<()>,
    ) -> Episode {
        let paths = paths(tool);
        let warnings = BuildWarnings::new(session_log(), files);
        let result = verb(&paths.bind(ArchivePorts {
            process,
            wait,
            files,
            warnings: &warnings,
        }));
        Episode {
            result,
            calls: process.calls(),
            delays: wait.delays(),
            warnings: warnings.raised(),
        }
    }

    fn archive_precombines(ops: &ArchiveOps<'_>) -> Result<()> {
        ops.archive_precombines(NAME)
    }

    fn add_previs(ops: &ArchiveOps<'_>) -> Result<()> {
        ops.add_previs(NAME)
    }

    /// What Step 3 starts from: loose meshes, and an archive from an earlier attempt.
    fn step_three_space() -> InMemoryFileSpace {
        let files = InMemoryFileSpace::new();
        files.add_file_with_contents(loose_mesh(), "mesh");
        files.add_file_with_contents(data_archive(), OLD_ARCHIVE);
        files
    }

    /// What Step 8 starts from: the Plugin Archive, and Step 6's `vis`.
    fn step_eight_space() -> InMemoryFileSpace {
        let files = InMemoryFileSpace::new();
        files.add_file_with_contents(data_archive(), OLD_ARCHIVE);
        files.add_file_with_contents(new_uvd(), "new previs");
        files
    }

    /// The restore list in `work`, parsed with the episode's own format.
    fn listed(files: &dyn FileSpace, work: &Path) -> Vec<RestoreEntry> {
        parse_restore_list(
            &files
                .read_lossy(&work.join(RESTORE_LIST_FILE_NAME))
                .unwrap(),
        )
        .expect("the episode writes a valid list")
    }

    fn source_entry(item: PathBuf, target: PathBuf) -> RestoreEntry {
        RestoreEntry {
            kind: RestoreKind::Source,
            item,
            target,
        }
    }

    fn archive_entry() -> RestoreEntry {
        RestoreEntry {
            kind: RestoreKind::Archive,
            item: PathBuf::from(NAME),
            target: data_archive(),
        }
    }

    fn pack_header(sources: &str) -> String {
        format!("Creating Archive {NAME} of {sources}:\n{SEPARATOR}\n")
    }

    fn extract_header() -> String {
        format!("Extracting Archive {NAME}:\n{SEPARATOR}\n")
    }

    // --- archive_precombines -------------------------------------------------------------

    /// Step 3 with Archive2: the argv and cwd, no wait, the header then stdout then stderr in the
    /// session log, the archive replaced by the built one, and nothing left behind.
    #[test]
    fn archive2_packs_the_loose_precombines_and_swaps_the_archive_in() {
        let files = step_three_space();
        let wait = RecordingWait::new();
        let process = RecordingProcessRunner::new().scripting_call(
            0,
            archive2_pack_writing_archive(&files)
                .with_stdout("Packed 1 file")
                .with_stderr("a warning"),
        );

        let episode = run_episode(
            ArchiveTool::Archive2,
            &files,
            &process,
            &wait,
            archive_precombines,
        );

        episode.result.unwrap();
        assert_eq!(
            episode.calls,
            vec![call(
                ArchiveTool::Archive2,
                &[
                    OsStr::new(r"meshes\precombined"),
                    &archive2_output_arg(&work()),
                    OsStr::new("-f=General"),
                    OsStr::new("-q"),
                ]
            )]
        );
        assert_eq!(episode.delays, Vec::<u64>::new());
        assert_eq!(
            files.read_lossy(&session_log()).unwrap(),
            format!(
                "{}Packed 1 file\na warning\n",
                pack_header(r"meshes\precombined")
            )
        );
        assert_eq!(files.read_lossy(&data_archive()).unwrap(), PACKED_ARCHIVE);
        assert!(!files.is_dir(&loose_precombined()));
        assert!(!files.is_dir(&work()));
        assert_eq!(episode.warnings, []);
    }

    /// Step 3 with BSArch: the meshes move into staging, the 5s wait comes before the pack, and
    /// the staging tree is what is packed, with the output beside it.
    #[test]
    fn bsarch_stages_the_precombines_waits_and_packs_the_staging_tree() {
        let files = step_three_space();
        let wait = RecordingWait::new();
        let process = RecordingProcessRunner::new().scripting_call(
            0,
            bsarch_pack_writing_archive(&files).with_stdout("Packing"),
        );

        let episode = run_episode(
            ArchiveTool::BSArch,
            &files,
            &process,
            &wait,
            archive_precombines,
        );

        episode.result.unwrap();
        assert_eq!(
            episode.calls,
            vec![call(
                ArchiveTool::BSArch,
                &[
                    OsStr::new("pack"),
                    staging().as_os_str(),
                    work().join(NAME).as_os_str(),
                    OsStr::new("-mt"),
                    OsStr::new("-fo4"),
                    OsStr::new("-z"),
                ]
            )]
        );
        assert_eq!(episode.delays, vec![MO2_DELAY_BEFORE_BSARCH_PACK_SECS]);
        assert_eq!(
            files.read_lossy(&session_log()).unwrap(),
            format!("{}Packing\n", pack_header(r"meshes\precombined"))
        );
        assert_eq!(files.read_lossy(&data_archive()).unwrap(), PACKED_ARCHIVE);
        assert!(!files.is_dir(&loose_precombined()));
        assert!(!files.is_dir(&work()));
    }

    /// The header reaches the session log before the tool runs, so a hung tool leaves a record.
    #[test]
    fn the_header_is_written_before_the_tool_runs() {
        for tool in [ArchiveTool::Archive2, ArchiveTool::BSArch] {
            let files = step_three_space();
            let wait = RecordingWait::new();
            let seen = RefCell::new(None);
            let process = RecordingProcessRunner::new().scripting_call(
                0,
                ScriptedCall::new().with_effect(&files, |space, _exe, _args| {
                    *seen.borrow_mut() = space.read_lossy(&session_log()).ok();
                }),
            );

            run_episode(tool, &files, &process, &wait, archive_precombines);

            assert_eq!(
                seen.borrow().as_deref(),
                Some(pack_header(r"meshes\precombined").as_str()),
                "tool: {tool:?}"
            );
        }
    }

    /// The restore list is written before what it guards: an Archive2 pack sees an empty list,
    /// a BSArch pack sees exactly the `source` entry for the folder just moved in, and neither
    /// the list nor the output is ever beneath the packed staging tree.
    #[test]
    fn the_restore_list_names_only_what_has_already_moved_out_of_data() {
        let cases = [
            (ArchiveTool::Archive2, vec![]),
            (
                ArchiveTool::BSArch,
                vec![source_entry(
                    PathBuf::from(STAGING_FOLDER_NAME)
                        .join("meshes")
                        .join("precombined"),
                    loose_precombined(),
                )],
            ),
        ];

        for (tool, expected) in cases {
            let files = step_three_space();
            let wait = RecordingWait::new();
            let at_pack = RefCell::new(None);
            let process = RecordingProcessRunner::new().scripting_call(
                0,
                ScriptedCall::new().with_effect(&files, |space, _exe, _args| {
                    *at_pack.borrow_mut() = Some(listed(space, &work()));
                    assert!(!space.is_file(&staging().join(RESTORE_LIST_FILE_NAME)));
                    assert_eq!(
                        space.find_first_file_with_extension(&staging(), "ba2"),
                        None
                    );
                    space.add_file_with_contents(work().join(NAME), PACKED_ARCHIVE);
                }),
            );

            run_episode(tool, &files, &process, &wait, archive_precombines)
                .result
                .unwrap();

            assert_eq!(at_pack.borrow().as_ref(), Some(&expected), "tool: {tool:?}");
        }
    }

    /// A pack that exits 0 and writes nothing — `Archive2 -c` with no sources does exactly
    /// that — never replaces the old archive, and BSArch moves the meshes back.
    #[test]
    fn a_pack_that_built_nothing_leaves_the_old_archive_alone() {
        for tool in [ArchiveTool::Archive2, ArchiveTool::BSArch] {
            let files = step_three_space();
            let wait = RecordingWait::new();
            let process = RecordingProcessRunner::new();

            let episode = run_episode(tool, &files, &process, &wait, archive_precombines);

            assert!(
                matches!(episode.result, Err(Error::NoPluginArchiveCreated)),
                "tool: {tool:?}, result: {:?}",
                episode.result
            );
            assert_eq!(files.read_lossy(&data_archive()).unwrap(), OLD_ARCHIVE);
            assert_eq!(files.read_lossy(&loose_mesh()).unwrap(), "mesh");
            assert!(!files.is_dir(&work()), "tool: {tool:?}");
        }
    }

    /// A non-zero pack exit stops with the tool's own error and code, the old archive untouched;
    /// BSArch moves the staged meshes back first.
    #[test]
    fn a_failed_pack_stops_with_its_exit_code_and_restores_the_loose_meshes() {
        for tool in [ArchiveTool::Archive2, ArchiveTool::BSArch] {
            let files = step_three_space();
            let wait = RecordingWait::new();
            let process = RecordingProcessRunner::new().returning_exit_code(-1);

            let episode = run_episode(tool, &files, &process, &wait, archive_precombines);

            let code = match episode.result {
                Err(Error::Archive2Failed { code }) if tool == ArchiveTool::Archive2 => code,
                Err(Error::BsarchFailed { code }) if tool == ArchiveTool::BSArch => code,
                other => panic!("tool: {tool:?}, result: {other:?}"),
            };
            assert_eq!(code, Some(-1));
            assert_eq!(files.read_lossy(&data_archive()).unwrap(), OLD_ARCHIVE);
            assert_eq!(files.read_lossy(&loose_mesh()).unwrap(), "mesh");
            assert!(!files.is_dir(&work()), "tool: {tool:?}");
        }
    }

    /// A move-back that fails replaces the pack's error, names both paths, and keeps the work
    /// folder with its `source` entry still listed, for the next run's restore.
    #[test]
    fn a_failed_move_back_keeps_the_work_folder_for_the_next_run() {
        let files = step_three_space();
        let wait = RecordingWait::new();
        // Something re-creates the loose folder while BSArch runs, so the move back is refused.
        let process = RecordingProcessRunner::new().scripting_call(
            0,
            ScriptedCall::new()
                .exiting_with(1)
                .with_effect(&files, |space, _exe, _args| {
                    space.add_file(loose_precombined().join("intruder.nif"));
                }),
        );

        let episode = run_episode(
            ArchiveTool::BSArch,
            &files,
            &process,
            &wait,
            archive_precombines,
        );

        let staged = precombined_dir(&staging());
        assert!(
            matches!(
                &episode.result,
                Err(Error::ArchiveMoveBackFailed { from, to, work: kept })
                    if *from == staged && *to == loose_precombined() && *kept == work()
            ),
            "result: {:?}",
            episode.result
        );
        assert_eq!(
            files
                .read_lossy(&staged.join("cell").join("mesh.nif"))
                .unwrap(),
            "mesh"
        );
        assert_eq!(
            listed(&files, &work()),
            vec![source_entry(
                PathBuf::from(STAGING_FOLDER_NAME)
                    .join("meshes")
                    .join("precombined"),
                loose_precombined(),
            )]
        );
    }

    /// A swap whose rename fails keeps the work folder and the built archive, with the list
    /// naming the archive (and, for BSArch, the staged meshes).
    #[test]
    fn a_failed_swap_keeps_the_built_archive_and_its_list() {
        for tool in [ArchiveTool::Archive2, ArchiveTool::BSArch] {
            let inner = step_three_space();
            let files = FaultyFileSpace::over(&inner).refusing_rename_to(data_archive());
            let wait = RecordingWait::new();
            let pack = match tool {
                ArchiveTool::Archive2 => archive2_pack_writing_archive(&inner),
                ArchiveTool::BSArch => bsarch_pack_writing_archive(&inner),
            };
            let process = RecordingProcessRunner::new().scripting_call(0, pack);

            let episode = run_episode(tool, &files, &process, &wait, archive_precombines);

            assert!(
                matches!(
                    &episode.result,
                    Err(Error::ArchiveSwapFailed { built, target, work: kept })
                        if *built == work().join(NAME)
                            && *target == data_archive()
                            && *kept == work()
                ),
                "tool: {tool:?}, result: {:?}",
                episode.result
            );
            assert_eq!(
                inner.read_lossy(&work().join(NAME)).unwrap(),
                PACKED_ARCHIVE
            );
            let mut expected = Vec::new();
            if tool == ArchiveTool::BSArch {
                expected.push(source_entry(
                    PathBuf::from(STAGING_FOLDER_NAME)
                        .join("meshes")
                        .join("precombined"),
                    loose_precombined(),
                ));
            }
            expected.push(archive_entry());
            assert_eq!(listed(&inner, &work()), expected, "tool: {tool:?}");
        }
    }

    /// A restore list that cannot be removed after a successful swap stops the run, with the
    /// new archive already in place and the work folder kept.
    #[test]
    fn a_restore_list_left_after_the_swap_stops_the_run() {
        let inner = step_three_space();
        let files = FaultyFileSpace::over(&inner)
            .refusing_file_removal(work().join(RESTORE_LIST_FILE_NAME));
        let wait = RecordingWait::new();
        let process =
            RecordingProcessRunner::new().scripting_call(0, bsarch_pack_writing_archive(&inner));

        let episode = run_episode(
            ArchiveTool::BSArch,
            &files,
            &process,
            &wait,
            archive_precombines,
        );

        assert!(
            matches!(
                &episode.result,
                Err(Error::ArchiveRestoreListNotRemoved { path })
                    if *path == work().join(RESTORE_LIST_FILE_NAME)
            ),
            "result: {:?}",
            episode.result
        );
        assert_eq!(inner.read_lossy(&data_archive()).unwrap(), PACKED_ARCHIVE);
        assert!(inner.is_dir(&work()));
    }

    /// A present `ArchiveWork` is skipped for `ArchiveWork.1`, and with that present too, for
    /// `ArchiveWork.2`. The step neither removes nor touches what it skipped, nor warns.
    #[test]
    fn a_present_work_folder_is_skipped_for_the_next_free_name() {
        let cases = [
            (vec![work()], fallout4_dir().join("ArchiveWork.1")),
            (
                vec![work(), fallout4_dir().join("ArchiveWork.1")],
                fallout4_dir().join("ArchiveWork.2"),
            ),
        ];

        for (stuck, expected) in cases {
            let files = step_three_space();
            for folder in &stuck {
                files.add_file_with_contents(folder.join("stuck.txt"), "stuck");
            }
            let wait = RecordingWait::new();
            let process = RecordingProcessRunner::new()
                .scripting_call(0, archive2_pack_writing_archive(&files));

            let episode = run_episode(
                ArchiveTool::Archive2,
                &files,
                &process,
                &wait,
                archive_precombines,
            );

            episode.result.unwrap();
            assert_eq!(episode.calls[0].args[1], archive2_output_arg(&expected));
            assert_eq!(files.read_lossy(&data_archive()).unwrap(), PACKED_ARCHIVE);
            for folder in &stuck {
                assert_eq!(
                    files.read_lossy(&folder.join("stuck.txt")).unwrap(),
                    "stuck"
                );
            }
            assert!(!files.is_dir(&expected));
            assert_eq!(episode.warnings, []);
        }
    }

    /// With BSArch, the staging move, its list entry and the pack all go to the fresh sibling
    /// too, so nothing a stuck `ArchiveWork` holds can be packed.
    #[test]
    fn bsarch_stages_and_packs_in_the_next_free_work_folder() {
        let files = step_three_space();
        files.add_file_with_contents(work().join("staging").join("stale.nif"), "stale");
        let fresh = fallout4_dir().join("ArchiveWork.1");
        let wait = RecordingWait::new();
        let at_pack = RefCell::new(None);
        let process = RecordingProcessRunner::new().scripting_call(
            0,
            ScriptedCall::new().with_effect(&files, |space, _exe, args| {
                *at_pack.borrow_mut() = Some((
                    space.is_file(
                        &fresh
                            .join("staging")
                            .join("meshes")
                            .join("precombined")
                            .join("cell")
                            .join("mesh.nif"),
                    ),
                    listed(space, &fresh),
                ));
                space.add_file_with_contents(&args[2], PACKED_ARCHIVE);
            }),
        );

        let episode = run_episode(
            ArchiveTool::BSArch,
            &files,
            &process,
            &wait,
            archive_precombines,
        );

        episode.result.unwrap();
        assert_eq!(
            episode.calls[0].args[1],
            fresh.join("staging").into_os_string()
        );
        assert_eq!(
            at_pack.borrow().clone(),
            Some((
                true,
                vec![source_entry(
                    PathBuf::from(STAGING_FOLDER_NAME)
                        .join("meshes")
                        .join("precombined"),
                    loose_precombined(),
                )]
            ))
        );
        assert_eq!(
            files
                .read_lossy(&work().join("staging").join("stale.nif"))
                .unwrap(),
            "stale"
        );
        assert!(!files.is_dir(&fresh));
    }

    /// A work folder that cannot be removed after a successful step is a warning, not a stop.
    #[test]
    fn a_work_folder_that_cannot_be_removed_is_a_cleanup_warning() {
        let inner = step_three_space();
        let files = FaultyFileSpace::over(&inner).refusing_dir_removal(work());
        let wait = RecordingWait::new();
        let process =
            RecordingProcessRunner::new().scripting_call(0, bsarch_pack_writing_archive(&inner));

        let episode = run_episode(
            ArchiveTool::BSArch,
            &files,
            &process,
            &wait,
            archive_precombines,
        );

        episode.result.unwrap();
        assert_eq!(
            episode.warnings,
            [BuildWarning::ArchiveCleanupFailed { path: work() }]
        );
        assert_eq!(inner.read_lossy(&data_archive()).unwrap(), PACKED_ARCHIVE);
    }

    /// The loose meshes that cannot be removed after the Archive2 swap are a warning too.
    #[test]
    fn loose_meshes_left_after_the_archive2_swap_are_a_cleanup_warning() {
        let inner = step_three_space();
        let files = FaultyFileSpace::over(&inner).refusing_dir_removal(loose_precombined());
        let wait = RecordingWait::new();
        let process =
            RecordingProcessRunner::new().scripting_call(0, archive2_pack_writing_archive(&inner));

        let episode = run_episode(
            ArchiveTool::Archive2,
            &files,
            &process,
            &wait,
            archive_precombines,
        );

        episode.result.unwrap();
        assert_eq!(
            episode.warnings,
            [BuildWarning::ArchiveCleanupFailed {
                path: loose_precombined()
            }]
        );
        assert_eq!(inner.read_lossy(&data_archive()).unwrap(), PACKED_ARCHIVE);
    }

    /// On a failure path, a work folder that cannot be removed is still only a warning: the
    /// step's own error is what is returned.
    #[test]
    fn a_cleanup_failure_never_replaces_the_step_error() {
        let inner = step_three_space();
        let files = FaultyFileSpace::over(&inner).refusing_dir_removal(work());
        let wait = RecordingWait::new();
        let process = RecordingProcessRunner::new().returning_exit_code(1);

        let episode = run_episode(
            ArchiveTool::Archive2,
            &files,
            &process,
            &wait,
            archive_precombines,
        );

        assert!(
            matches!(episode.result, Err(Error::Archive2Failed { code: Some(1) })),
            "result: {:?}",
            episode.result
        );
        assert_eq!(
            episode.warnings,
            [BuildWarning::ArchiveCleanupFailed { path: work() }]
        );
    }

    // --- add_previs ----------------------------------------------------------------------

    /// Step 8 with Archive2: extract, wait 5s, repack precombines and `vis`, swap, then remove
    /// both loose folders.
    #[test]
    fn archive2_extracts_waits_and_repacks_with_previs() {
        let files = step_eight_space();
        let wait = RecordingWait::new();
        let delays_at_pack = RefCell::new(None);
        let process = RecordingProcessRunner::new()
            .scripting_call(
                0,
                archive2_extract_writing_precombines(&files, data(), false)
                    .with_stdout("Extracted"),
            )
            .scripting_call(
                1,
                ScriptedCall::new().with_stdout("Packed").with_effect(
                    &files,
                    |space, _exe, _args| {
                        *delays_at_pack.borrow_mut() = Some(wait.delays());
                        space.add_file_with_contents(work().join(NAME), PACKED_ARCHIVE);
                    },
                ),
            );

        let episode = run_episode(ArchiveTool::Archive2, &files, &process, &wait, add_previs);

        episode.result.unwrap();
        assert_eq!(
            episode.calls,
            vec![
                call(
                    ArchiveTool::Archive2,
                    &[OsStr::new(NAME), OsStr::new("-e=."), OsStr::new("-q")]
                ),
                call(
                    ArchiveTool::Archive2,
                    &[
                        OsStr::new(r"meshes\precombined,vis"),
                        &archive2_output_arg(&work()),
                        OsStr::new("-f=General"),
                        OsStr::new("-q"),
                    ]
                ),
            ]
        );
        assert_eq!(episode.delays, vec![MO2_DELAY_AFTER_ARCHIVE2_EXTRACT_SECS]);
        assert_eq!(
            delays_at_pack.borrow().as_deref(),
            Some([MO2_DELAY_AFTER_ARCHIVE2_EXTRACT_SECS].as_slice())
        );
        assert_eq!(
            files.read_lossy(&session_log()).unwrap(),
            format!(
                "{}Extracted\n{}Packed\n",
                extract_header(),
                pack_header(r"meshes\precombined,vis")
            )
        );
        assert_eq!(files.read_lossy(&data_archive()).unwrap(), PACKED_ARCHIVE);
        assert!(!files.is_dir(&loose_precombined()));
        assert!(!files.is_dir(&loose_vis()));
        assert!(!files.is_dir(&work()));
    }

    /// Step 8 with BSArch: unpack into staging, move the new `vis` in, wait 5s, pack, swap.
    #[test]
    fn bsarch_unpacks_stages_previs_and_repacks() {
        let files = step_eight_space();
        let wait = RecordingWait::new();
        let at_pack = RefCell::new(None);
        let process = RecordingProcessRunner::new()
            .scripting_call(0, bsarch_unpack_writing_precombines(&files, false))
            .scripting_call(
                1,
                ScriptedCall::new().with_effect(&files, |space, _exe, _args| {
                    *at_pack.borrow_mut() = Some((
                        wait.delays(),
                        space
                            .read_lossy(&staging().join("vis").join("new.uvd"))
                            .ok(),
                        space.is_file(&extracted_mesh(&staging())),
                    ));
                    space.add_file_with_contents(work().join(NAME), PACKED_ARCHIVE);
                }),
            );

        let episode = run_episode(ArchiveTool::BSArch, &files, &process, &wait, add_previs);

        episode.result.unwrap();
        assert_eq!(
            episode.calls,
            vec![
                call(
                    ArchiveTool::BSArch,
                    &[
                        OsStr::new("unpack"),
                        data_archive().as_os_str(),
                        staging().as_os_str(),
                    ]
                ),
                call(
                    ArchiveTool::BSArch,
                    &[
                        OsStr::new("pack"),
                        staging().as_os_str(),
                        work().join(NAME).as_os_str(),
                        OsStr::new("-mt"),
                        OsStr::new("-fo4"),
                        OsStr::new("-z"),
                    ]
                ),
            ]
        );
        assert_eq!(episode.delays, vec![MO2_DELAY_BEFORE_BSARCH_PACK_SECS]);
        assert_eq!(
            at_pack.borrow().clone(),
            Some((
                vec![MO2_DELAY_BEFORE_BSARCH_PACK_SECS],
                Some("new previs".to_string()),
                true
            ))
        );
        assert_eq!(
            files.read_lossy(&session_log()).unwrap(),
            format!(
                "{}{}",
                extract_header(),
                pack_header(r"meshes\precombined,vis")
            )
        );
        assert_eq!(files.read_lossy(&data_archive()).unwrap(), PACKED_ARCHIVE);
        assert!(!files.is_dir(&loose_vis()));
        assert!(!files.is_dir(&work()));
    }

    /// An unpack that also yields the archive's old `vis` has it removed before the new one
    /// moves in, so only the new cluster files are packed, and the unpacked copies are never
    /// listed.
    #[test]
    fn bsarch_replaces_the_archives_old_previs_with_the_new() {
        let files = step_eight_space();
        let wait = RecordingWait::new();
        let at_pack = RefCell::new(None);
        let process = RecordingProcessRunner::new()
            .scripting_call(0, bsarch_unpack_writing_precombines(&files, true))
            .scripting_call(
                1,
                ScriptedCall::new().with_effect(&files, |space, _exe, _args| {
                    *at_pack.borrow_mut() = Some((
                        space.is_file(&extracted_old_uvd(&staging())),
                        space.is_file(&staging().join("vis").join("new.uvd")),
                        listed(space, &work()),
                    ));
                    space.add_file_with_contents(work().join(NAME), PACKED_ARCHIVE);
                }),
            );

        let episode = run_episode(ArchiveTool::BSArch, &files, &process, &wait, add_previs);

        episode.result.unwrap();
        assert_eq!(
            at_pack.borrow().clone(),
            Some((
                false,
                true,
                vec![source_entry(
                    PathBuf::from(STAGING_FOLDER_NAME).join("vis"),
                    loose_vis()
                )]
            ))
        );
    }

    /// The unpacked old `vis` decides what the archive contains, so failing to remove it is a
    /// stop, before the new `vis` moves or the pack runs.
    #[test]
    fn an_unremovable_unpacked_old_previs_stops_the_step() {
        let inner = step_eight_space();
        let files = FaultyFileSpace::over(&inner).refusing_dir_removal(staging().join("vis"));
        let wait = RecordingWait::new();
        let process = RecordingProcessRunner::new()
            .scripting_call(0, bsarch_unpack_writing_precombines(&inner, true));

        let episode = run_episode(ArchiveTool::BSArch, &files, &process, &wait, add_previs);

        assert!(
            matches!(episode.result, Err(Error::Io(_))),
            "result: {:?}",
            episode.result
        );
        assert_eq!(episode.calls.len(), 1);
        assert_eq!(inner.read_lossy(&new_uvd()).unwrap(), "new previs");
        assert_eq!(inner.read_lossy(&data_archive()).unwrap(), OLD_ARCHIVE);
    }

    /// An archive with no precombined meshes is never rebuilt from `vis` alone: no pack, the old
    /// archive untouched, the work folder gone, and BSArch never moves `vis`.
    #[test]
    fn an_archive_without_precombines_is_never_rebuilt() {
        for tool in [ArchiveTool::Archive2, ArchiveTool::BSArch] {
            let files = step_eight_space();
            let wait = RecordingWait::new();
            // The extract or unpack succeeds and writes nothing.
            let process = RecordingProcessRunner::new();

            let episode = run_episode(tool, &files, &process, &wait, add_previs);

            assert!(
                matches!(
                    &episode.result,
                    Err(Error::PluginArchiveHasNoPrecombines { name }) if name == NAME
                ),
                "tool: {tool:?}, result: {:?}",
                episode.result
            );
            assert_eq!(episode.calls.len(), 1, "tool: {tool:?}");
            assert_eq!(files.read_lossy(&data_archive()).unwrap(), OLD_ARCHIVE);
            assert_eq!(files.read_lossy(&new_uvd()).unwrap(), "new previs");
            assert!(!files.is_dir(&work()), "tool: {tool:?}");
        }
    }

    /// A failed extract or unpack stops with its own error, before any pack.
    #[test]
    fn a_failed_extract_or_unpack_stops_with_its_exit_code() {
        for tool in [ArchiveTool::Archive2, ArchiveTool::BSArch] {
            let files = step_eight_space();
            let wait = RecordingWait::new();
            let process = RecordingProcessRunner::new().returning_exit_code(3);

            let episode = run_episode(tool, &files, &process, &wait, add_previs);

            let code = match episode.result {
                Err(Error::Archive2ExtractFailed { code }) if tool == ArchiveTool::Archive2 => code,
                Err(Error::BsarchUnpackFailed { code }) if tool == ArchiveTool::BSArch => code,
                other => panic!("tool: {tool:?}, result: {other:?}"),
            };
            assert_eq!(code, Some(3));
            assert_eq!(episode.calls.len(), 1);
            assert_eq!(episode.delays, Vec::<u64>::new());
            assert_eq!(files.read_lossy(&data_archive()).unwrap(), OLD_ARCHIVE);
            assert!(!files.is_dir(&work()), "tool: {tool:?}");
        }
    }

    /// A failed Step 8 pack leaves the old archive, and BSArch moves `vis` back while the
    /// unpacked precombines, a copy, go with the work folder.
    #[test]
    fn a_failed_rebuild_pack_restores_the_loose_previs() {
        for tool in [ArchiveTool::Archive2, ArchiveTool::BSArch] {
            let files = step_eight_space();
            let wait = RecordingWait::new();
            let extract = match tool {
                ArchiveTool::Archive2 => {
                    archive2_extract_writing_precombines(&files, data(), false)
                }
                ArchiveTool::BSArch => bsarch_unpack_writing_precombines(&files, false),
            };
            let process = RecordingProcessRunner::new()
                .scripting_call(0, extract)
                .scripting_call(1, ScriptedCall::new().exiting_with(2));

            let episode = run_episode(tool, &files, &process, &wait, add_previs);

            let code = match episode.result {
                Err(Error::Archive2Failed { code }) if tool == ArchiveTool::Archive2 => code,
                Err(Error::BsarchFailed { code }) if tool == ArchiveTool::BSArch => code,
                other => panic!("tool: {tool:?}, result: {other:?}"),
            };
            assert_eq!(code, Some(2));
            assert_eq!(files.read_lossy(&data_archive()).unwrap(), OLD_ARCHIVE);
            assert_eq!(files.read_lossy(&new_uvd()).unwrap(), "new previs");
            assert!(!files.is_dir(&work()), "tool: {tool:?}");
        }
    }

    /// The loose `vis` that cannot be removed after the Archive2 swap is a cleanup warning.
    #[test]
    fn loose_previs_left_after_the_archive2_swap_is_a_cleanup_warning() {
        let inner = step_eight_space();
        let files = FaultyFileSpace::over(&inner).refusing_dir_removal(loose_vis());
        let wait = RecordingWait::new();
        let process = RecordingProcessRunner::new()
            .scripting_call(
                0,
                archive2_extract_writing_precombines(&inner, data(), false),
            )
            .scripting_call(1, archive2_pack_writing_archive(&inner));

        let episode = run_episode(ArchiveTool::Archive2, &files, &process, &wait, add_previs);

        episode.result.unwrap();
        assert_eq!(
            episode.warnings,
            [BuildWarning::ArchiveCleanupFailed { path: loose_vis() }]
        );
        assert_eq!(inner.read_lossy(&data_archive()).unwrap(), PACKED_ARCHIVE);
        assert!(!inner.is_dir(&loose_precombined()));
    }

    // --- the work-folder rules -----------------------------------------------------------

    #[test]
    fn only_archive_work_and_its_numbered_siblings_are_work_folders() {
        for name in [
            "ArchiveWork",
            "archivework",
            "ArchiveWork.1",
            "ARCHIVEWORK.42",
        ] {
            assert!(is_work_folder_name(name), "{name}");
        }
        for name in [
            "ArchiveWorkspace",
            "ArchiveWork.",
            "ArchiveWork.bak",
            "ArchiveWork.1x",
            "ArchiveWork.orphaned.1",
            "Archive",
        ] {
            assert!(!is_work_folder_name(name), "{name}");
        }
    }

    #[test]
    fn a_restore_list_round_trips_and_ignores_blank_lines() {
        let entries = vec![
            source_entry(PathBuf::from(r"staging\vis"), loose_vis()),
            archive_entry(),
        ];
        let text = format!("\n{}\n{}", entries[0].line(), entries[1].line());

        assert_eq!(parse_restore_list(&text), Some(entries));
    }

    /// Two fields, an unknown kind, an absolute item, or an item that climbs out of the folder
    /// makes the whole list invalid.
    #[test]
    fn an_invalid_line_invalidates_the_whole_list() {
        let valid = archive_entry().line();
        let absolute = std::env::current_dir().unwrap().join("vis");
        for invalid in [
            "source\tstaging\\vis\n".to_string(),
            "moved\tstaging\\vis\tData\\vis\n".to_string(),
            format!("source\t{}\tData\\vis\n", absolute.display()),
            "source\t..\\vis\tData\\vis\n".to_string(),
            "source\tstaging\\vis\tData\\vis\textra\n".to_string(),
        ] {
            assert_eq!(
                parse_restore_list(&format!("{valid}{invalid}")),
                None,
                "{invalid}"
            );
        }
    }

    // --- the run-start restore, where it needs the episode or a real disk ----------------

    /// A crash round trip: a BSArch Step 3 whose swap fails, then a fresh run's restore over
    /// the same space, leaves the loose precombines back in `Data`, an archive in `Data`
    /// (never none), and no work folder.
    #[test]
    fn a_failed_swap_is_undone_by_the_next_runs_restore() {
        let inner = step_three_space();
        let files = FaultyFileSpace::over(&inner).refusing_rename_to(data_archive());
        let wait = RecordingWait::new();
        let process =
            RecordingProcessRunner::new().scripting_call(0, bsarch_pack_writing_archive(&inner));
        let failed = run_episode(
            ArchiveTool::BSArch,
            &files,
            &process,
            &wait,
            archive_precombines,
        );
        assert!(matches!(
            failed.result,
            Err(Error::ArchiveSwapFailed { .. })
        ));

        let warnings = BuildWarnings::new(session_log(), &inner);
        restore_archive_work_folders(&fallout4_dir(), &inner, &warnings).unwrap();

        assert_eq!(inner.read_lossy(&loose_mesh()).unwrap(), "mesh");
        // The swap deleted the old archive before its rename failed, so the restore moved the
        // listed new one in.
        assert_eq!(inner.read_lossy(&data_archive()).unwrap(), PACKED_ARCHIVE);
        assert!(!inner.is_dir(&work()));
        assert_eq!(
            warnings.raised(),
            [
                BuildWarning::ArchiveWorkRestored {
                    item: precombined_dir(&staging()),
                    target: loose_precombined(),
                },
                BuildWarning::ArchiveWorkRestored {
                    item: work().join(NAME),
                    target: data_archive(),
                },
            ]
        );
    }

    /// An empty leftover folder cannot be represented in memory, so it is pinned on disk: it is
    /// removed, with no warning.
    #[test]
    fn an_empty_leftover_folder_on_disk_is_removed() {
        let dir = tempfile::tempdir().unwrap();
        let leftover = dir.path().join("ArchiveWork.2");
        std::fs::create_dir_all(&leftover).unwrap();
        let files = SystemFileSpace;
        let warnings = BuildWarnings::new(dir.path().join("session.log"), &files);

        restore_archive_work_folders(dir.path(), &files, &warnings).unwrap();

        assert!(!leftover.exists());
        assert_eq!(warnings.raised(), []);
    }
}
