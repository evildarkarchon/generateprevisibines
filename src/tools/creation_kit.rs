//! Creation Kit invocation (`:RunCK` in batch).
//!
//! [`CreationKitOps`] owns the whole Creation Kit episode: the ENB/ReShade DLL guard, the
//! stale-log delete, the spawn, the mandated MO2 sync delay, the session-log fold-in, and the
//! non-zero-exit policy. Callers state build meaning — "generate the precombines for this
//! plugin in this build mode" — and never Creation Kit's command grammar, which is why
//! [`CkOperation`] and the batch qualifier strings are private to this module.

use std::ffi::OsString;
use std::path::PathBuf;

use crate::config::BuildMode;
use crate::error::Result;
use crate::files::FileSpace;
use crate::logging;
use crate::tools::dll::DllGuard;
use crate::tools::process::ProcessRunner;
use crate::tools::wait::{MO2_DELAY_AFTER_CK_SECS, Wait};

/// Creation Kit command-line operations from the batch workflow.
///
/// Deliberately private. A Workflow Operation is the step's build meaning *before* an adapter
/// is chosen, so CK's own verb enum must not cross the seam — the four domain methods on
/// [`CreationKitOps`] are what callers see.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CkOperation {
    GeneratePrecombined,
    CompressPsg,
    BuildCdx,
    GeneratePreVisData,
}

impl CkOperation {
    const fn flag(self) -> &'static str {
        match self {
            Self::GeneratePrecombined => "GeneratePrecombined",
            Self::CompressPsg => "CompressPSG",
            Self::BuildCdx => "BuildCDX",
            Self::GeneratePreVisData => "GeneratePreVisData",
        }
    }
}

/// What a completed Creation Kit run leaves for the caller to judge.
///
/// Deliberately carries no exit status: the batch's `:RunCK` treats a non-zero exit uniformly
/// across all four operations, and that policy is settled inside [`CreationKitOps::run`], so a
/// field for it would have no reader. Success criteria stay with the Workflow Operation, which
/// is what the log content is for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CkRun {
    /// Creation Kit log contents after the run.
    ///
    /// `None` when Creation Kit wrote no log — a state the batch distinguishes explicitly
    /// ("Unable to find log"). A log that exists but cannot be read is an error, not a `None`.
    /// The Generate Precombines Operation reads this for its handle-array scan, which is why
    /// the log never reaches a Workflow Operation as a path.
    pub(crate) log: Option<String>,
}

/// The resolved paths one Workflow Run's Creation Kit episodes run against.
///
/// Preparation resolves these once, from the Workflow Toolchain; execution binds them to the
/// three ports an episode runs through. Deliberately not a shared tool-context bag: it carries
/// exactly what [`CreationKitOps::new`] needs, nothing reads it for anything else, and it has
/// no `Default` — "no Creation Kit was prepared" is an absent value, not one assembled out of
/// empty paths.
#[derive(Debug, Clone)]
pub(crate) struct CreationKitPaths {
    /// The resolved `CreationKit.exe`.
    pub(crate) exe: PathBuf,
    /// The Fallout 4 install directory: the spawn's working directory and the DLL guard's root.
    pub(crate) fallout4_dir: PathBuf,
    /// The log CKPE configures Creation Kit to write.
    pub(crate) ck_log_path: PathBuf,
    /// The Workflow Run's session log, which each episode folds its Creation Kit log into.
    pub(crate) session_log: PathBuf,
}

impl CreationKitPaths {
    /// Bind the resolved paths to the three ports one Creation Kit episode runs through.
    ///
    /// The ports are chosen at execution time rather than stored here, so the paths a Workflow
    /// Run carries stay plain data and a test can drive the real episode over recording ports.
    pub(crate) fn bind<'a>(
        &'a self,
        process: &'a dyn ProcessRunner,
        wait: &'a dyn Wait,
        files: &'a dyn FileSpace,
    ) -> CreationKitOps<'a> {
        CreationKitOps::new(
            self.exe.clone(),
            self.fallout4_dir.clone(),
            self.ck_log_path.clone(),
            Some(self.session_log.clone()),
            process,
            wait,
            files,
        )
    }
}

