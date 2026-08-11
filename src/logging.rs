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
// No production caller yet: the batch writes a banner line per step, and the steps that do
// so are not ported. It stays live through its test until those operations land.
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "the per-step banner lines arrive with the Workflow Operations that emit them"
    )
)]
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

/// Append already-read Creation Kit log contents to the session log (batch `:RunCK` lines 460–461).
///
/// Takes `contents` rather than a path so the Creation Kit adapter reads its log exactly once
/// and uses that one read for both this append and the content it hands back to the Workflow
/// Operation. "No log at all" is therefore the caller's state to recognise, not this
/// function's. Returns [`crate::error::Error::Io`] when the session log cannot be extended.
pub fn append_ck_log(session_log: &Path, contents: &str, files: &dyn FileSpace) -> Result<()> {
    // Assembled in full before the single append: the framing — leading blank line, banner,
    // and a normalised trailing newline so the next banner starts on its own line — is one
    // block of the log, and building it here keeps that true whatever `FileSpace` backs it.
    let mut block = format!("\n----- Creation Kit log -----\n{contents}");
    if !contents.ends_with('\n') {
        block.push('\n');
    }

    files.append(session_log, &block)
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

    #[test]
    fn append_ck_log_frames_the_creation_kit_contents() {
        let files = InMemoryFileSpace::new();
        let session_log = files.temp_dir().join("MyMod.log");
        init_session_log(&session_log, "clean", "MyMod.esp", &files).unwrap();

        append_ck_log(&session_log, "Masterfile: Fallout4.esm\n", &files).unwrap();

        assert_eq!(
            files.read_lossy(&session_log).unwrap(),
            "Starting clean Build V2.95 of MyMod.esp\n\n----- Creation Kit log -----\nMasterfile: Fallout4.esm\n"
        );
    }

    /// A Creation Kit log with no final newline still ends the block on one, so a later
    /// append starts on its own line.
    #[test]
    fn append_ck_log_normalises_a_missing_trailing_newline() {
        let files = InMemoryFileSpace::new();
        let session_log = files.temp_dir().join("MyMod.log");

        append_ck_log(&session_log, "truncated", &files).unwrap();

        assert_eq!(
            files.read_lossy(&session_log).unwrap(),
            "\n----- Creation Kit log -----\ntruncated\n"
        );
    }

    /// Pinned on [`SystemFileSpace`]: the framing has to survive a real file, and this is the
    /// one place the session log is written through the standard-library adapter.
    ///
    /// Tolerance for the non-UTF-8 bytes Creation Kit emits is no longer asserted here — this
    /// function no longer reads the log. That property now belongs to
    /// `SystemFileSpace::read_lossy`, where `files::tests` pins it.
    #[test]
    fn append_ck_log_frames_through_the_system_space_too() {
        let dir = tempfile::tempdir().unwrap();
        let files = SystemFileSpace;
        let session_log = dir.path().join("session.log");

        append_ck_log(&session_log, "ok\u{fffd}\n", &files).unwrap();

        let session = std::fs::read_to_string(session_log).unwrap();
        assert!(session.contains("Creation Kit log"));
        assert!(session.contains("ok"));
    }
}
