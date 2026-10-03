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
}

impl fmt::Display for BuildWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
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
