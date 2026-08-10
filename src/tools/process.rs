//! The `ProcessRunner` seam: spawning an external tool and waiting for it to exit.
//!
//! An internal seam, private to the tools layer. It exists so the *ordering* an external-tool
//! episode is obliged to keep — DLL guard, log delete, spawn, MO2 delay, log append — becomes
//! assertable without launching Creation Kit. It is deliberately absent from what a Workflow
//! Operation is handed: a Workflow Operation states build meaning, not how a process starts.
//!
//! The interface is deliberately minimal. `FO4Edit` will need an async spawn plus a process
//! handle (`AppActivate` / `CloseMainWindow` / `TaskKill` / poll); that widening happens
//! against a real caller rather than being guessed at now.

use std::ffi::OsString;
use std::path::Path;
use std::process::{Command, ExitStatus};

use crate::error::Result;

/// Spawn an external tool and wait for it.
///
/// Implementors must be `Debug` so the adapters that hold a `ProcessRunner` can keep
/// deriving `Debug`.
pub(crate) trait ProcessRunner: std::fmt::Debug {
    /// Spawn `exe` with `args`, working directory `cwd`, and wait for it to exit.
    ///
    /// Equivalent to the batch `START "..." /D"<cwd>" /wait <exe> <args>`.
    /// Returns the exit status; a non-zero status is **not** an error here — exit-code
    /// policy belongs to the caller.
    ///
    /// `cwd` is not optional: every Creation Kit invocation sets `/D"<fo4dir>"` and every
    /// Archive2 call runs with the working directory set to `Data`.
    fn run(&self, exe: &Path, args: &[OsString], cwd: &Path) -> Result<ExitStatus>;
}

/// The production `ProcessRunner`, backed by [`std::process::Command`].
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct SystemProcessRunner;

impl ProcessRunner for SystemProcessRunner {
    fn run(&self, exe: &Path, args: &[OsString], cwd: &Path) -> Result<ExitStatus> {
        Ok(Command::new(exe).current_dir(cwd).args(args).status()?)
    }
}

#[cfg(test)]
pub(crate) use recording::{RecordedProcessCall, RecordingProcessRunner};

#[cfg(test)]
mod recording {
    use std::cell::RefCell;
    use std::ffi::OsString;
    use std::fmt;
    use std::path::{Path, PathBuf};
    use std::process::ExitStatus;

    use super::ProcessRunner;
    use crate::error::Result;
    use crate::files::FileSpace;

