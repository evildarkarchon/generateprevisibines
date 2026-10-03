//! The `ProcessRunner` seam: starting an external tool, then either waiting for it to exit
//! (optionally capturing what it printed) or handing back the live process.
//!
//! An internal seam, private to the tools layer. It exists so the *ordering* an external-tool
//! episode is obliged to keep — DLL guard, log delete, spawn, MO2 delay, log append — becomes
//! assertable without launching Creation Kit. It is deliberately absent from what a Workflow
//! Operation is handed: a Workflow Operation states build meaning, not how a process starts.
//!
//! There are three ways to start a tool, matching how the batch launches each one. [`run`] is
//! `START /wait`, used by every Creation Kit call; the batch does not redirect Creation Kit's
//! output, so neither does `run`. [`run_capturing`] is `START /wait ... >> "%Logfile_%"`, used
//! by the archive tools (Archive2 and BSArch): it waits the same way and also returns the
//! tool's stdout and stderr as a [`ProcessOutput`], so the Archive episode can fold both into
//! the session log. [`spawn`] is `START /B`, used by the FO4Edit episode: it starts FO4Edit,
//! polls for the log FO4Edit's script writes, and then asks FO4Edit to close, so it needs the
//! running process's id and a non-blocking exit check. The window work (dismissing Module
//! Selection, the close request) is not this seam's job; it targets windows by the process id
//! a [`RunningProcess`] reports.
//!
//! [`run`]: ProcessRunner::run
//! [`run_capturing`]: ProcessRunner::run_capturing
//! [`spawn`]: ProcessRunner::spawn

use std::ffi::OsString;
use std::path::Path;
use std::process::{Child, Command, ExitStatus};

use crate::error::Result;

/// Start an external tool, and either wait for it or hand back the running process.
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

    /// Spawn `exe` with `args`, working directory `cwd`, wait for it to exit, and capture
    /// its stdout and stderr.
    ///
    /// Equivalent to the batch `START ... /wait <exe> <args> >> "%Logfile_%"`, which runs
    /// each archive tool. `args` and `cwd` are handled exactly as in [`run`](Self::run), and
    /// as there, a non-zero exit is reported in [`ProcessOutput::status`], not raised. Fails
    /// only when the process cannot be started or its output cannot be read. The child gets
    /// a null stdin, so a tool that prompts for input reads end-of-file instead of hanging
    /// the run.
    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "the Archive episode (ArchiveOps) that runs Archive2 and BSArch is not ported yet"
        )
    )]
    fn run_capturing(&self, exe: &Path, args: &[OsString], cwd: &Path) -> Result<ProcessOutput>;

    /// Spawn `exe` with `args`, working directory `cwd`, and return without waiting.
    ///
    /// Equivalent to the batch `START /B`, which launches FO4Edit. `args` and `cwd` are
    /// handled exactly as in [`run`](Self::run). Fails only when the process cannot be
    /// started; how the process later exits is read through [`RunningProcess::try_wait`].
    fn spawn(&self, exe: &Path, args: &[OsString], cwd: &Path) -> Result<Box<dyn RunningProcess>>;
}

/// A process started by [`ProcessRunner::spawn`] that may still be running.
///
/// There is deliberately no `kill`. xEdit saves its merge only on its close path, so the
/// FO4Edit episode asks it to close and never kills it, and nothing else needs to. Dropping a
/// `RunningProcess` leaves the process running; the handle is only a way to observe it.
pub(crate) trait RunningProcess: std::fmt::Debug {
    /// The operating system's process id, which window lookups target.
    fn id(&self) -> u32;

    /// Report the exit status if the process has exited, without blocking.
    ///
    /// `None` while the process is still running. Once it has exited, every later call keeps
    /// reporting the same status. As with [`ProcessRunner::run`], a non-zero status is not an
    /// error here.
    fn try_wait(&mut self) -> Result<Option<ExitStatus>>;
}

/// What a tool started by [`ProcessRunner::run_capturing`] left behind once it exited.
///
/// Both streams are decoded lossily: neither Archive2 nor BSArch promises UTF-8, and a
/// mangled character in a log line is better than losing the line.
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "the Archive episode (ArchiveOps) that reads Archive2's and BSArch's output is not ported yet"
    )
)]
#[derive(Debug, Clone)]
pub(crate) struct ProcessOutput {
    /// How the process exited. A non-zero status is not an error at this seam.
    pub(crate) status: ExitStatus,
    /// Everything the process wrote to stdout.
    pub(crate) stdout: String,
    /// Everything the process wrote to stderr, kept apart from stdout.
    pub(crate) stderr: String,
}

/// The production `ProcessRunner`, backed by [`std::process::Command`].
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct SystemProcessRunner;

