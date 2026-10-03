//! Session logging to a temp file (batch uses `%TEMP%\%PluginName%.log`).
//!
//! Every path and every byte here goes through the [`FileSpace`] seam rather than `std::fs`.
//! The log location used to come from the process-global `std::env::temp_dir()`, which put
//! the machine's real `%TEMP%` in the path of any test that prepared a Workflow Run — and
//! several test modules build one for the same plugin name, so they collided on a single
//! file outside their own fixtures.

use std::path::{Path, PathBuf};

use crate::config::PluginIdentity;
use crate::error::{Error, Result};
use crate::files::FileSpace;
use crate::warning::BuildWarning;

/// Path for the per-run log file, rooted in `files`' temporary directory.
#[must_use]
pub fn session_log_path(plugin: &PluginIdentity, files: &dyn FileSpace) -> PathBuf {
    files.temp_dir().join(format!("{}.log", plugin.base_name))
}

/// The file name of the log FO4Edit's merge scripts write (batch `%UnattenedLogfile_%`).
///
/// A bare name, because the FO4Edit episode passes it to `-log:` relative to FO4Edit's working
/// directory; see [`unattended_log_path`] for where that puts it.
pub(crate) const UNATTENDED_LOG_FILE_NAME: &str = "UnattendedScript.log";

/// Path matching batch unattended xEdit log location, rooted in `files`' temporary directory.
///
/// The FO4Edit episode runs FO4Edit with `files`' temporary directory as its working directory
/// and a relative `-log:`, so this is where the log lands.
#[must_use]
pub fn unattended_log_path(files: &dyn FileSpace) -> PathBuf {
    files.temp_dir().join(UNATTENDED_LOG_FILE_NAME)
}

/// Append a banner line, plus its line terminator, to the session log at `path`.
///
/// Creates the file when it is absent. Returns [`crate::error::Error::Io`] when `files`
/// cannot extend it.
pub fn append_log_line(path: &Path, line: &str, files: &dyn FileSpace) -> Result<()> {
    files.append(path, &format!("{line}\n"))
}

/// Batch-aligned session log header (`:Precomb2`, batch line 266).
///
/// The version here is a literal reproduction of the one hardcoded into this `echo`, not a copy
/// of the batch reference version. The two used to differ: through V2.98 the batch wrote `V2.95`
/// here while its author header (line 15) and banner (line 18) had moved on. V2.99 brought this
/// line back in step. Matching the batch's *output* is the point, because anything parsing or
/// diffing a session log against a batch-produced one depends on it. Bump it only if the
/// batch's own line 266 changes, whatever its header says.
#[must_use]
pub fn build_session_header(build_mode: &str, plugin_file: &str) -> String {
    format!("Starting {build_mode} Build V2.99 of {plugin_file}")
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

/// The batch's rule between the run banner and the `Start` line (`:RunCK` line 462).
///
/// Thirty-six `=`, as the batch's `echo` writes them, so a session log this port produces lines
/// up against one the batch produced. Trailing whitespace is the one thing not reproduced: every
/// `echo … >> "%Logfile_%"` in `:RunCK` leaves a space before the redirect, and `init_session_log`
/// already drops the batch header's two.
///
/// `:RunScript` writes the same rule under its FO4Edit banner (line 541), so
/// [`append_xedit_run_header`] uses it too.
const CK_RUN_SEPARATOR: &str = "====================================";

/// Open one Creation Kit run's session-log entry (batch `:RunCK` lines 461–463).
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

/// Close a Creation Kit run's timing bracket (batch `:RunCK` line 466).
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

/// Append a Creation Kit run's log, or record that there was none (batch `:RunCK` lines 469–470).
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
        // Batch line 469, two spaces before the path included.
        return append_log_line(
            session_log,
            &format!("Unable to find log  {}", ck_log_path.display()),
            files,
        );
    };

    append_tool_log_contents(session_log, contents, files)
}

/// Open one FO4Edit script run's session-log entry (batch `:RunScript` lines 539–541).
///
/// `script` is the bare `.pas` name and `plugins_txt` the plugin list FO4Edit is pointed at.
/// `data_dir_override` adds the batch's `%ModDir_%` (` -D:"<data>"`), which is set only when
/// `-FO4:` was given. The quotes are the batch's, written into the log as it writes them; they
/// never reach FO4Edit's argv.
///
/// Written before the episode's first delay, so a FO4Edit that hangs or never launches still
/// leaves a record of which script was attempted, the same reason the Creation Kit header is
/// written before its spawn. Trailing spaces from the batch's `echo … >>` are dropped, as
/// [`CK_RUN_SEPARATOR`] explains.
///
/// Returns [`crate::error::Error::Io`] when the session log cannot be extended.
pub(crate) fn append_xedit_run_header(
    session_log: &Path,
    script: &str,
    plugin_file: &str,
    plugins_txt: &Path,
    data_dir_override: Option<&Path>,
    files: &dyn FileSpace,
) -> Result<()> {
    let data_dir = data_dir_override
        .map(|data_dir| format!(" -D:\"{}\"", data_dir.display()))
        .unwrap_or_default();
    files.append(
        session_log,
        &format!(
            "Running xEdit script {script} against {plugin_file}\n\
             Params -fo4 -autoexit -P:\"{}\"{data_dir}\n\
             {CK_RUN_SEPARATOR}\n",
            plugins_txt.display()
        ),
    )
}

