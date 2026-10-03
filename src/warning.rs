//! Build Warnings: conditions a Workflow Operation reports while the Workflow Run continues.
//!
//! [`BuildWarning`] is to a degraded-but-continuing build what [`crate::error::Error`] is to a
//! stop: one typed home for the batch's wording. `Display` carries the text *without* the
//! `WARNING - ` prefix, in the same convention as `Error`, whose `ERROR - ` prefix is added only
//! where it is printed.
//!
//! [`BuildWarnings`] is the collector a Workflow Operation raises them through. It is concrete
//! data in `OperationPorts` rather than a trait (ADR-0002): there is one way to raise a warning,
//! so an interface over it would be a hypothetical seam.

use std::cell::RefCell;
use std::fmt;
use std::path::PathBuf;

use crate::error::{Result, exit_code_text};
use crate::files::FileSpace;
use crate::logging;

/// A condition that degrades a build without stopping it.
///
/// Other specs add their own variants (Steps 2, 3, 7, 8 and Finish); each carries the batch's
/// wording in `Display`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BuildWarning {
    /// Creation Kit exited non-zero but the operation's postconditions passed (batch `:RunCK`
    /// line 472).
    ///
    /// `operation` is the batch verb (`GeneratePrecombined`, `CompressPSG`, …), supplied by the
    /// Creation Kit episode so a Workflow Operation never names one. `code` is `None` when the
    /// platform reports no exit code — a signal-terminated process off Windows.
    CreationKitNonZeroExit {
        operation: &'static str,
        code: Option<i32>,
    },
    /// The Creation Kit log reports `ERROR: visibility task did not complete.` after Step 6
    /// produced `Previs.esp` (batch 319–320): at least one cluster's `.uvd` is missing.
    VisibilityTaskIncomplete,
    /// The Step 2 merge script's log contains `Error: ` (batch 284–285): the merge finished,
    /// but the script reported something it could not merge.
    MergePrecombinesHadErrors,
    /// The Step 7 merge script's log lacks `Completed: No Errors.` (batch 327–328): the merge
    /// finished, but the script did not report a clean run. The inverse polarity of
    /// [`Self::MergePrecombinesHadErrors`], as the batch tests each log.
    MergePrevisHadErrors,
    /// The Archive episode could not remove something the Plugin Archive no longer depends on:
    /// its work folder after a step, or the loose `meshes\precombined` or `vis` after a swap.
    ///
    /// Never a stop. On a step that then fails, the step's own error is still what stops the
    /// run, so a cleanup problem never hides why it stopped.
    ArchiveCleanupFailed { path: PathBuf },
    /// The run-start restore found nothing to restore in a leftover `ArchiveWork*` folder, but
    /// could not remove it. Each archive step then builds in the next free name instead.
    ArchiveWorkFolderNotCleared { path: PathBuf },
    /// The run-start restore moved `item`, which only a leftover work folder held, back to
    /// `target` in `Data`. A warning because files moved under the operator, who should know
    /// which step to rerun.
    ArchiveWorkRestored { item: PathBuf, target: PathBuf },
    /// The run-start restore could not put a leftover work folder's `items` back, so it renamed
    /// the folder `from` to `to` rather than delete the only copy of anything.
    ArchiveWorkSetAside {
        from: PathBuf,
        to: PathBuf,
        items: LeftoverItems,
    },
    /// As [`Self::ArchiveWorkSetAside`], but the rename failed too, so the folder at `path` was
    /// left where it is. It is never removed.
    ArchiveWorkNotRestored { path: PathBuf, items: LeftoverItems },
}

/// What a leftover archive work folder still holds when the run-start restore gives up on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LeftoverItems {
    /// The listed items still in the folder, as paths relative to it.
    Listed(Vec<PathBuf>),
    /// The folder's restore list could not be read or parsed, so what it holds is unknown.
    UnreadableList,
}

impl fmt::Display for LeftoverItems {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Listed(items) => {
                let items = items
                    .iter()
                    .map(|item| item.display().to_string())
                    .collect::<Vec<_>>();
                f.write_str(&items.join(", "))
            }
            Self::UnreadableList => {
                f.write_str("whatever its restore list names (the list could not be read)")
            }
        }
    }
}