impl ProcessRunner for SystemProcessRunner {
    fn run(&self, exe: &Path, args: &[OsString], cwd: &Path) -> Result<ExitStatus> {
        Ok(Command::new(exe).current_dir(cwd).args(args).status()?)
    }

    fn run_capturing(&self, exe: &Path, args: &[OsString], cwd: &Path) -> Result<ProcessOutput> {
        // `output()` pipes stdout and stderr and gives the child a null stdin.
        let output = Command::new(exe).current_dir(cwd).args(args).output()?;
        Ok(ProcessOutput {
            status: output.status,
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }

    fn spawn(&self, exe: &Path, args: &[OsString], cwd: &Path) -> Result<Box<dyn RunningProcess>> {
        let child = Command::new(exe).current_dir(cwd).args(args).spawn()?;
        Ok(Box::new(SystemRunningProcess { child }))
    }
}

/// The production [`RunningProcess`], wrapping a [`std::process::Child`].
///
/// Relies on `Child`'s drop behaviour: dropping a `Child` neither kills nor waits for the
/// process, which is exactly the "dropping leaves it running" contract.
#[derive(Debug)]
struct SystemRunningProcess {
    child: Child,
}

impl RunningProcess for SystemRunningProcess {
    fn id(&self) -> u32 {
        self.child.id()
    }

    fn try_wait(&mut self) -> Result<Option<ExitStatus>> {
        Ok(self.child.try_wait()?)
    }
}

#[cfg(test)]
pub(crate) use recording::{
    ExitFlag, ProcessCallKind, RECORDED_PROCESS_ID, RecordedProcessCall, RecordingProcessRunner,
    ScriptedCall, ScriptedExit,
};

#[cfg(test)]
mod recording {
    use std::cell::{Cell, RefCell};
    use std::collections::BTreeMap;
    use std::ffi::OsString;
    use std::fmt;
    use std::path::{Path, PathBuf};
    use std::process::ExitStatus;
    use std::rc::Rc;

    use super::{ProcessOutput, ProcessRunner, RunningProcess};
    use crate::error::Result;
    use crate::files::FileSpace;

    /// The process id every recorded spawn reports.
    ///
    /// Fixed, so a test can assert that a window lookup or close request targeted *this* run's
    /// process. Not a value any real process is likely to hold, so it reads as fake in output.
    pub(crate) const RECORDED_PROCESS_ID: u32 = 4242;

    /// One external-tool start, as the caller under test issued it.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub(crate) struct RecordedProcessCall {
        pub(crate) exe: PathBuf,
        pub(crate) args: Vec<OsString>,
        pub(crate) cwd: PathBuf,
        pub(crate) kind: ProcessCallKind,
    }

    /// Which [`ProcessRunner`] method started a [`RecordedProcessCall`].
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(crate) enum ProcessCallKind {
        /// [`ProcessRunner::run`]: started and waited for (`START /wait`).
        Run,
        /// [`ProcessRunner::run_capturing`]: started and waited for, output captured
        /// (`START /wait ... >> "%Logfile_%"`).
        RunCapturing,
        /// [`ProcessRunner::spawn`]: started and left running (`START /B`).
        Spawn,
    }

    /// A simulated file effect for one scripted call, given that call's `exe` and `args`.
    type CallEffect<'a> = Box<dyn Fn(&Path, &[OsString]) + 'a>;

    /// What one particular call to a [`RecordingProcessRunner`] reports and does.
    ///
    /// Installed with [`RecordingProcessRunner::scripting_call`]. Each part is an override:
    /// an exit code left unset falls back to the runner-wide
    /// [`returning_exit_code`](RecordingProcessRunner::returning_exit_code), output left unset
    /// is empty, and an effect left unset falls back to the runner-wide
    /// [`with_effects`](RecordingProcessRunner::with_effects). The exit code applies to
    /// [`run`](ProcessRunner::run) and [`run_capturing`](ProcessRunner::run_capturing); the
    /// output only to `run_capturing`, the one method that returns it. A spawn uses only the
    /// effect: how a spawned process exits is [`ScriptedExit`]'s job.
    #[derive(Default)]
    pub(crate) struct ScriptedCall<'a> {
        code: Option<i32>,
        stdout: String,
        stderr: String,
        effect: Option<CallEffect<'a>>,
    }

    impl<'a> ScriptedCall<'a> {
        /// A call that behaves like an unscripted one until a builder method says otherwise.
        #[must_use]
        pub(crate) fn new() -> Self {
            Self::default()
        }

        /// Report `code` as this call's exit status.
        #[must_use]
        pub(crate) fn exiting_with(mut self, code: i32) -> Self {
            self.code = Some(code);
            self
        }