/// Append FO4Edit's unattended log to the session log (batch `:RunScript` line 559).
///
/// The same newline normalisation as [`append_ck_log`]: a log with no final newline gets one,
/// so whatever follows it starts on its own line, and an empty log adds nothing.
///
/// Returns [`crate::error::Error::Io`] when the session log cannot be extended.
pub(crate) fn append_unattended_log(
    session_log: &Path,
    contents: &str,
    files: &dyn FileSpace,
) -> Result<()> {
    append_tool_log_contents(session_log, contents, files)
}

/// Open one archive pack's session-log entry (batch `:Archive` lines 392–393 and 426).
///
/// `sources` is what is packed, as the batch prints it: `meshes\precombined`, or
/// `meshes\precombined,vis` for the Step 8 rebuild. The doubled space the batch leaves where the
/// empty `%Arch2Quals_%` sits is dropped, as the FO4Edit header drops `echo`'s trailing spaces.
///
/// Written before the tool runs and before any move or wait, so a pack that hangs still leaves
/// a record of what was attempted.
///
/// Returns [`crate::error::Error::Io`] when the session log cannot be extended.
pub(crate) fn append_archive_pack_header(
    session_log: &Path,
    archive_name: &str,
    sources: &str,
    files: &dyn FileSpace,
) -> Result<()> {
    files.append(
        session_log,
        &format!("Creating Archive {archive_name} of {sources}:\n{CK_RUN_SEPARATOR}\n"),
    )
}

/// Open one archive extract's session-log entry (batch `:Extract` line 415).
///
/// Also written before BSArch's `unpack`, which the batch never runs but which takes the
/// extract's place in the port's Step 8 rebuild. Placed before the tool runs for the same reason
/// as [`append_archive_pack_header`].
///
/// Returns [`crate::error::Error::Io`] when the session log cannot be extended.
pub(crate) fn append_archive_extract_header(
    session_log: &Path,
    archive_name: &str,
    files: &dyn FileSpace,
) -> Result<()> {
    files.append(
        session_log,
        &format!("Extracting Archive {archive_name}:\n{CK_RUN_SEPARATOR}\n"),
    )
}

/// Fold one archive tool run's captured output into the session log: stdout, then stderr.
///
/// Each stream gets the same newline normalisation as [`append_ck_log`], and an empty one adds
/// nothing. There is no stream label. Keeping stderr at all is a divergence: the batch's
/// `>> "%Logfile_%"` keeps stdout only, which loses Archive2's `-1` stack traces.
///
/// Returns [`crate::error::Error::Io`] when the session log cannot be extended.
pub(crate) fn append_archive_tool_output(
    session_log: &Path,
    stdout: &str,
    stderr: &str,
    files: &dyn FileSpace,
) -> Result<()> {
    append_tool_log_contents(session_log, stdout, files)?;
    append_tool_log_contents(session_log, stderr, files)
}

/// Append an external tool's log `contents` so the session log ends on a newline.
///
/// The one normalisation every folded tool log shares: Creation Kit's log, FO4Edit's unattended
/// log, and the archive tools' captured output.
fn append_tool_log_contents(
    session_log: &Path,
    contents: &str,
    files: &dyn FileSpace,
) -> Result<()> {
    // Assembled before the single append so the log and the newline that normalises it are one
    // write, whatever `FileSpace` backs it. An empty log is left alone: the batch's `type` of an
    // empty file appends nothing, and a blank line would claim content that is not there.
    if contents.is_empty() || contents.ends_with('\n') {
        return files.append(session_log, contents);
    }

    files.append(session_log, &format!("{contents}\n"))
}

/// The console and session-log line for a raised Build Warning (batch `WARNING - …`, line 472).
///
/// One line, two destinations: the collector prints exactly what it appends, so the console and
/// the session log can never disagree on a warning's wording.
#[must_use]
pub(crate) fn warning_line(warning: &BuildWarning) -> String {
    format!("WARNING - {warning}")
}

/// The line that opens a stopped Workflow Run's report (batch `ERROR - …`, e.g. line 471).
#[must_use]
pub(crate) fn error_line(error: &Error) -> String {
    format!("ERROR - {error}")
}