impl fmt::Display for BuildWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ArchiveCleanupFailed { path } => {
                write!(f, "Could not remove {}. Remove it by hand.", path.display())
            }
            Self::ArchiveWorkFolderNotCleared { path } => write!(
                f,
                "Could not remove leftover archive work folder {}. Remove it by hand.",
                path.display()
            ),
            Self::ArchiveWorkRestored { item, target } => write!(
                f,
                "Moved {} back to {}. An earlier run stopped before it had finished with it.",
                item.display(),
                target.display()
            ),
            Self::ArchiveWorkSetAside { from, to, items } => write!(
                f,
                "Set leftover archive work folder {} aside as {}. It still holds {items}, which \
                 could not be moved back into Data. Recover them, then delete the folder.",
                from.display(),
                to.display()
            ),
            Self::ArchiveWorkNotRestored { path, items } => write!(
                f,
                "Leftover archive work folder {} still holds {items}, which could not be moved \
                 back into Data or set aside. Recover them by hand.",
                path.display()
            ),
            Self::MergePrecombinesHadErrors => f.write_str("Merge Precombines had errors"),
            Self::MergePrevisHadErrors => f.write_str("Merge Previs had errors"),
            Self::VisibilityTaskIncomplete => {
                f.write_str("GeneratePreVisData failed to build at least one Cluster uvd")
            }
            Self::CreationKitNonZeroExit { operation, code } => write!(
                f,
                "{operation} ended with error {} but seemed to finish so error ignored.",
                // Shared with the missing-output error, so "unknown" has one home.
                exit_code_text(*code)
            ),
        }
    }
}

/// The Build Warnings raised during one Workflow Run, surfaced as they are raised.
///
/// Interior mutability because the ports a Workflow Operation receives are shared references.
/// Not `Sync`, and it does not need to be: a Workflow Run dispatches its operations on one
/// thread.
#[derive(Debug)]
pub(crate) struct BuildWarnings<'a> {
    session_log: PathBuf,
    files: &'a dyn FileSpace,
    raised: RefCell<Vec<BuildWarning>>,
}

impl<'a> BuildWarnings<'a> {
    /// A collector that has raised nothing yet and appends to `session_log` through `files`.
    #[must_use]
    pub(crate) fn new(session_log: PathBuf, files: &'a dyn FileSpace) -> Self {
        Self {
            session_log,
            files,
            raised: RefCell::new(Vec::new()),
        }
    }

    /// Raise `warning`: keep it, print `WARNING - <text>`, and append that line to the session
    /// log.
    ///
    /// Kept *first*, before anything that can fail, so a warning is never lost to a failed
    /// append: the caller sees the `Io` error, but anything reading [`Self::raised`] afterwards
    /// (tests, and later Finish) still sees every warning that was raised.
    ///
    /// Returns [`crate::error::Error::Io`] when the session log cannot be extended, matching
    /// every other session-log append a Creation Kit episode makes.
    pub(crate) fn raise(&self, warning: BuildWarning) -> Result<()> {
        let line = logging::warning_line(&warning);
        self.raised.borrow_mut().push(warning);
        println!("{line}");
        logging::append_log_line(&self.session_log, &line, self.files)
    }

