//! Session logging to a temp file (batch uses `%TEMP%\%PluginName%.log`).
//!
//! Every path and every byte here goes through the [`FileSpace`] seam rather than `std::fs`.
//! The log location used to come from the process-global `std::env::temp_dir()`, which put
//! the machine's real `%TEMP%` in the path of any test that prepared a Workflow Run — and
//! several test modules build one for the same plugin name, so they collided on a single
//! file outside their own fixtures.

use std::path::{Path, PathBuf};

use crate::config::PluginIdentity;
use crate::error::Result;
use crate::files::FileSpace;

/// Path for the per-run log file, rooted in `files`' temporary directory.
#[must_use]
pub fn session_log_path(plugin: &PluginIdentity, files: &dyn FileSpace) -> PathBuf {
    files.temp_dir().join(format!("{}.log", plugin.base_name))
}

/// Path matching batch unattended xEdit log location, rooted in `files`' temporary directory.
// No production caller since the Creation Kit paths stopped travelling in a shared tool
// context: this log belongs to the xEdit runs, and the Workflow Operations that launch them
// are not ported. It stays live through its test until the step 2 xEdit adapter names it
// again (deferred to Phase B by ADR-0002).
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "the unattended xEdit log arrives with the xEdit-backed Workflow Operations"
    )
)]
#[must_use]
pub fn unattended_log_path(files: &dyn FileSpace) -> PathBuf {
    files.temp_dir().join("UnattendedScript.log")
}

/// Append a banner line, plus its line terminator, to the session log at `path`.
///
/// Creates the file when it is absent. Returns [`crate::error::Error::Io`] when `files`
/// cannot extend it.
pub fn append_log_line(path: &Path, line: &str, files: &dyn FileSpace) -> Result<()> {
    files.append(path, &format!("{line}\n"))
}

/// Batch-aligned session log header (`:Precomb2`, batch line 260).
///
/// The `V2.95` here is **not** the batch reference version — that is V2.98, as the author
/// header (line 15) and the banner (line 18) say. It is hardcoded into this one `echo` and never
/// updated it as the script moved on. This is a literal reproduction of a line the batch writes
/// to the session log, so it matches the batch's *output*, not the batch's *version*. Anything
/// parsing or diffing a session log against a batch-produced one depends on that. Bump it only
/// if the batch's own line 260 changes.
#[must_use]
pub fn build_session_header(build_mode: &str, plugin_file: &str) -> String {
    format!("Starting {build_mode} Build V2.95 of {plugin_file}")
}

/// Initialize the session log at `path` with the build header (batch `Starting %BuildMode_% Build`).
///
/// Replaces any log left by a previous run rather than appending to it. Returns
/// [`crate::error::Error::Io`] when `files` cannot write the file.
pub fn init_session_log(
    path: &Path,
    build_mode: &str,
    plugin_file: &str,
    files: &dyn FileSpace,
) -> Result<()> {
    let header = build_session_header(build_mode, plugin_file);
    files.write(path, &format!("{header}\n"))
}

/// The batch's rule between the run banner and the `Start` line (`:RunCK` line 453).
///
/// Thirty-six `=`, as the batch's `echo` writes them, so a session log this port produces lines
/// up against one the batch produced. Trailing whitespace is the one thing not reproduced: every
/// `echo … >> "%Logfile_%"` in `:RunCK` leaves a space before the redirect, and `init_session_log`
/// already drops the batch header's two.
const CK_RUN_SEPARATOR: &str = "====================================";

/// Open one Creation Kit run's session-log entry (batch `:RunCK` lines 452–454).
///
/// `operation` is the batch's `%1` — `GeneratePrecombined` and friends. Four runs share one
/// session log across a build, so this is what attributes everything below it to a step.
/// `started_at` is a local time of day, not a duration; see [`crate::tools`]' `Clock`.
///
/// Written *before* the spawn, exactly as the batch writes it, and that placement is the point
/// rather than an implementation detail: it is what leaves a record behind when Creation Kit
/// crashes, hangs, or never launches at all. Deferring the whole entry until the run returned
/// would lose precisely the runs worth recording.
///
/// Returns [`crate::error::Error::Io`] when the session log cannot be extended.
pub fn append_ck_run_header(
    session_log: &Path,
    operation: &str,
    started_at: &str,
    files: &dyn FileSpace,
) -> Result<()> {
    files.append(
        session_log,
        &format!("Running CK option {operation}:\n{CK_RUN_SEPARATOR}\nStart {started_at}\n"),
    )
}

/// Close a Creation Kit run's timing bracket (batch `:RunCK` line 457).
///
/// A named function rather than an [`append_log_line`] call at the adapter: every literal the
/// session log reproduces from the batch lives in this module, so there is one place to check a
/// wording change against. Written after the spawn returns and before the mandated MO2 delay,
/// which is where the batch writes it — the bracket measures Creation Kit, not the workaround.
///
/// Returns [`crate::error::Error::Io`] when the session log cannot be extended.
pub fn append_ck_run_ended(
    session_log: &Path,
    ended_at: &str,
    files: &dyn FileSpace,
) -> Result<()> {
    append_log_line(session_log, &format!("Ended {ended_at}"), files)
}