/// The failure line a stopped Workflow Run prints after its error (batch `:Failed`, line 376).
///
/// `plugin_base_name` is the batch's `%PluginName_%`: the plugin without its extension.
#[must_use]
pub(crate) fn build_failed_line(plugin_base_name: &str) -> String {
    format!("Build of Patch {plugin_base_name} failed.")
}

/// The last line of every Workflow Run that got as far as executing (batch line 368).
///
/// Console-only, as in the batch: the session log has no use for its own path.
#[must_use]
pub(crate) fn see_log_line(session_log: &Path) -> String {
    format!("See Log at {}", session_log.display())
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
    fn session_header_reproduces_batch_line_266() {
        let header = build_session_header("clean", "MyMod.esp");
        assert!(header.contains("V2.99"));
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
            "Starting filtered Build V2.99 of MyMod.esp\n"
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
            "Starting clean Build V2.99 of MyMod.esp\n\
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
    /// lines 461–463 before `START` rather than after it. Nothing downstream gets to append.
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

    /// Batch 539–541, without `%ModDir_%`: no `-FO4:` was given, so FO4Edit finds `Data` itself.
    #[test]
    fn an_xedit_run_header_names_the_script_plugin_and_plugin_list() {
        let files = InMemoryFileSpace::new();
        let session_log = files.temp_dir().join("MyMod.log");
        let plugins_txt = PathBuf::from(r"C:\Temp\Plugins.txt");

        append_xedit_run_header(
            &session_log,
            "Batch_FO4MergeCombinedObjectsAndCheck.pas",
            "MyMod.esp",
            &plugins_txt,
            None,
            &files,
        )
        .unwrap();

        assert_eq!(
            files.read_lossy(&session_log).unwrap(),
            format!(
                "Running xEdit script Batch_FO4MergeCombinedObjectsAndCheck.pas against MyMod.esp\n\
                 Params -fo4 -autoexit -P:\"{}\"\n\
                 ====================================\n",
                plugins_txt.display()
            )
        );
    }

    /// With `-FO4:`, the batch's `%ModDir_%` follows the plugin list, quoted as it writes it.
    #[test]
    fn an_xedit_run_header_carries_the_data_override() {
        let files = InMemoryFileSpace::new();
        let session_log = files.temp_dir().join("MyMod.log");
        let plugins_txt = PathBuf::from(r"C:\Temp\Plugins.txt");
        let data_dir = PathBuf::from(r"D:\Games\Fallout4\Data");

        append_xedit_run_header(
            &session_log,
            "Batch_FO4MergePrevisandCleanRefr.pas",
            "MyMod.esp",
            &plugins_txt,
            Some(&data_dir),
            &files,
        )
        .unwrap();

        let session = files.read_lossy(&session_log).unwrap();
        assert!(
            session.contains(&format!(
                "Params -fo4 -autoexit -P:\"{}\" -D:\"{}\"\n",
                plugins_txt.display(),
                data_dir.display()
            )),
            "session log: {session}"
        );
    }

    /// The unattended log is folded with the Creation Kit log's newline rules.
    #[test]
    fn an_unattended_log_is_folded_ending_on_a_newline() {
        let files = InMemoryFileSpace::new();
        let session_log = files.temp_dir().join("MyMod.log");

        append_unattended_log(&session_log, "Completed: No Errors.", &files).unwrap();
        append_unattended_log(&session_log, "", &files).unwrap();
        append_unattended_log(&session_log, "second\n", &files).unwrap();

        assert_eq!(
            files.read_lossy(&session_log).unwrap(),
            "Completed: No Errors.\nsecond\n"
        );
    }

    /// Batch 392–393 and 415, without the doubled space the empty `%Arch2Quals_%` left.
    #[test]
    fn the_archive_headers_name_the_archive_and_what_is_packed() {
        let files = InMemoryFileSpace::new();
        let session_log = files.temp_dir().join("MyMod.log");

        append_archive_extract_header(&session_log, "MyMod - Main.ba2", &files).unwrap();
        append_archive_pack_header(
            &session_log,
            "MyMod - Main.ba2",
            "meshes\\precombined,vis",
            &files,
        )
        .unwrap();

        assert_eq!(
            files.read_lossy(&session_log).unwrap(),
            "Extracting Archive MyMod - Main.ba2:\n\
             ====================================\n\
             Creating Archive MyMod - Main.ba2 of meshes\\precombined,vis:\n\
             ====================================\n"
        );
    }

    /// Stdout then stderr, each ending on a newline, and an empty stream adds nothing.
    #[test]
    fn archive_tool_output_folds_stdout_then_stderr() {
        let files = InMemoryFileSpace::new();
        let session_log = files.temp_dir().join("MyMod.log");

        append_archive_tool_output(&session_log, "Packed 2 files", "stack trace\n", &files)
            .unwrap();
        append_archive_tool_output(&session_log, "", "", &files).unwrap();

        assert_eq!(
            files.read_lossy(&session_log).unwrap(),
            "Packed 2 files\nstack trace\n"
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