/// Creation Kit process launcher with DLL guard, log lifecycle and MO2 sync delay.
///
/// Holds its own narrow inputs rather than a shared context bag, and borrows the three ports
/// the episode is built from for its lifetime.
#[derive(Debug)]
pub(crate) struct CreationKitOps<'a> {
    exe: PathBuf,
    fallout4_dir: PathBuf,
    ck_log_path: PathBuf,
    session_log: Option<PathBuf>,
    process: &'a dyn ProcessRunner,
    wait: &'a dyn Wait,
    files: &'a dyn FileSpace,
}

impl<'a> CreationKitOps<'a> {
    /// Build the Creation Kit adapter from resolved paths and the three ports it runs through.
    ///
    /// `ck_log_path` is the log CKPE configures Creation Kit to write; this adapter owns its
    /// whole lifecycle, deleting it before each run and folding it into `session_log` after.
    /// `session_log` is `None` when the run has no session log to fold into.
    #[must_use]
    pub(crate) const fn new(
        exe: PathBuf,
        fallout4_dir: PathBuf,
        ck_log_path: PathBuf,
        session_log: Option<PathBuf>,
        process: &'a dyn ProcessRunner,
        wait: &'a dyn Wait,
        files: &'a dyn FileSpace,
    ) -> Self {
        Self {
            exe,
            fallout4_dir,
            ck_log_path,
            session_log,
            process,
            wait,
            files,
        }
    }

    /// Generate precombined meshes for `plugin_file` (batch `:Precomb2`, lines 262-266).
    ///
    /// A Clean build passes `clean all`; Filtered and Xbox both pass `filtered all`.
    pub(crate) fn generate_precombined(
        &self,
        plugin_file: &str,
        build_mode: BuildMode,
    ) -> Result<CkRun> {
        self.run(
            CkOperation::GeneratePrecombined,
            plugin_file,
            precombine_qualifiers(build_mode),
        )
    }

    /// Compress the geometry PSG produced by the precombine run (batch `:CompPSG`).
    // No caller until the Step 3 Workflow Operation lands; it stays live through its tests
    // because the batch's four Creation Kit operations are one episode with one shared `run`,
    // and splitting three of them out to add back later would be churn.
    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "the Step 3 Workflow Operation that calls this is not ported yet"
        )
    )]
    pub(crate) fn compress_psg(&self, plugin_file: &str) -> Result<CkRun> {
        self.run(CkOperation::CompressPsg, plugin_file, "")
    }

    /// Build the CDX index (batch `:BuildCDX`).
    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "the Step 4 Workflow Operation that calls this is not ported yet"
        )
    )]
    pub(crate) fn build_cdx(&self, plugin_file: &str) -> Result<CkRun> {
        self.run(CkOperation::BuildCdx, plugin_file, "")
    }

    /// Generate previs data (batch `:PreVis`).
    ///
    /// The qualifiers are `clean all` for every build mode — the batch hardcodes them here,
    /// unlike the precombine run, which is why this method takes no [`BuildMode`]. That is the
    /// shipped behaviour and is deliberately preserved, not a divergence to fix.
    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "the Step 6 Workflow Operation that calls this is not ported yet"
        )
    )]
    pub(crate) fn generate_previs_data(&self, plugin_file: &str) -> Result<CkRun> {
        self.run(CkOperation::GeneratePreVisData, plugin_file, "clean all")
    }

    /// Run one Creation Kit operation end to end (`START /wait` equivalent).
    ///
    /// The order below is the batch's `:RunCK`, and every part of it is a required workaround
    /// rather than incidental sequencing — see `docs/workarounds.md` §2 and §3:
    ///
    /// 1. disable the ENB/ReShade DLLs, which crash Creation Kit
    /// 2. delete any stale Creation Kit log, so what is read back is only this run's
    /// 3. spawn, with the Fallout 4 directory as the working directory
    /// 4. wait for MO2's virtual filesystem to sync
    /// 5. read the log once, serving both the session-log append and the return value
    /// 6. fold it into the session log
    /// 7. warn — never fail — on a non-zero exit; the Workflow Operation's postconditions
    ///    decide whether the step succeeded
    /// 8. restore the DLLs as the guard drops, on every exit path including an error
    ///
    /// Returns [`crate::error::Error::Io`] when the DLL guard, the stale-log delete, the spawn
    /// itself, or reading back a log Creation Kit did write fails.
    fn run(&self, operation: CkOperation, plugin_file: &str, qualifiers: &str) -> Result<CkRun> {
        let _dll_guard = DllGuard::disable(self.files, &self.fallout4_dir)?;

        if self.files.is_file(&self.ck_log_path) {
            self.files.remove_file(&self.ck_log_path)?;
        }

        let mut args = vec![OsString::from(operation_arg(operation, plugin_file))];
        args.extend(qualifier_args(qualifiers).map(OsString::from));
        let status = self.process.run(&self.exe, &args, &self.fallout4_dir)?;

        self.wait.sync_delay(MO2_DELAY_AFTER_CK_SECS);

        let log = self.read_ck_log()?;
        if let (Some(session), Some(contents)) = (&self.session_log, &log) {
            logging::append_ck_log(session, contents, self.files)?;
        }

        if !status.success() {
            tracing::warn!(
                code = ?status.code(),
                "Creation Kit exited with non-zero status; workflow operation postconditions determine success"
            );
        }

        Ok(CkRun { log })
    }

    /// The Creation Kit log after a run, or `None` when Creation Kit wrote none.
    ///
    /// A log that is *there* but cannot be read raises [`crate::error::Error::Io`] rather than
    /// answering `None`. The two states are not interchangeable: the caller's handle-array scan
    /// clears a run it finds no marker in, so folding a locked or unreadable log into "no log"
    /// would let a Creation Kit failure pass as a success. Only Creation Kit having written
    /// nothing at all — the batch's "Unable to find log" — is a `None`.
    fn read_ck_log(&self) -> Result<Option<String>> {
        if !self.files.is_file(&self.ck_log_path) {
            return Ok(None);
        }

        self.files.read_lossy(&self.ck_log_path).map(Some)
    }
}