        /// Report `stdout` as what this call wrote to stdout.
        #[must_use]
        pub(crate) fn with_stdout(mut self, stdout: impl Into<String>) -> Self {
            self.stdout = stdout.into();
            self
        }

        /// Report `stderr` as what this call wrote to stderr.
        #[must_use]
        pub(crate) fn with_stderr(mut self, stderr: impl Into<String>) -> Self {
            self.stderr = stderr.into();
            self
        }

        /// Simulate this call's filesystem effects into the space the caller observes.
        ///
        /// Like [`RecordingProcessRunner::with_effects`], but for this call alone, and the
        /// callback also receives the call's `exe` and `args`. That is what lets one
        /// rebuild's extract and pack each leave their own files behind, written where
        /// *their* arguments said to write them.
        #[must_use]
        pub(crate) fn with_effect<S: FileSpace>(
            mut self,
            space: &'a S,
            apply: impl Fn(&S, &Path, &[OsString]) + 'a,
        ) -> Self {
            self.effect = Some(Box::new(move |exe, args| apply(space, exe, args)));
            self
        }
    }

    impl fmt::Debug for ScriptedCall<'_> {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            // As for the runner: the callback is not `Debug`, so show only whether one is set.
            f.debug_struct("ScriptedCall")
                .field("code", &self.code)
                .field("stdout", &self.stdout)
                .field("stderr", &self.stderr)
                .field("effect", &self.effect.is_some())
                .finish()
        }
    }

    /// How a process returned by [`RecordingProcessRunner::spawn`] exits.
    ///
    /// Each spawn gets its own copy, so poll counts are per process. A [`ExitFlag`] clones
    /// by sharing, so every process spawned with the same flag exits together.
    #[derive(Debug, Clone)]
    pub(crate) enum ScriptedExit {
        /// The process never exits: `try_wait` reports `None` forever.
        Never,
        /// The first `polls` calls to `try_wait` report `None`; every later call reports `code`.
        /// `polls: 0` is a process that has already exited by its first poll.
        AfterPolls { polls: usize, code: i32 },
        /// `try_wait` reports `None` until `flag` is set, and `code` from then on.
        WhenFlagged { flag: ExitFlag, code: i32 },
    }

    /// A shared switch that makes [`ScriptedExit::WhenFlagged`] processes exit.
    ///
    /// Clones share one switch. It lets something other than the test body decide when the
    /// process exits — the FO4Edit episode tests make "FO4Edit exits when asked to close" an
    /// effect of the recording window double, which holds a clone and sets it.
    #[derive(Debug, Clone, Default)]
    pub(crate) struct ExitFlag(Rc<Cell<bool>>);

    impl ExitFlag {
        /// A switch that is not yet set.
        #[must_use]
        pub(crate) fn new() -> Self {
            Self::default()
        }

        /// Set the switch. Every process scripted on this flag (or a clone) exits from its
        /// next poll on. Setting it again changes nothing.
        pub(crate) fn set(&self) {
            self.0.set(true);
        }

        /// Whether the switch has been set.
        #[must_use]
        pub(crate) fn is_set(&self) -> bool {
            self.0.get()
        }
    }

    /// A test [`ProcessRunner`] that records every run, capturing run and spawn, and
    /// launches nothing.
    ///
    /// Interior mutability, like the other recording adapters: the caller under test holds
    /// the runner by shared reference across the whole episode.
    pub(crate) struct RecordingProcessRunner<'a> {
        calls: RefCell<Vec<RecordedProcessCall>>,
        status: ExitStatus,
        spawned_exit: ScriptedExit,
        effects: Option<Box<dyn Fn() + 'a>>,
        /// Per-call overrides, keyed by the call's index in [`calls`](Self::calls).
        scripted: BTreeMap<usize, ScriptedCall<'a>>,
    }

    impl Default for RecordingProcessRunner<'_> {
        fn default() -> Self {
            Self {
                calls: RefCell::new(Vec::new()),
                status: exit_status_with_code(0),
                spawned_exit: ScriptedExit::Never,
                effects: None,
                scripted: BTreeMap::new(),
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

        /// Script how every process this runner spawns exits.
        ///
        /// Defaults to [`ScriptedExit::Never`]: FO4Edit stays open until something asks it to
        /// close, so a test has to say when it goes away. Has no bearing on [`run`], whose
        /// status comes from [`returning_exit_code`](Self::returning_exit_code).
        ///
        /// [`run`]: ProcessRunner::run
        #[must_use]
        pub(crate) fn spawning(mut self, exit: ScriptedExit) -> Self {
            self.spawned_exit = exit;
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

        /// Script what the call at `index` reports and does.
        ///
        /// `index` is 0-based and counts every call of every kind, so it is the position the
        /// call will hold in [`calls`](Self::calls). Calls without a script behave as they
        /// would without this method. Scripting the same index twice keeps the later script.
        #[must_use]
        pub(crate) fn scripting_call(mut self, index: usize, call: ScriptedCall<'a>) -> Self {
            self.scripted.insert(index, call);
            self
        }

        /// The runs, capturing runs and spawns recorded so far, in one list, in call order.
        #[must_use]
        pub(crate) fn calls(&self) -> Vec<RecordedProcessCall> {
            self.calls.borrow().clone()
        }

        /// Record a start of any kind, then apply the simulated effects.
        ///
        /// Returns this call's script, if one was installed for its index.
        fn start(
            &self,
            exe: &Path,
            args: &[OsString],
            cwd: &Path,
            kind: ProcessCallKind,
        ) -> Option<&ScriptedCall<'a>> {
            let index = {
                let mut calls = self.calls.borrow_mut();
                calls.push(RecordedProcessCall {
                    exe: exe.to_path_buf(),
                    args: args.to_vec(),
                    cwd: cwd.to_path_buf(),
                    kind,
                });
                calls.len() - 1
            };
            let script = self.scripted.get(&index);

            // Recorded before the effects run, so a callback that inspects the space sees a
            // call history consistent with "the tool has started". The `calls` borrow is
            // released first, so an effect may read `calls()` itself.
            if let Some(effect) = script.and_then(|script| script.effect.as_ref()) {
                effect(exe, args);
            } else if let Some(effects) = &self.effects {
                effects();
            }
            script
        }

        /// The exit status a waited-for call reports: its script's code, else the runner's.
        fn status_for(&self, script: Option<&ScriptedCall<'_>>) -> ExitStatus {
            script
                .and_then(|script| script.code)
                .map_or(self.status, exit_status_with_code)
        }
    }

    impl ProcessRunner for RecordingProcessRunner<'_> {
        fn run(&self, exe: &Path, args: &[OsString], cwd: &Path) -> Result<ExitStatus> {
            let script = self.start(exe, args, cwd, ProcessCallKind::Run);
            Ok(self.status_for(script))
        }

        fn run_capturing(
            &self,
            exe: &Path,
            args: &[OsString],
            cwd: &Path,
        ) -> Result<ProcessOutput> {
            let script = self.start(exe, args, cwd, ProcessCallKind::RunCapturing);
            Ok(ProcessOutput {
                status: self.status_for(script),
                stdout: script
                    .map(|script| script.stdout.clone())
                    .unwrap_or_default(),
                stderr: script
                    .map(|script| script.stderr.clone())
                    .unwrap_or_default(),
            })
        }

        fn spawn(
            &self,
            exe: &Path,
            args: &[OsString],
            cwd: &Path,
        ) -> Result<Box<dyn RunningProcess>> {
            // Effects apply at spawn time, as for `run`. A test that needs output to appear
            // later (on a dismissal, on the Nth poll) installs it as an effect of that later
            // seam instead.
            self.start(exe, args, cwd, ProcessCallKind::Spawn);
            Ok(Box::new(RecordingRunningProcess {
                exit: self.spawned_exit.clone(),
                polls: 0,
            }))
        }
    }

    /// The process a [`RecordingProcessRunner`] spawn returns: nothing runs, it only follows
    /// its [`ScriptedExit`].
    #[derive(Debug)]
    struct RecordingRunningProcess {
        exit: ScriptedExit,
        /// `try_wait` calls so far, counted for [`ScriptedExit::AfterPolls`].
        polls: usize,
    }

    impl RunningProcess for RecordingRunningProcess {
        fn id(&self) -> u32 {
            RECORDED_PROCESS_ID
        }

        fn try_wait(&mut self) -> Result<Option<ExitStatus>> {
            let exit_code = match &self.exit {
                ScriptedExit::Never => None,
                ScriptedExit::AfterPolls {
                    polls: exit_after,
                    code,
                } => (self.polls >= *exit_after).then_some(*code),
                ScriptedExit::WhenFlagged { flag, code } => flag.is_set().then_some(*code),
            };
            // Counted after the check, so `AfterPolls { polls: n }` reports `None` on exactly
            // the first n polls.
            self.polls += 1;
            Ok(exit_code.map(exit_status_with_code))
        }
    }

    impl fmt::Debug for RecordingProcessRunner<'_> {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            // The effects callback is not `Debug`; whether one is installed is the part a
            // failing assertion actually needs to see.
            f.debug_struct("RecordingProcessRunner")
                .field("calls", &self.calls)
                .field("status", &self.status)
                .field("spawned_exit", &self.spawned_exit)
                .field("effects", &self.effects.is_some())
                .field("scripted", &self.scripted)
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

    use super::{
        ExitFlag, ProcessCallKind, ProcessRunner, RECORDED_PROCESS_ID, RecordedProcessCall,
        RecordingProcessRunner, RunningProcess, ScriptedCall, ScriptedExit, SystemProcessRunner,
    };
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
                    OsString::from("-GeneratePrecombined:My Mod.esp"),
                    OsString::from("clean"),
                    OsString::from("all"),
                ],
                &cwd,
            )
            .unwrap();
        runner
            .run(&exe, &[OsString::from("-BuildCDX:My Mod.esp")], &cwd)
            .unwrap();

        assert_eq!(
            runner.calls(),
            vec![
                RecordedProcessCall {
                    exe: exe.clone(),
                    args: vec![
                        OsString::from("-GeneratePrecombined:My Mod.esp"),
                        OsString::from("clean"),
                        OsString::from("all"),
                    ],
                    cwd: cwd.clone(),
                    kind: ProcessCallKind::Run,
                },
                RecordedProcessCall {
                    exe,
                    args: vec![OsString::from("-BuildCDX:My Mod.esp")],
                    cwd,
                    kind: ProcessCallKind::Run,
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
                &[OsString::from("-BuildCDX:My Mod.esp")],
                Path::new(r"C:\Games\Fallout4"),
            )
            .unwrap();

        // The simulated tool wrote its log into the very space the caller reads back.
        assert_eq!(
            space.read_lossy(&log).unwrap(),
            "Masterfile: Fallout4.esm\n"
        );
    }

    #[test]
    fn recording_runner_records_spawns_and_runs_in_one_ordered_list() {
        let runner = RecordingProcessRunner::new();
        let ck = PathBuf::from(r"C:\Games\Fallout4\CreationKit.exe");
        let fo4edit = PathBuf::from(r"C:\Tools\FO4Edit\FO4Edit.exe");
        let fo4dir = PathBuf::from(r"C:\Games\Fallout4");
        let temp = PathBuf::from(r"C:\Users\me\AppData\Local\Temp");

        runner
            .run(&ck, &[OsString::from("-BuildCDX:My Mod.esp")], &fo4dir)
            .unwrap();
        runner
            .spawn(&fo4edit, &[OsString::from("-fo4")], &temp)
            .unwrap();
        runner
            .run(
                &ck,
                &[OsString::from("-GeneratePreVisData:My Mod.esp")],
                &fo4dir,
            )
            .unwrap();

        assert_eq!(
            runner.calls(),
            vec![
                RecordedProcessCall {
                    exe: ck.clone(),
                    args: vec![OsString::from("-BuildCDX:My Mod.esp")],
                    cwd: fo4dir.clone(),
                    kind: ProcessCallKind::Run,
                },
                RecordedProcessCall {
                    exe: fo4edit,
                    args: vec![OsString::from("-fo4")],
                    cwd: temp,
                    kind: ProcessCallKind::Spawn,
                },
                RecordedProcessCall {
                    exe: ck,
                    args: vec![OsString::from("-GeneratePreVisData:My Mod.esp")],
                    cwd: fo4dir,
                    kind: ProcessCallKind::Run,
                },
            ]
        );
    }

    #[test]
    fn recording_runner_applies_simulated_effects_at_spawn_time() {
        let space = InMemoryFileSpace::new();
        let log = PathBuf::from(r"C:\Temp\UnattendedScript.log");
        let runner = {
            let log = log.clone();
            RecordingProcessRunner::new().with_effects(&space, move |space| {
                space.add_file_with_contents(&log, "Completed: No Errors.\n");
            })
        };

        let _process = runner
            .spawn(Path::new("FO4Edit.exe"), &[], Path::new(r"C:\Temp"))
            .unwrap();

        // Applied by the spawn itself, before the caller has polled the process even once.
        assert!(space.is_file(&log));
    }

    #[test]
    fn a_recorded_process_reports_the_fixed_fake_pid() {
        let runner = RecordingProcessRunner::new();

        let process = spawn_fo4edit(&runner);

        assert_eq!(process.id(), RECORDED_PROCESS_ID);
    }

    #[test]
    fn a_recorded_process_scripted_never_to_exit_keeps_running() {
        let runner = RecordingProcessRunner::new().spawning(ScriptedExit::Never);
        let mut process = spawn_fo4edit(&runner);

        for _ in 0..100 {
            assert!(process.try_wait().unwrap().is_none());
        }
    }

    #[test]
    fn a_recorded_process_exits_after_the_scripted_number_of_polls() {
        let runner =
            RecordingProcessRunner::new().spawning(ScriptedExit::AfterPolls { polls: 2, code: 3 });
        let mut process = spawn_fo4edit(&runner);

        assert!(process.try_wait().unwrap().is_none());
        assert!(process.try_wait().unwrap().is_none());
        assert_eq!(process.try_wait().unwrap().unwrap().code(), Some(3));
        // Like `Child::try_wait`, an exited process keeps reporting its exit status.
        assert_eq!(process.try_wait().unwrap().unwrap().code(), Some(3));
    }

    #[test]
    fn each_recorded_spawn_counts_its_own_polls() {
        let runner =
            RecordingProcessRunner::new().spawning(ScriptedExit::AfterPolls { polls: 1, code: 0 });
        let mut first = spawn_fo4edit(&runner);
        assert!(first.try_wait().unwrap().is_none());
        assert!(first.try_wait().unwrap().is_some());

        let mut second = spawn_fo4edit(&runner);

        assert!(second.try_wait().unwrap().is_none());
    }

    #[test]
    fn a_recorded_process_exits_once_its_external_flag_is_set() {
        let flag = ExitFlag::new();
        let runner = RecordingProcessRunner::new().spawning(ScriptedExit::WhenFlagged {
            flag: flag.clone(),
            code: 0,
        });
        let mut process = spawn_fo4edit(&runner);

        assert!(process.try_wait().unwrap().is_none());
        assert!(process.try_wait().unwrap().is_none());

        // Whoever holds a clone — in the FO4Edit episode tests, the recording window double
        // asked to close FO4Edit — makes the process exit.
        flag.set();

        assert!(process.try_wait().unwrap().unwrap().success());
    }

    #[test]
    fn recording_runner_reports_each_scripted_call_and_defaults_the_rest() {
        let runner = RecordingProcessRunner::new()
            .scripting_call(
                0,
                ScriptedCall::new()
                    .exiting_with(0)
                    .with_stdout("Extracting 12 files\n")
                    .with_stderr("warning: one\n"),
            )
            .scripting_call(
                1,
                ScriptedCall::new()
                    .exiting_with(3)
                    .with_stdout("Packing\n")
                    .with_stderr("Unhandled exception\n"),
            );
        let exe = Path::new("Archive2.exe");
        let cwd = Path::new(r"C:\Games\Fallout4\Data");

        let first = runner.run_capturing(exe, &[], cwd).unwrap();
        let second = runner.run_capturing(exe, &[], cwd).unwrap();
        let third = runner.run_capturing(exe, &[], cwd).unwrap();

        assert_eq!(first.status.code(), Some(0));
        assert_eq!(first.stdout, "Extracting 12 files\n");
        assert_eq!(first.stderr, "warning: one\n");

        // A non-zero exit is reported, not raised, exactly as for `run`.
        assert_eq!(second.status.code(), Some(3));
        assert_eq!(second.stdout, "Packing\n");
        assert_eq!(second.stderr, "Unhandled exception\n");

        // Unscripted: today's defaults — success and no output.
        assert!(third.status.success());
        assert_eq!(third.stdout, "");
        assert_eq!(third.stderr, "");
    }

    #[test]
    fn an_unscripted_capturing_call_falls_back_to_the_runner_wide_exit_code() {
        let runner = RecordingProcessRunner::new().returning_exit_code(2);

        let output = runner
            .run_capturing(Path::new("Archive2.exe"), &[], Path::new("Data"))
            .unwrap();

        assert_eq!(output.status.code(), Some(2));
    }

    #[test]
    fn a_scripted_exit_code_also_applies_to_a_run() {
        let runner =
            RecordingProcessRunner::new().scripting_call(1, ScriptedCall::new().exiting_with(5));
        let exe = Path::new("CreationKit.exe");
        let cwd = Path::new("Fallout4");

        assert!(runner.run(exe, &[], cwd).unwrap().success());
        assert_eq!(runner.run(exe, &[], cwd).unwrap().code(), Some(5));
    }

    #[test]
    fn a_per_call_effect_sees_its_own_calls_exe_and_args() {
        let space = InMemoryFileSpace::new();
        let seen = std::cell::RefCell::new(Vec::new());
        let runner = RecordingProcessRunner::new()
            .scripting_call(
                0,
                ScriptedCall::new().with_effect(&space, |space, exe, args| {
                    seen.borrow_mut().push((exe.to_path_buf(), args.to_vec()));
                    space.add_file_with_contents(Path::new(r"C:\Data\meshes\a.nif"), "nif");
                }),
            )
            .scripting_call(
                1,
                ScriptedCall::new().with_effect(&space, |space, exe, args| {
                    seen.borrow_mut().push((exe.to_path_buf(), args.to_vec()));
                    space.add_file_with_contents(Path::new(r"C:\Data\Mod - Main.ba2"), "ba2");
                }),
            );
        let cwd = Path::new(r"C:\Data");

        runner
            .run_capturing(
                Path::new("Archive2.exe"),
                &[
                    OsString::from("Mod - Main.ba2"),
                    OsString::from("-extract=."),
                ],
                cwd,
            )
            .unwrap();
        assert!(space.is_file(Path::new(r"C:\Data\meshes\a.nif")));
        assert!(!space.is_file(Path::new(r"C:\Data\Mod - Main.ba2")));

        runner
            .run_capturing(Path::new("BSArch.exe"), &[OsString::from("Pack")], cwd)
            .unwrap();
        assert!(space.is_file(Path::new(r"C:\Data\Mod - Main.ba2")));

        assert_eq!(
            *seen.borrow(),
            vec![
                (
                    PathBuf::from("Archive2.exe"),
                    vec![
                        OsString::from("Mod - Main.ba2"),
                        OsString::from("-extract=.")
                    ],
                ),
                (PathBuf::from("BSArch.exe"), vec![OsString::from("Pack")]),
            ]
        );
    }

    #[test]
    fn a_per_call_effect_replaces_the_shared_effect_for_that_call_only() {
        let space = InMemoryFileSpace::new();
        let shared = PathBuf::from(r"C:\shared.txt");
        let scripted = PathBuf::from(r"C:\scripted.txt");
        let runner = {
            let shared = shared.clone();
            let scripted = scripted.clone();
            RecordingProcessRunner::new()
                .with_effects(&space, move |space| {
                    space.add_file_with_contents(&shared, "shared");
                })
                .scripting_call(
                    0,
                    ScriptedCall::new().with_effect(&space, move |space, _exe, _args| {
                        space.add_file_with_contents(&scripted, "scripted");
                    }),
                )
        };
        let exe = Path::new("tool.exe");
        let cwd = Path::new("cwd");

        runner.run_capturing(exe, &[], cwd).unwrap();
        assert!(space.is_file(&scripted));
        assert!(!space.is_file(&shared));

        // An unscripted call keeps today's behaviour: the shared effect, if one is installed.
        runner.run_capturing(exe, &[], cwd).unwrap();
        assert!(space.is_file(&shared));
    }

    #[test]
    fn recording_runner_records_runs_capturing_runs_and_spawns_in_one_ordered_list() {
        let runner = RecordingProcessRunner::new();
        let ck = PathBuf::from(r"C:\Games\Fallout4\CreationKit.exe");
        let archive2 = PathBuf::from(r"C:\Games\Fallout4\Tools\Archive2\Archive2.exe");
        let fo4edit = PathBuf::from(r"C:\Tools\FO4Edit\FO4Edit.exe");
        let fo4dir = PathBuf::from(r"C:\Games\Fallout4");
        let data = PathBuf::from(r"C:\Games\Fallout4\Data");

        runner
            .run(&ck, &[OsString::from("-BuildCDX:My Mod.esp")], &fo4dir)
            .unwrap();
        runner
            .run_capturing(
                &archive2,
                &[OsString::from("-create=My Mod - Main.ba2")],
                &data,
            )
            .unwrap();
        runner
            .spawn(&fo4edit, &[OsString::from("-fo4")], &fo4dir)
            .unwrap();

        assert_eq!(
            runner.calls(),
            vec![
                RecordedProcessCall {
                    exe: ck,
                    args: vec![OsString::from("-BuildCDX:My Mod.esp")],
                    cwd: fo4dir.clone(),
                    kind: ProcessCallKind::Run,
                },
                RecordedProcessCall {
                    exe: archive2,
                    args: vec![OsString::from("-create=My Mod - Main.ba2")],
                    cwd: data,
                    kind: ProcessCallKind::RunCapturing,
                },
                RecordedProcessCall {
                    exe: fo4edit,
                    args: vec![OsString::from("-fo4")],
                    cwd: fo4dir,
                    kind: ProcessCallKind::Spawn,
                },
            ]
        );
    }

    /// Spawn a stand-in FO4Edit on a recording runner; the exe, args and cwd are irrelevant
    /// to the tests that use it, which only exercise the returned process.
    fn spawn_fo4edit(runner: &RecordingProcessRunner<'_>) -> Box<dyn RunningProcess> {
        runner
            .spawn(Path::new("FO4Edit.exe"), &[], Path::new("Temp"))
            .unwrap()
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

    /// The capturing half of the system adapter, against a real child process.
    ///
    /// The probe writes one line to stdout and another to stderr, then runs the marker-file
    /// check, so a single run shows that the streams arrive separately, the exit code is
    /// reported (not raised), and `cwd` and `args` are honoured as in [`run`].
    ///
    /// [`run`]: ProcessRunner::run
    #[test]
    fn system_runner_captures_stdout_and_stderr_separately() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("marker.txt"), b"marker").unwrap();
        let runner = SystemProcessRunner;

        let output = runner
            .run_capturing(Path::new(SHELL), &capturing_probe_args(), dir.path())
            .unwrap();

        assert_eq!(output.status.code(), Some(7));
        assert_eq!(output.stdout.trim_end(), "to-stdout");
        assert_eq!(output.stderr.trim_end(), "to-stderr");
    }

    #[test]
    fn system_runner_run_capturing_surfaces_a_missing_executable_as_an_error() {
        let dir = tempdir().unwrap();
        let runner = SystemProcessRunner;

        assert!(
            runner
                .run_capturing(&dir.path().join("no-such-tool.exe"), &[], dir.path())
                .is_err()
        );
    }

    /// The spawning half of the system adapter, against a real child process.
    ///
    /// The same marker-file probe as the `run` test, so a wrong `cwd` or mangled `args` changes
    /// the exit code `try_wait` eventually reports.
    #[test]
    fn system_runner_spawns_a_live_process_with_the_given_args_and_working_directory() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("marker.txt"), b"marker").unwrap();
        let runner = SystemProcessRunner;

        let mut process = runner
            .spawn(Path::new(SHELL), &marker_probe_args(), dir.path())
            .unwrap();

        assert_ne!(process.id(), 0);
        let status = poll_until(|| process.try_wait().unwrap())
            .expect("the probe shell should exit within the poll budget");
        assert_eq!(status.code(), Some(7));
    }

    #[test]
    fn system_runner_spawn_surfaces_a_missing_executable_as_an_error() {
        let dir = tempdir().unwrap();
        let runner = SystemProcessRunner;

        assert!(
            runner
                .spawn(&dir.path().join("no-such-tool.exe"), &[], dir.path())
                .is_err()
        );
    }

    /// Dropping the handle must not take the process with it: FO4Edit saves its merge only on
    /// its own close path. The child writes `done.txt` only after a short delay, so a drop that
    /// killed it would leave the file missing.
    #[test]
    fn dropping_a_spawned_process_leaves_it_running() {
        let dir = tempdir().unwrap();
        let done = dir.path().join("done.txt");
        let runner = SystemProcessRunner;

        let process = runner
            .spawn(Path::new(SHELL), &delayed_marker_args(), dir.path())
            .unwrap();
        drop(process);

        assert!(
            !done.exists(),
            "the child should still be in its delay when the handle is dropped"
        );
        assert!(
            poll_until(|| done.exists().then_some(())).is_some(),
            "the child should finish and write done.txt after its handle was dropped"
        );
    }

    /// Call `probe` every 20 ms until it returns `Some`, giving up after about 10 seconds.
    ///
    /// Bounded so a child that never exits fails the test instead of hanging the suite.
    fn poll_until<T>(mut probe: impl FnMut() -> Option<T>) -> Option<T> {
        for _ in 0..500 {
            if let Some(value) = probe() {
                return Some(value);
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        None
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

    /// Arguments for a shell that prints `to-stdout` to stdout and `to-stderr` to stderr, then
    /// exits 7 when `marker.txt` sits in its working directory.
    ///
    /// Bare tokens, for the reason given on [`marker_probe_args`]; `cmd` echoes the space
    /// before each `&` and redirect, which the test trims.
    #[cfg(windows)]
    fn capturing_probe_args() -> Vec<OsString> {
        [
            "/C",
            "echo",
            "to-stdout",
            "&",
            "echo",
            "to-stderr",
            "1>&2",
            "&",
            "if",
            "exist",
            "marker.txt",
            "exit",
            "7",
        ]
        .into_iter()
        .map(OsString::from)
        .collect()
    }

    #[cfg(not(windows))]
    fn capturing_probe_args() -> Vec<OsString> {
        vec![
            OsString::from("-c"),
            OsString::from("echo to-stdout; echo to-stderr >&2; test -f marker.txt && exit 7"),
        ]
    }

    /// Arguments for a shell that waits about two seconds, then writes `done.txt` into its
    /// working directory.
    ///
    /// `ping` stands in for a sleep because `timeout` refuses to run without console input.
    /// Its output goes to a file in the temporary directory rather than into the test output.
    /// Bare tokens again, for the reason given on [`marker_probe_args`]; the `&` reaches `cmd`
    /// unquoted and separates the two commands.
    #[cfg(windows)]
    fn delayed_marker_args() -> Vec<OsString> {
        [
            "/C",
            "ping",
            "-n",
            "3",
            "127.0.0.1",
            ">ping.txt",
            "&",
            "echo",
            "done>done.txt",
        ]
        .into_iter()
        .map(OsString::from)
        .collect()
    }

    #[cfg(not(windows))]
    fn delayed_marker_args() -> Vec<OsString> {
        vec![
            OsString::from("-c"),
            OsString::from("sleep 2; echo done > done.txt"),
        ]
    }
}