/// Append a Creation Kit run's log, or record that there was none (batch `:RunCK` lines 460–461).
///
/// Takes `contents` rather than a path so the Creation Kit adapter reads its log exactly once
/// and uses that one read for both this append and the content it hands back to the Workflow
/// Operation. `None` — Creation Kit wrote no log at all — becomes the batch's `Unable to find
/// log` line, which is the only thing distinguishing "Creation Kit ran and said nothing" from
/// "Creation Kit never ran"; `ck_log_path` is the path that line names.
///
/// Returns [`crate::error::Error::Io`] when the session log cannot be extended.
pub fn append_ck_log(
    session_log: &Path,
    contents: Option<&str>,
    ck_log_path: &Path,
    files: &dyn FileSpace,
) -> Result<()> {
    let Some(contents) = contents else {
        // Batch line 460, two spaces before the path included.
        return append_log_line(
            session_log,
            &format!("Unable to find log  {}", ck_log_path.display()),
            files,
        );
    };

    // Assembled before the single append so the log and the newline that normalises it are one
    // write, whatever `FileSpace` backs it. An empty log is left alone: the batch's `type` of an
    // empty file appends nothing, and a blank line would claim content that is not there.
    if contents.is_empty() || contents.ends_with('\n') {
        return files.append(session_log, contents);
    }

    files.append(session_log, &format!("{contents}\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::files::{InMemoryFileSpace, SystemFileSpace};

    #[test]
    fn session_log_uses_plugin_base_name_under_the_space_temp_dir() {
        let files = InMemoryFileSpace::new();
        let plugin = PluginIdentity::parse("MyMod.esp");

        let path = session_log_path(&plugin, &files);

        // Rooted in the caller's space rather than the process-global temp directory, which
        // is what keeps two runs of the same plugin name out of one shared file.
        assert_eq!(path, files.temp_dir().join("MyMod.log"));
    }

    #[test]
    fn unattended_log_sits_beside_the_session_log() {
        let files = InMemoryFileSpace::new();

        assert_eq!(
            unattended_log_path(&files),
            files.temp_dir().join("UnattendedScript.log")
        );
    }

    #[test]
    fn session_header_matches_batch_version() {
        let header = build_session_header("clean", "MyMod.esp");
        assert!(header.contains("V2.95"));
        assert!(header.contains("MyMod.esp"));
    }

    /// A second run of the same plugin starts a fresh log rather than growing the old one.
    #[test]
    fn init_session_log_replaces_a_previous_run() {
        let files = InMemoryFileSpace::new();
        let log = files.temp_dir().join("MyMod.log");

        init_session_log(&log, "clean", "MyMod.esp", &files).unwrap();
        init_session_log(&log, "filtered", "MyMod.esp", &files).unwrap();

        assert_eq!(
            files.read_lossy(&log).unwrap(),
            "Starting filtered Build V2.95 of MyMod.esp\n"
        );
    }

    #[test]
    fn append_log_line_terminates_each_line() {
        let files = InMemoryFileSpace::new();
        let log = files.temp_dir().join("MyMod.log");

        append_log_line(&log, "Precombine complete", &files).unwrap();
        append_log_line(&log, "Previs complete", &files).unwrap();

        assert_eq!(
            files.read_lossy(&log).unwrap(),
            "Precombine complete\nPrevis complete\n"
        );
    }

    /// A Creation Kit log path for the framing tests; only its spelling is under assertion.
    fn ck_log() -> PathBuf {
        PathBuf::from("Fallout4").join("CK.log")
    }

    /// Write one whole run's entry, the way the Creation Kit adapter writes it.
    ///
    /// Three appends rather than one, in the batch's own order: the header goes down before
    /// Creation Kit is launched, `Ended` when it returns, the log after the MO2 delay.
    fn append_whole_run(
        session_log: &Path,
        operation: &str,
        contents: Option<&str>,
        files: &dyn FileSpace,
    ) {
        append_ck_run_header(session_log, operation, "09:00:00.00", files).unwrap();
        append_ck_run_ended(session_log, "09:04:12.34", files).unwrap();
        append_ck_log(session_log, contents, &ck_log(), files).unwrap();
    }

    /// The whole `:RunCK` entry, asserted exactly: which operation ran, the batch's rule, both
    /// timestamps, then the log.
    #[test]
    fn a_creation_kit_run_is_framed_by_its_operation_and_timestamps() {
        let files = InMemoryFileSpace::new();
        let session_log = files.temp_dir().join("MyMod.log");
        init_session_log(&session_log, "clean", "MyMod.esp", &files).unwrap();

        append_whole_run(
            &session_log,
            "GeneratePrecombined",
            Some("Masterfile: Fallout4.esm\n"),
            &files,
        );

        assert_eq!(
            files.read_lossy(&session_log).unwrap(),
            "Starting clean Build V2.95 of MyMod.esp\n\
             Running CK option GeneratePrecombined:\n\
             ====================================\n\
             Start 09:00:00.00\n\
             Ended 09:04:12.34\n\
             Masterfile: Fallout4.esm\n"
        );
    }

    /// Creation Kit wrote no log: the entry still lands, and says so by name.
    ///
    /// This is the case the missing framing hurt most — a crashed Creation Kit used to leave
    /// nothing behind at all, so the session log could not distinguish it from a run that never
    /// happened.
    #[test]
    fn a_missing_creation_kit_log_is_named_in_the_session_log() {
        let files = InMemoryFileSpace::new();
        let session_log = files.temp_dir().join("MyMod.log");

        append_whole_run(&session_log, "GeneratePrecombined", None, &files);

        assert_eq!(
            files.read_lossy(&session_log).unwrap(),
            format!(
                "Running CK option GeneratePrecombined:\n\
                 ====================================\n\
                 Start 09:00:00.00\n\
                 Ended 09:04:12.34\n\
                 Unable to find log  {}\n",
                ck_log().display()
            )
        );
    }

    /// The header alone is a complete record of a run that never came back.
    ///
    /// A Creation Kit that hangs or is killed leaves exactly this, because the batch writes
    /// lines 452–454 before `START` rather than after it. Nothing downstream gets to append.
    #[test]
    fn a_run_that_never_returns_still_leaves_its_header() {
        let files = InMemoryFileSpace::new();
        let session_log = files.temp_dir().join("MyMod.log");

        append_ck_run_header(&session_log, "GeneratePreVisData", "09:00:00.00", &files).unwrap();

        assert_eq!(
            files.read_lossy(&session_log).unwrap(),
            "Running CK option GeneratePreVisData:\n\
             ====================================\n\
             Start 09:00:00.00\n"
        );
    }

    /// Two runs in one session log stay attributable to their own operations.
    #[test]
    fn consecutive_runs_each_get_their_own_entry() {
        let files = InMemoryFileSpace::new();
        let session_log = files.temp_dir().join("MyMod.log");

        append_whole_run(&session_log, "GeneratePrecombined", Some("first\n"), &files);
        append_whole_run(&session_log, "BuildCDX", Some("second\n"), &files);

        assert_eq!(
            files.read_lossy(&session_log).unwrap(),
            "Running CK option GeneratePrecombined:\n\
             ====================================\n\
             Start 09:00:00.00\n\
             Ended 09:04:12.34\n\
             first\n\
             Running CK option BuildCDX:\n\
             ====================================\n\
             Start 09:00:00.00\n\
             Ended 09:04:12.34\n\
             second\n"
        );
    }

    /// A Creation Kit log with no final newline still ends on one, so the next run's banner
    /// starts on its own line.
    #[test]
    fn append_ck_log_normalises_a_missing_trailing_newline() {
        let files = InMemoryFileSpace::new();
        let session_log = files.temp_dir().join("MyMod.log");

        append_ck_log(&session_log, Some("truncated"), &ck_log(), &files).unwrap();

        assert_eq!(files.read_lossy(&session_log).unwrap(), "truncated\n");
    }

    /// An empty log gets no invented blank line — the batch's `type` of an empty file writes
    /// nothing, and a blank line would claim content Creation Kit did not produce.
    #[test]
    fn an_empty_creation_kit_log_adds_no_line_of_its_own() {
        let files = InMemoryFileSpace::new();
        let session_log = files.temp_dir().join("MyMod.log");

        append_ck_run_ended(&session_log, "09:04:12.34", &files).unwrap();
        append_ck_log(&session_log, Some(""), &ck_log(), &files).unwrap();

        assert_eq!(
            files.read_lossy(&session_log).unwrap(),
            "Ended 09:04:12.34\n"
        );
    }

    /// Pinned on [`SystemFileSpace`]: the framing has to survive a real file, and this is the
    /// one place the session log is written through the standard-library adapter.
    ///
    /// Tolerance for the non-UTF-8 bytes Creation Kit emits is no longer asserted here — these
    /// functions no longer read the log. That property now belongs to
    /// `SystemFileSpace::read_lossy`, where `files::tests` pins it.
    #[test]
    fn a_creation_kit_run_frames_through_the_system_space_too() {
        let dir = tempfile::tempdir().unwrap();
        let files = SystemFileSpace;
        let session_log = dir.path().join("session.log");

        append_whole_run(
            &session_log,
            "GeneratePrecombined",
            Some("ok\u{fffd}\n"),
            &files,
        );

        let session = std::fs::read_to_string(session_log).unwrap();
        assert!(session.contains("Running CK option GeneratePrecombined:"));
        assert!(session.contains("Start 09:00:00.00"));
        assert!(session.contains("Ended 09:04:12.34"));
        assert!(session.contains("ok"));
    }
}