    /// One external-tool spawn, as the caller under test issued it.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub(crate) struct RecordedProcessCall {
        pub(crate) exe: PathBuf,
        pub(crate) args: Vec<OsString>,
        pub(crate) cwd: PathBuf,
    }

    /// A test [`ProcessRunner`] that records every spawn and launches nothing.
    ///
    /// Interior mutability, like the other recording adapters: the caller under test holds
    /// the runner by shared reference across the whole episode.
    pub(crate) struct RecordingProcessRunner<'a> {
        calls: RefCell<Vec<RecordedProcessCall>>,
        status: ExitStatus,
        effects: Option<Box<dyn Fn() + 'a>>,
    }

    impl Default for RecordingProcessRunner<'_> {
        fn default() -> Self {
            Self {
                calls: RefCell::new(Vec::new()),
                status: exit_status_with_code(0),
                effects: None,
            }
        }
    }

    impl<'a> RecordingProcessRunner<'a> {
        /// A runner whose simulated tool succeeds and leaves nothing behind.
        #[must_use]
        pub(crate) fn new() -> Self {
            Self::default()
        }

        /// Report `code` as the simulated tool's exit status.
        ///
        /// A non-zero code is not an error at this seam — it is the input to the caller's
        /// exit-code policy, which is the thing under test.
        #[must_use]
        pub(crate) fn returning_exit_code(mut self, code: i32) -> Self {
            self.status = exit_status_with_code(code);
            self
        }

        /// Simulate the tool's filesystem effects into the space the caller observes.
        ///
        /// The external tools this port stands in for return nothing useful — they *write
        /// files*, and the caller reads them back afterwards. Binding the callback to the
        /// same [`FileSpace`] the caller was given is what makes "the tool wrote a log, and
        /// the caller read *that* log" assertable. The space is taken as a concrete type so
        /// the callback can reach seeding helpers that are not on the trait — `FileSpace` has
        /// no write operation yet, so revisit this signature once it gains one.
        #[must_use]
        pub(crate) fn with_effects<S: FileSpace>(
            mut self,
            space: &'a S,
            apply: impl Fn(&S) + 'a,
        ) -> Self {
            self.effects = Some(Box::new(move || apply(space)));
            self
        }

        /// The spawns recorded so far, in call order.
        #[must_use]
        pub(crate) fn calls(&self) -> Vec<RecordedProcessCall> {
            self.calls.borrow().clone()
        }
    }

    impl ProcessRunner for RecordingProcessRunner<'_> {
        fn run(&self, exe: &Path, args: &[OsString], cwd: &Path) -> Result<ExitStatus> {
            self.calls.borrow_mut().push(RecordedProcessCall {
                exe: exe.to_path_buf(),
                args: args.to_vec(),
                cwd: cwd.to_path_buf(),
            });

            // Recorded before the effects run, so a callback that inspects the space sees a
            // call history consistent with "the tool has started".
            if let Some(effects) = &self.effects {
                effects();
            }

            Ok(self.status)
        }
    }

    impl fmt::Debug for RecordingProcessRunner<'_> {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            // The effects callback is not `Debug`; whether one is installed is the part a
            // failing assertion actually needs to see.
            f.debug_struct("RecordingProcessRunner")
                .field("calls", &self.calls)
                .field("status", &self.status)
                .field("effects", &self.effects.is_some())
                .finish()
        }
    }

    /// Build an [`ExitStatus`] carrying `code`.
    ///
    /// The standard library offers no portable constructor, and the two platform extensions
    /// disagree on the encoding: Windows stores the process exit code directly, while Unix
    /// stores a `wait` status whose low byte is the terminating signal, putting the code in
    /// the high byte.
    fn exit_status_with_code(code: i32) -> ExitStatus {
        #[cfg(windows)]
        {
            use std::os::windows::process::ExitStatusExt;
            ExitStatus::from_raw(code.cast_unsigned())
        }
        #[cfg(not(windows))]
        {
            use std::os::unix::process::ExitStatusExt;
            ExitStatus::from_raw(code << 8)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::fs;
    use std::path::{Path, PathBuf};

    use tempfile::tempdir;

    use super::{ProcessRunner, RecordedProcessCall, RecordingProcessRunner, SystemProcessRunner};
    use crate::files::{FileSpace, InMemoryFileSpace};

    #[test]
    fn recording_runner_records_every_spawn_in_order() {
        let runner = RecordingProcessRunner::new();
        let exe = PathBuf::from(r"C:\Games\Fallout4\CreationKit.exe");
        let cwd = PathBuf::from(r"C:\Games\Fallout4");

        runner
            .run(
                &exe,
                &[
                    OsString::from("-GeneratePrecombined:\"My Mod.esp\""),
                    OsString::from("clean"),
                    OsString::from("all"),
                ],
                &cwd,
            )
            .unwrap();
        runner
            .run(&exe, &[OsString::from("-BuildCDX:\"My Mod.esp\"")], &cwd)
            .unwrap();

        assert_eq!(
            runner.calls(),
            vec![
                RecordedProcessCall {
                    exe: exe.clone(),
                    args: vec![
                        OsString::from("-GeneratePrecombined:\"My Mod.esp\""),
                        OsString::from("clean"),
                        OsString::from("all"),
                    ],
                    cwd: cwd.clone(),
                },
                RecordedProcessCall {
                    exe,
                    args: vec![OsString::from("-BuildCDX:\"My Mod.esp\"")],
                    cwd,
                },
            ]
        );
    }

    #[test]
    fn recording_runner_reports_the_configured_exit_status() {
        let quiet = RecordingProcessRunner::new();
        let noisy = RecordingProcessRunner::new().returning_exit_code(2);
        let exe = Path::new("CreationKit.exe");
        let cwd = Path::new("Fallout4");

        assert!(quiet.run(exe, &[], cwd).unwrap().success());

        // A non-zero exit is reported, not raised: exit-code policy belongs to the caller.
        let status = noisy.run(exe, &[], cwd).unwrap();
        assert!(!status.success());
        assert_eq!(status.code(), Some(2));
    }

    #[test]
    fn recording_runner_applies_simulated_effects_to_the_shared_space() {
        let space = InMemoryFileSpace::new();
        let log = PathBuf::from(r"C:\Games\Fallout4\Logs\CK.log");
        let runner = {
            let log = log.clone();
            RecordingProcessRunner::new().with_effects(&space, move |space| {
                space.add_file_with_contents(&log, "Masterfile: Fallout4.esm\n");
            })
        };

        assert!(!space.is_file(&log));

        runner
            .run(
                Path::new("CreationKit.exe"),
                &[OsString::from("-BuildCDX:\"My Mod.esp\"")],
                Path::new(r"C:\Games\Fallout4"),
            )
            .unwrap();

        // The simulated tool wrote its log into the very space the caller reads back.
        assert_eq!(
            space.read_lossy(&log).unwrap(),
            "Masterfile: Fallout4.esm\n"
        );
    }

    /// The system adapter, against a real child process.
    ///
    /// Asserts all three inputs at once: the executable is found, `args` reach it as distinct
    /// arguments, and `cwd` is the directory the child sees — the marker file is only visible
    /// from the temporary directory, so a wrong working directory changes the exit code.
    #[test]
    fn system_runner_spawns_with_the_given_args_and_working_directory() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("marker.txt"), b"marker").unwrap();
        let runner = SystemProcessRunner;

        let status = runner
            .run(Path::new(SHELL), &marker_probe_args(), dir.path())
            .unwrap();

        assert_eq!(status.code(), Some(7));
    }

    #[test]
    fn system_runner_surfaces_a_missing_executable_as_an_error() {
        let dir = tempdir().unwrap();
        let runner = SystemProcessRunner;

        assert!(
            runner
                .run(&dir.path().join("no-such-tool.exe"), &[], dir.path())
                .is_err()
        );
    }

    #[cfg(windows)]
    const SHELL: &str = "cmd.exe";
    #[cfg(not(windows))]
    const SHELL: &str = "/bin/sh";

    /// Arguments for a shell that exits 7 when `marker.txt` sits in its working directory.
    ///
    /// Each Windows argument is a bare token on purpose: `Command` only adds quotes around
    /// arguments containing spaces, and `cmd.exe /C` does not strip quotes reliably once the
    /// quoted text holds anything it considers special.
    #[cfg(windows)]
    fn marker_probe_args() -> Vec<OsString> {
        ["/C", "if", "exist", "marker.txt", "exit", "7"]
            .into_iter()
            .map(OsString::from)
            .collect()
    }

    #[cfg(not(windows))]
    fn marker_probe_args() -> Vec<OsString> {
        vec![
            OsString::from("-c"),
            OsString::from("test -f marker.txt && exit 7"),
        ]
    }
}