    /// Every warning raised so far, in the order it was raised.
    // Read only by tests until the Finish epilogue lands, which reports on the run's warnings.
    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "the Finish epilogue that reads the raised warnings is not ported yet"
        )
    )]
    #[must_use]
    pub(crate) fn raised(&self) -> Vec<BuildWarning> {
        self.raised.borrow().clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::files::InMemoryFileSpace;

    fn non_zero_exit(code: Option<i32>) -> BuildWarning {
        BuildWarning::CreationKitNonZeroExit {
            operation: "GeneratePrecombined",
            code,
        }
    }

    /// Batch line 472, minus the `WARNING - ` prefix that is added where it is printed.
    #[test]
    fn the_non_zero_exit_warning_reads_as_the_batch_wording() {
        assert_eq!(
            non_zero_exit(Some(3)).to_string(),
            "GeneratePrecombined ended with error 3 but seemed to finish so error ignored."
        );
    }

    #[test]
    fn a_missing_exit_code_renders_as_unknown_rather_than_panicking() {
        assert_eq!(
            non_zero_exit(None).to_string(),
            "GeneratePrecombined ended with error unknown but seemed to finish so error ignored."
        );
    }

    /// Batch line 320, minus the `WARNING - ` prefix.
    #[test]
    fn the_visibility_task_warning_reads_as_the_batch_wording() {
        assert_eq!(
            BuildWarning::VisibilityTaskIncomplete.to_string(),
            "GeneratePreVisData failed to build at least one Cluster uvd"
        );
    }

    /// Batch line 285, minus the `WARNING - ` prefix.
    #[test]
    fn the_merge_precombines_warning_reads_as_the_batch_wording() {
        assert_eq!(
            BuildWarning::MergePrecombinesHadErrors.to_string(),
            "Merge Precombines had errors"
        );
    }

    /// Batch line 328, minus the `WARNING - ` prefix.
    #[test]
    fn the_merge_previs_warning_reads_as_the_batch_wording() {
        assert_eq!(
            BuildWarning::MergePrevisHadErrors.to_string(),
            "Merge Previs had errors"
        );
    }

    #[test]
    fn the_archive_cleanup_warnings_name_the_path_to_remove_by_hand() {
        let path = PathBuf::from(r"C:\Fallout4\ArchiveWork");

        assert_eq!(
            BuildWarning::ArchiveCleanupFailed { path: path.clone() }.to_string(),
            format!("Could not remove {}. Remove it by hand.", path.display())
        );
        assert_eq!(
            BuildWarning::ArchiveWorkFolderNotCleared { path: path.clone() }.to_string(),
            format!(
                "Could not remove leftover archive work folder {}. Remove it by hand.",
                path.display()
            )
        );
    }

    #[test]
    fn the_run_start_restore_warnings_say_what_moved_and_where() {
        let from = PathBuf::from(r"C:\Fallout4\ArchiveWork");
        let to = PathBuf::from(r"C:\Fallout4\ArchiveWork.orphaned.1");
        let items = LeftoverItems::Listed(vec![
            PathBuf::from(r"staging\vis"),
            PathBuf::from("MyMod - Main.ba2"),
        ]);

        assert_eq!(
            BuildWarning::ArchiveWorkRestored {
                item: from.join("staging").join("vis"),
                target: PathBuf::from(r"C:\Fallout4\Data\vis"),
            }
            .to_string(),
            format!(
                "Moved {} back to C:\\Fallout4\\Data\\vis. An earlier run stopped before it had \
                 finished with it.",
                from.join("staging").join("vis").display()
            )
        );
        assert_eq!(
            BuildWarning::ArchiveWorkSetAside {
                from: from.clone(),
                to: to.clone(),
                items: items.clone(),
            }
            .to_string(),
            "Set leftover archive work folder C:\\Fallout4\\ArchiveWork aside as \
             C:\\Fallout4\\ArchiveWork.orphaned.1. It still holds staging\\vis, MyMod - Main.ba2, \
             which could not be moved back into Data. Recover them, then delete the folder."
        );
        assert_eq!(
            BuildWarning::ArchiveWorkNotRestored {
                path: from,
                items: LeftoverItems::UnreadableList,
            }
            .to_string(),
            "Leftover archive work folder C:\\Fallout4\\ArchiveWork still holds whatever its \
             restore list names (the list could not be read), which could not be moved back \
             into Data or set aside. Recover them by hand."
        );
    }

    #[test]
    fn raising_keeps_the_warning_and_appends_it_to_the_session_log() {
        let files = InMemoryFileSpace::new();
        let session_log = files.temp_dir().join("MyMod.log");
        files.add_file_with_contents(&session_log, "Starting clean Build V2.99 of MyMod.esp\n");
        let warnings = BuildWarnings::new(session_log.clone(), &files);

        warnings.raise(non_zero_exit(Some(1))).unwrap();
        warnings.raise(non_zero_exit(Some(2))).unwrap();

        assert_eq!(
            warnings.raised(),
            vec![non_zero_exit(Some(1)), non_zero_exit(Some(2))]
        );
        assert_eq!(
            files.read_lossy(&session_log).unwrap(),
            "Starting clean Build V2.99 of MyMod.esp\n\
             WARNING - GeneratePrecombined ended with error 1 but seemed to finish so error ignored.\n\
             WARNING - GeneratePrecombined ended with error 2 but seemed to finish so error ignored.\n"
        );
    }

    /// Keep-before-emit: a failed append surfaces as an error but loses no warning.
    #[test]
    fn a_failed_append_still_keeps_the_warning() {
        let files = InMemoryFileSpace::new();
        let session_log = files.temp_dir().join("MyMod.log");
        files.refuse_appends_to(&session_log);
        let warnings = BuildWarnings::new(session_log, &files);

        let error = warnings.raise(non_zero_exit(Some(1))).unwrap_err();

        assert!(matches!(error, crate::error::Error::Io(_)));
        assert_eq!(warnings.raised(), vec![non_zero_exit(Some(1))]);
    }
}