/// CK `-GeneratePrecombined` qualifier string (batch lines 262-266).
const fn precombine_qualifiers(build_mode: BuildMode) -> &'static str {
    if matches!(build_mode, BuildMode::Clean) {
        "clean all"
    } else {
        "filtered all"
    }
}

fn operation_arg(operation: CkOperation, plugin_file: &str) -> String {
    format!("-{}:\"{plugin_file}\"", operation.flag())
}

fn qualifier_args(qualifiers: &str) -> impl Iterator<Item = &str> {
    qualifiers.split_whitespace()
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::path::Path;
    use std::process::ExitStatus;

    use super::*;
    use crate::files::InMemoryFileSpace;
    use crate::tools::process::{RecordedProcessCall, RecordingProcessRunner};
    use crate::tools::wait::RecordingWait;

    fn fallout4_dir() -> PathBuf {
        PathBuf::from("Fallout4")
    }

    fn creation_kit_exe() -> PathBuf {
        fallout4_dir().join("CreationKit.exe")
    }

    fn ck_log() -> PathBuf {
        fallout4_dir().join("CK.log")
    }

    fn session_log() -> PathBuf {
        PathBuf::from("in-memory-temp").join("MyMod.log")
    }

    fn enb_dll() -> PathBuf {
        fallout4_dir().join("d3d11.dll")
    }

    fn disabled_enb_dll() -> PathBuf {
        fallout4_dir().join("d3d11.dll-PJMdisabled")
    }

    /// A quiet Creation Kit log: readable and free of any failure marker.
    const QUIET_CK_LOG: &str = "Masterfile: Fallout4.esm\n";

    /// Build the adapter under test over the supplied ports, with a session log configured.
    fn ops<'a>(
        process: &'a dyn ProcessRunner,
        wait: &'a dyn Wait,
        files: &'a dyn FileSpace,
    ) -> CreationKitOps<'a> {
        CreationKitOps::new(
            creation_kit_exe(),
            fallout4_dir(),
            ck_log(),
            Some(session_log()),
            process,
            wait,
            files,
        )
    }

    /// Run one Creation Kit operation over a fresh space and return the single recorded spawn.
    ///
    /// The assertion subject is what reaches `CreateProcess`, so the recorded `args` are the
    /// answer rather than any helper's return value.
    fn spawn_for(operation: impl Fn(&CreationKitOps<'_>) -> Result<CkRun>) -> RecordedProcessCall {
        let files = InMemoryFileSpace::new();
        let process = RecordingProcessRunner::new();
        let wait = RecordingWait::new();

        operation(&ops(&process, &wait, &files)).unwrap();

        let mut calls = process.calls();
        assert_eq!(calls.len(), 1, "expected exactly one Creation Kit spawn");
        calls.remove(0)
    }

    /// A [`ProcessRunner`] whose spawn fails, standing in for an unlaunchable `CreationKit.exe`.
    #[derive(Debug)]
    struct FailingProcessRunner;

    impl ProcessRunner for FailingProcessRunner {
        fn run(&self, _exe: &Path, _args: &[OsString], _cwd: &Path) -> Result<ExitStatus> {
            Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "CreationKit.exe could not be launched",
            )
            .into())
        }
    }

    /// One observable moment in a Creation Kit episode.
    ///
    /// Only the episode's *own* mutations are moments. Creation Kit's simulated output goes
    /// straight into the underlying space so the tool's writes never appear here as if the
    /// adapter had made them.
    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Moment {
        Renamed(PathBuf, PathBuf),
        Removed(PathBuf),
        Spawned,
        Waited(u64),
        Appended(PathBuf),
    }

    type Timeline = RefCell<Vec<Moment>>;

    /// A [`FileSpace`] that records the mutations made through it onto a shared timeline.
    ///
    /// The DLL guard, the stale-log delete and the session-log append all reach the filesystem
    /// through this seam, and the spawn and the delay reach their own — so one timeline shared
    /// by all three doubles is what turns "these things happened" into "they happened in this
    /// order", which is the whole point of workarounds §2 and §3.
    #[derive(Debug)]
    struct TracingFileSpace<'a> {
        inner: &'a InMemoryFileSpace,
        timeline: &'a Timeline,
    }

    impl FileSpace for TracingFileSpace<'_> {
        fn is_file(&self, path: &Path) -> bool {
            self.inner.is_file(path)
        }

        fn find_first_file_with_extension(
            &self,
            directory: &Path,
            extension: &str,
        ) -> Option<PathBuf> {
            self.inner
                .find_first_file_with_extension(directory, extension)
        }

        fn remove_file(&self, path: &Path) -> Result<()> {
            self.timeline
                .borrow_mut()
                .push(Moment::Removed(path.to_path_buf()));
            self.inner.remove_file(path)
        }

        fn remove_dir_all(&self, directory: &Path) -> Result<()> {
            self.inner.remove_dir_all(directory)
        }

        fn read_lossy(&self, path: &Path) -> Result<String> {
            self.inner.read_lossy(path)
        }

        fn rename(&self, from: &Path, to: &Path) -> Result<()> {
            self.timeline
                .borrow_mut()
                .push(Moment::Renamed(from.to_path_buf(), to.to_path_buf()));
            self.inner.rename(from, to)
        }

        fn exists(&self, path: &Path) -> bool {
            self.inner.exists(path)
        }

        fn write(&self, path: &Path, contents: &str) -> Result<()> {
            self.inner.write(path, contents)
        }

        fn append(&self, path: &Path, contents: &str) -> Result<()> {
            self.timeline
                .borrow_mut()
                .push(Moment::Appended(path.to_path_buf()));
            self.inner.append(path, contents)
        }

        fn temp_dir(&self) -> PathBuf {
            self.inner.temp_dir()
        }
    }

    /// A [`Wait`] that records its delays onto the shared timeline instead of sleeping.
    #[derive(Debug)]
    struct TracingWait<'a> {
        timeline: &'a Timeline,
    }

    impl Wait for TracingWait<'_> {
        fn sync_delay(&self, seconds: u64) {
            self.timeline.borrow_mut().push(Moment::Waited(seconds));
        }
    }

    /// The episode's mandated ordering, asserted as one exact sequence.
    ///
    /// DLLs disabled before the spawn and restored after it; the stale log deleted before the
    /// spawn; the ten-second MO2 delay after the spawn and before the session-log append.
    #[test]
    fn the_creation_kit_episode_keeps_its_mandated_order() {
        let timeline = Timeline::default();
        let inner = InMemoryFileSpace::new();
        // Something for the guard to move, and a log left by a previous run for the episode to
        // clear — without the delete, the log read back could be this stale one.
        inner.add_file_with_contents(enb_dll(), "enb");
        inner.add_file_with_contents(ck_log(), "stale\n");

        let files = TracingFileSpace {
            inner: &inner,
            timeline: &timeline,
        };
        let wait = TracingWait {
            timeline: &timeline,
        };
        let process = RecordingProcessRunner::new().with_effects(&inner, |space| {
            timeline.borrow_mut().push(Moment::Spawned);
            space.add_file_with_contents(ck_log(), QUIET_CK_LOG);
        });

        ops(&process, &wait, &files)
            .generate_precombined("MyMod.esp", BuildMode::Clean)
            .unwrap();

        assert_eq!(
            timeline.borrow().as_slice(),
            [
                Moment::Renamed(enb_dll(), disabled_enb_dll()),
                Moment::Removed(ck_log()),
                Moment::Spawned,
                Moment::Waited(MO2_DELAY_AFTER_CK_SECS),
                Moment::Appended(session_log()),
                Moment::Renamed(disabled_enb_dll(), enb_dll()),
            ]
        );
        // The restore is a real round trip, not a placeholder recreated at the same path.
        assert_eq!(inner.read_lossy(&enb_dll()).unwrap(), "enb");
    }

    /// The quoted plugin name is one argv entry, and the qualifiers are separate ones.
    ///
    /// Asserted against what the process port received rather than against the formatted Rust
    /// string, because a helper returning `-GeneratePrecombined:"My Mod.esp"` says nothing
    /// about how many arguments reach `CreateProcess`.
    #[test]
    fn the_plugin_argument_stays_one_argv_entry_beside_separate_qualifiers() {
        let call = spawn_for(|ck| ck.generate_precombined("My Mod.esp", BuildMode::Clean));

        assert_eq!(
            call,
            RecordedProcessCall {
                exe: creation_kit_exe(),
                args: vec![
                    OsString::from("-GeneratePrecombined:\"My Mod.esp\""),
                    OsString::from("clean"),
                    OsString::from("all"),
                ],
                cwd: fallout4_dir(),
            }
        );
    }

    #[test]
    fn generate_precombined_qualifiers_follow_the_build_mode() {
        let cases = [
            (BuildMode::Clean, "clean"),
            (BuildMode::Filtered, "filtered"),
            (BuildMode::Xbox, "filtered"),
        ];

        for (build_mode, expected_first_qualifier) in cases {
            let call = spawn_for(|ck| ck.generate_precombined("MyMod.esp", build_mode));

            assert_eq!(
                call.args,
                vec![
                    OsString::from("-GeneratePrecombined:\"MyMod.esp\""),
                    OsString::from(expected_first_qualifier),
                    OsString::from("all"),
                ],
                "build mode: {build_mode:?}"
            );
            assert_eq!(call.cwd, fallout4_dir(), "build mode: {build_mode:?}");
        }
    }

    #[test]
    fn compress_psg_and_build_cdx_pass_no_qualifiers() {
        let compress = spawn_for(|ck| ck.compress_psg("MyMod.esp"));
        let cdx = spawn_for(|ck| ck.build_cdx("MyMod.esp"));

        assert_eq!(
            compress.args,
            vec![OsString::from("-CompressPSG:\"MyMod.esp\"")]
        );
        assert_eq!(cdx.args, vec![OsString::from("-BuildCDX:\"MyMod.esp\"")]);
        assert_eq!(compress.cwd, fallout4_dir());
        assert_eq!(cdx.cwd, fallout4_dir());
    }

    /// Previs data is generated with `clean all` whatever the build mode.
    ///
    /// One case covers every build mode because the method takes none: the batch hardcodes
    /// these qualifiers, and the absent parameter is what makes that unconditional.
    #[test]
    fn generate_previs_data_always_passes_clean_all() {
        let call = spawn_for(|ck| ck.generate_previs_data("MyMod.esp"));

        assert_eq!(
            call.args,
            vec![
                OsString::from("-GeneratePreVisData:\"MyMod.esp\""),
                OsString::from("clean"),
                OsString::from("all"),
            ]
        );
        assert_eq!(call.cwd, fallout4_dir());
    }

    /// The log is read once, and that one read serves both the session log and the caller.
    #[test]
    fn the_creation_kit_log_reaches_both_the_session_log_and_the_caller() {
        let files = InMemoryFileSpace::new();
        let wait = RecordingWait::new();
        let process = RecordingProcessRunner::new().with_effects(&files, |space| {
            space.add_file_with_contents(ck_log(), QUIET_CK_LOG);
        });

        let ck_run = ops(&process, &wait, &files)
            .generate_precombined("MyMod.esp", BuildMode::Clean)
            .unwrap();

        assert_eq!(ck_run.log.as_deref(), Some(QUIET_CK_LOG));
        assert_eq!(
            files.read_lossy(&session_log()).unwrap(),
            format!("\n----- Creation Kit log -----\n{QUIET_CK_LOG}")
        );
        assert_eq!(wait.delays(), vec![MO2_DELAY_AFTER_CK_SECS]);
    }

    /// Creation Kit that writes no log: the batch's "Unable to find log" state.
    #[test]
    fn an_absent_creation_kit_log_yields_no_content_and_appends_nothing() {
        let files = InMemoryFileSpace::new();
        let process = RecordingProcessRunner::new();
        let wait = RecordingWait::new();

        let ck_run = ops(&process, &wait, &files).build_cdx("MyMod.esp").unwrap();

        assert!(ck_run.log.is_none());
        assert!(!files.is_file(&session_log()));
    }

    /// A log Creation Kit wrote but that cannot be read is an error, not an absent log.
    ///
    /// The distinction has teeth: the caller clears a run whose log holds no failure marker,
    /// so answering `None` here would turn a locked or unreadable log into a silent pass for a
    /// Creation Kit run that may well have failed. A recorded-but-contentless file is exactly
    /// what [`InMemoryFileSpace`] uses to model "present, unreadable".
    #[test]
    fn a_present_but_unreadable_creation_kit_log_is_an_error() {
        let files = InMemoryFileSpace::new();
        let wait = RecordingWait::new();
        let process = RecordingProcessRunner::new().with_effects(&files, |space| {
            space.add_file(ck_log());
        });

        let error = ops(&process, &wait, &files)
            .generate_precombined("MyMod.esp", BuildMode::Clean)
            .unwrap_err();

        assert!(matches!(error, crate::error::Error::Io(_)));
        // Nothing half-written reached the session log, and the DLL guard still ran to
        // completion — the failure is reported, not compounded.
        assert!(!files.is_file(&session_log()));
    }

    /// A non-zero exit is a warning, not an error: `:RunCK` treats it uniformly across all
    /// four operations, and the Workflow Operation's postconditions decide the outcome.
    #[test]
    fn a_non_zero_creation_kit_exit_is_not_an_error() {
        let files = InMemoryFileSpace::new();
        let wait = RecordingWait::new();
        let process = RecordingProcessRunner::new()
            .returning_exit_code(2)
            .with_effects(&files, |space| {
                space.add_file_with_contents(ck_log(), QUIET_CK_LOG);
            });

        let ck_run = ops(&process, &wait, &files)
            .generate_precombined("MyMod.esp", BuildMode::Filtered)
            .unwrap();

        // The log still comes back, so the caller can judge the run on its own criteria.
        assert_eq!(ck_run.log.as_deref(), Some(QUIET_CK_LOG));
    }

    /// The guard restores on every exit path, including one that never reaches the spawn's
    /// successful return.
    #[test]
    fn a_failed_spawn_still_restores_the_dlls() {
        let files = InMemoryFileSpace::new();
        files.add_file_with_contents(enb_dll(), "enb");
        let wait = RecordingWait::new();

        let error = ops(&FailingProcessRunner, &wait, &files)
            .generate_precombined("MyMod.esp", BuildMode::Clean)
            .unwrap_err();

        assert!(matches!(error, crate::error::Error::Io(_)));
        assert!(files.is_file(&enb_dll()));
        assert!(!files.is_file(&disabled_enb_dll()));
        assert_eq!(files.read_lossy(&enb_dll()).unwrap(), "enb");
        // A tool that never started has nothing for MO2 to sync.
        assert!(wait.delays().is_empty());
    }

    #[test]
    fn qualifier_args_split_batch_words() {
        assert_eq!(
            qualifier_args("filtered all").collect::<Vec<_>>(),
            vec!["filtered", "all"]
        );
        assert_eq!(
            qualifier_args("  clean   all  ").collect::<Vec<_>>(),
            vec!["clean", "all"]
        );
        assert!(qualifier_args("").collect::<Vec<_>>().is_empty());
    }
}
