//! FO4Edit invocation (`:RunScript` in batch, V2.99 lines 530–564).
//!
//! [`Fo4EditOps`] owns the whole FO4Edit episode: the plugin list, the stale-log delete, the
//! session-log banner, the launch, the wait for the script's log (dismissing this run's Module
//! Selection dialog on the way), the close sequence, the session-log fold-in, and the two fatal
//! checks every script run shares. Callers state build meaning — "merge the precombines into this
//! plugin" — and never xEdit's command grammar, which is why [`XeditScript`] and the source
//! plugins are private to this module.
//!
//! The episode returns the script's log text and does not judge it beyond the shared fatals. Each
//! step's own criterion (`Error: ` present for Step 2, `Completed: No Errors.` absent for Step 7)
//! stays with its Workflow Operation (ADR-0001/0002).
//!
//! FO4Edit has no headless mode, so the episode drives its windows: see `docs/workarounds.md`
//! and `docs/episodes.md` § FO4Edit. Every window action goes through [`DesktopWindows`] and is
//! aimed at a window of the process this run started; no input or message ever goes to any
//! other window, and FO4Edit is never killed.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::files::FileSpace;
use crate::logging;
use crate::text::contains_ignore_ascii_case;
use crate::tools::desktop::{DesktopWindows, WindowHandle};
use crate::tools::process::{ProcessRunner, RunningProcess};
use crate::tools::wait::{
    FO4EDIT_CLOSE_DELAYS_SECS, FO4EDIT_POLL_INTERVAL_SECS, FO4EDIT_STARTUP_DELAY_SECS,
    MO2_DELAY_BEFORE_FO4EDIT_SECS, Wait,
};

/// The Step 2 merge script (batch line 282), which plugin validation also requires (line 141).
pub(crate) const MERGE_COMBINED_OBJECTS_SCRIPT: &str = "Batch_FO4MergeCombinedObjectsAndCheck.pas";

/// The Step 7 merge script (batch line 326), which plugin validation also requires (line 140).
pub(crate) const MERGE_PREVIS_SCRIPT: &str = "Batch_FO4MergePrevisandCleanRefr.pas";

/// The plugin list FO4Edit is pointed at with `-P:`, in the space's temporary directory.
const PLUGINS_TXT_FILE_NAME: &str = "Plugins.txt";

/// The exact caption of xEdit's plugin-selection dialog, the only window the episode dismisses.
const MODULE_SELECTION_CAPTION: &str = "Module Selection";

/// The caption of the Module Selection button that accepts the preselected plugins.
const OK_BUTTON_CAPTION: &str = "OK";

/// Poll waits after which a still-unseen Module Selection earns its one-time hint.
///
/// 30 seconds since launch: the 5-second startup delay plus five 5-second poll waits. Counted in
/// waits rather than read from a clock so the hint lands on the same poll under `RecordingWait`.
const MODULE_SELECTION_HINT_AFTER_POLLS: u32 = 5;

/// Poll waits per "still waiting" line: one a minute.
const POLLS_PER_STILL_WAITING_LINE: u32 = 12;

/// The FO4Edit scripts the batch runs.
///
/// Deliberately private, as `CkOperation` is in the Creation Kit episode: the two domain methods
/// on [`Fo4EditOps`] are what callers see, so neither a script name nor a source plugin crosses
/// into a Workflow Operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum XeditScript {
    MergeCombinedObjects,
    MergePrevis,
}

impl XeditScript {
    /// The script's file name in FO4Edit's `Edit Scripts` folder.
    const fn file_name(self) -> &'static str {
        match self {
            Self::MergeCombinedObjects => MERGE_COMBINED_OBJECTS_SCRIPT,
            Self::MergePrevis => MERGE_PREVIS_SCRIPT,
        }
    }

    /// The Creation Kit output the script merges into the target plugin (the batch's `%4`).
    const fn source_plugin(self) -> &'static str {
        match self {
            Self::MergeCombinedObjects => "CombinedObjects.esp",
            Self::MergePrevis => "Previs.esp",
        }
    }
}

/// The resolved paths one Workflow Run's FO4Edit episodes run against.
///
/// Mirrors `CreationKitPaths`: preparation resolves these once, from the Workflow Toolchain, and
/// execution binds them to the ports an episode runs through. No `Default`, because "FO4Edit was
/// not prepared" is an absent value rather than one built from empty paths.
#[derive(Debug, Clone)]
pub(crate) struct Fo4EditPaths {
    /// The resolved FO4Edit/xEdit executable.
    pub(crate) exe: PathBuf,
    /// The `Data` directory FO4Edit is pointed at with `-D:`.
    ///
    /// `Some` only when the operator gave `-FO4:`. The batch sets `%ModDir_%` only in `:SetFO4`
    /// (line 498); otherwise FO4Edit finds the install the way it always does.
    pub(crate) data_dir_override: Option<PathBuf>,
    /// The Workflow Run's session log, which each episode folds the script's log into.
    pub(crate) session_log: PathBuf,
}

impl Fo4EditPaths {
    /// Bind the resolved paths to the ports one FO4Edit episode runs through.
    ///
    /// The ports are chosen at execution time rather than stored here, so the paths a Workflow
    /// Run carries stay plain data and a test can drive the real episode over recording ports.
    pub(crate) const fn bind<'a>(&'a self, ports: Fo4EditPorts<'a>) -> Fo4EditOps<'a> {
        Fo4EditOps { paths: self, ports }
    }
}

/// The internal seams one FO4Edit episode runs through.
///
/// Not part of what a Workflow Operation is handed (ADR-0002); crate-visible only so the tests
/// in `src/workflow/` can assemble recording ones.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Fo4EditPorts<'a> {
    /// Starts FO4Edit and reports whether it is still running.
    pub(crate) process: &'a dyn ProcessRunner,
    /// Every delay of the episode: the MO2 sync delays and the poll interval.
    pub(crate) wait: &'a dyn Wait,
    /// Lists FO4Edit's windows, dismisses Module Selection, and asks FO4Edit to close.
    pub(crate) desktop: &'a dyn DesktopWindows,
    /// `Plugins.txt`, the unattended log's lifecycle, and the session-log appends.
    pub(crate) files: &'a dyn FileSpace,
}

/// FO4Edit script runner: plugin list, log lifecycle, Module Selection, close, and MO2 delays.
#[derive(Debug)]
pub(crate) struct Fo4EditOps<'a> {
    paths: &'a Fo4EditPaths,
    ports: Fo4EditPorts<'a>,
}

/// The next thing the episode tries on a visible Module Selection.
///
/// One rung per poll. The ladder advances whether or not a rung "worked", because a posted
/// message reports only that it was posted: whether the dialog went away is for the next poll's
/// window listing to show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DismissalRung {
    /// Post `BM_CLICK` to the dialog's `OK` button (rung A; the probe on #40 ranked it first).
    ClickOk,
    /// Post ENTER to the dialog (rung C).
    PostEnter,
    /// Ask the operator to press OK, once.
    AskOperator,
    /// Everything has been tried; leave the dialog to the operator.
    Exhausted,
}

impl Fo4EditOps<'_> {
    /// Merge `CombinedObjects.esp` into `plugin_file` (batch Step 2, line 282).
    ///
    /// Returns the script's log text for the Workflow Operation to judge. Fails with the shared
    /// fatals and episode stops described on [`Self::run`].
    // No production caller until Step 2 is registered (#59); the tests below drive it.
    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "Step 2, the first caller of the FO4Edit episode, is not registered yet"
        )
    )]
    pub(crate) fn merge_combined_objects(&self, plugin_file: &str) -> Result<String> {
        self.run(XeditScript::MergeCombinedObjects, plugin_file)
    }

    /// Merge `Previs.esp` into `plugin_file` (batch Step 7, line 326).
    ///
    /// Returns the script's log text for the Workflow Operation to judge. Fails with the shared
    /// fatals and episode stops described on [`Self::run`].
    // No production caller until Step 7 is registered (#60); the tests below drive it.
    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "Step 7, the second caller of the FO4Edit episode, is not registered yet"
        )
    )]
    pub(crate) fn merge_previs(&self, plugin_file: &str) -> Result<String> {
        self.run(XeditScript::MergePrevis, plugin_file)
    }

    /// Run one FO4Edit script end to end (`:RunScript`).
    ///
    /// In the batch's order, keeping every delay:
    ///
    /// 1. write `Plugins.txt`, listing the target and the source plugin
    /// 2. delete a stale unattended log, so the poll waits for this run's
    /// 3. open the session-log entry — script, plugin and parameters
    /// 4. wait 10s, launch FO4Edit without waiting for it, wait 5s
    /// 5. poll every 5s for the log, dismissing this run's Module Selection on the way
    /// 6. ask this run's FO4Edit to close, with the batch's 10s/15s/10s delays
    /// 7. fold the log into the session log
    /// 8. check the two fatals every script run shares
    ///
    /// Returns the log text. Stops with [`Error::Fo4EditExitedEarly`] when FO4Edit exits before
    /// the log appears, [`Error::Fo4EditStillRunning`] when it ignores both close requests,
    /// [`Error::UnattendedLogMissing`] when the log is gone by the time it is read,
    /// [`Error::Fo4EditScriptMissingModules`] or [`Error::Fo4EditScriptFailed`] on the shared
    /// fatals, and [`Error::Io`] when a file operation or the launch fails.
    fn run(&self, script: XeditScript, plugin_file: &str) -> Result<String> {
        let files = self.ports.files;
        let temp_dir = files.temp_dir();
        let plugins_txt = temp_dir.join(PLUGINS_TXT_FILE_NAME);
        files.write(
            &plugins_txt,
            &plugins_txt_contents(plugin_file, script.source_plugin()),
        )?;

        let log = logging::unattended_log_path(files);
        if files.is_file(&log) {
            files.remove_file(&log)?;
        }

        // Written before the first delay rather than after the run, so a FO4Edit that hangs or
        // never launches still leaves a record of which script was attempted.
        logging::append_xedit_run_header(
            &self.paths.session_log,
            script.file_name(),
            plugin_file,
            &plugins_txt,
            self.paths.data_dir_override.as_deref(),
            files,
        )?;

        self.ports.wait.sync_delay(MO2_DELAY_BEFORE_FO4EDIT_SECS);

        let args = launch_args(
            script,
            plugin_file,
            &plugins_txt,
            self.paths.data_dir_override.as_deref(),
        );
        // The temporary directory is the working directory because `-log:` is relative; see
        // `launch_args`.
        let mut fo4edit = self
            .ports
            .process
            .spawn(&self.paths.exe, &args, &temp_dir)?;

        self.ports.wait.sync_delay(FO4EDIT_STARTUP_DELAY_SECS);

        self.wait_for_log(script, fo4edit.as_mut(), &log)?;
        self.close(fo4edit.as_mut())?;

        let contents = self.read_log(&log)?;
        logging::append_unattended_log(&self.paths.session_log, &contents, files)?;

        check_shared_fatals(script, plugin_file, &contents)?;
        Ok(contents)
    }

    /// Poll every 5 seconds until the script's log appears (batch 548–550).
    ///
    /// Each poll, in order: the log ends the wait; an exited FO4Edit stops the run; a visible
    /// Module Selection gets the next [`DismissalRung`]; a one-time hint is printed when Module
    /// Selection has not been seen 30 seconds after launch; a "still waiting" line is printed
    /// every minute. There is no timeout — a merge can take a long time, and the operator can
    /// see from the console that the run is waiting on FO4Edit.
    ///
    /// Returns [`Error::Fo4EditExitedEarly`] when FO4Edit exits first: a divergence, since the
    /// batch would poll forever.
    fn wait_for_log(
        &self,
        script: XeditScript,
        fo4edit: &mut dyn RunningProcess,
        log: &Path,
    ) -> Result<()> {
        let pid = fo4edit.id();
        let mut next_rung = DismissalRung::ClickOk;
        let mut module_selection_seen = false;
        let mut poll_waits: u32 = 0;

        loop {
            if self.ports.files.is_file(log) {
                return Ok(());
            }

            if let Some(status) = fo4edit.try_wait()? {
                return Err(Error::Fo4EditExitedEarly {
                    script: script.file_name(),
                    code: status.code(),
                });
            }

            if let Some(module_selection) = self.visible_module_selection(pid) {
                module_selection_seen = true;
                next_rung = self.try_rung(next_rung, module_selection);
            }

            // Equality rather than `>=`, so the hint is printed on exactly one poll.
            if !module_selection_seen && poll_waits == MODULE_SELECTION_HINT_AFTER_POLLS {
                tracing::info!(
                    "FO4Edit hasn't shown Module Selection; if it's showing another dialog, \
                     dismiss it"
                );
            }

            if poll_waits > 0 && poll_waits.is_multiple_of(POLLS_PER_STILL_WAITING_LINE) {
                tracing::info!(
                    "Still waiting for FO4Edit to finish {} ({} min)",
                    script.file_name(),
                    poll_waits / POLLS_PER_STILL_WAITING_LINE
                );
            }

            self.ports.wait.sync_delay(FO4EDIT_POLL_INTERVAL_SECS);
            poll_waits += 1;
        }
    }

    /// The visible window of process `pid` captioned exactly `Module Selection`, if there is one.
    ///
    /// Only this run's process is listed, and the caption must match exactly, so a Module
    /// Selection belonging to another xEdit, a developer message, or a "What's New" dialog is
    /// never chosen. A hidden window is skipped: it cannot be what is waiting on the operator.
    fn visible_module_selection(&self, pid: u32) -> Option<WindowHandle> {
        self.ports
            .desktop
            .top_level_windows(pid)
            .into_iter()
            .find(|window| window.visible && window.caption == MODULE_SELECTION_CAPTION)
            .map(|window| window.handle)
    }

    /// Try `rung` on `module_selection` and return the rung to try on the next poll.
    ///
    /// A failed action is not an error: the dialog may have gone between the listing and the
    /// action (the operator pressed OK, say), and the next poll's listing is what judges whether
    /// it is still there. Every action names `module_selection` as its target, so nothing is
    /// sent to any other window.
    fn try_rung(&self, rung: DismissalRung, module_selection: WindowHandle) -> DismissalRung {
        match rung {
            DismissalRung::ClickOk => {
                match self
                    .ports
                    .desktop
                    .click_button(module_selection, OK_BUTTON_CAPTION)
                {
                    Ok(true) => {}
                    Ok(false) => tracing::debug!("Module Selection has no OK button to click"),
                    Err(error) => tracing::debug!(%error, "clicking Module Selection's OK failed"),
                }
                DismissalRung::PostEnter
            }
            DismissalRung::PostEnter => {
                if let Err(error) = self.ports.desktop.post_enter(module_selection) {
                    tracing::debug!(%error, "posting ENTER to Module Selection failed");
                }
                DismissalRung::AskOperator
            }
            DismissalRung::AskOperator => {
                // The guaranteed fallback, and the only one under Wine, whose results for the
                // two rungs above are unverified (#41).
                tracing::warn!("Press OK in FO4Edit's Module Selection window");
                DismissalRung::Exhausted
            }
            DismissalRung::Exhausted => DismissalRung::Exhausted,
        }
    }

    /// Ask this run's FO4Edit to close, keeping the batch's delays (552–558).
    ///
    /// FO4Edit is never killed: xEdit writes the merged plugin from its own close path, so a
    /// kill could lose or truncate the merge. A FO4Edit that has already exited gets no close
    /// request, but every delay still runs, because the delays give MO2 time to move the merged
    /// plugin rather than FO4Edit time to close.
    ///
    /// Returns [`Error::Fo4EditStillRunning`] when FO4Edit is still running after both requests.
    fn close(&self, fo4edit: &mut dyn RunningProcess) -> Result<()> {
        let [before_first, before_second, before_read] = FO4EDIT_CLOSE_DELAYS_SECS;

        self.ports.wait.sync_delay(before_first);
        self.request_close_if_running(fo4edit)?;
        self.ports.wait.sync_delay(before_second);
        self.request_close_if_running(fo4edit)?;
        self.ports.wait.sync_delay(before_read);

        if fo4edit.try_wait()?.is_none() {
            return Err(Error::Fo4EditStillRunning { pid: fo4edit.id() });
        }
        Ok(())
    }

    /// Post `WM_CLOSE` to every top-level window of `fo4edit`, unless it has already exited.
    ///
    /// Every window, not only the main form: the probe on #40 found that closing all of them
    /// closes FO4Edit in about two seconds and saves the plugin. Only this process's windows are
    /// listed, so another xEdit the operator has open is never touched — the batch's close
    /// reached every `FO4Edit.exe` on the machine. A failed request is not an error: the window
    /// may already be gone, and whether FO4Edit closed is checked after the next delay.
    fn request_close_if_running(&self, fo4edit: &mut dyn RunningProcess) -> Result<()> {
        if fo4edit.try_wait()?.is_some() {
            return Ok(());
        }

        for window in self.ports.desktop.top_level_windows(fo4edit.id()) {
            if let Err(error) = self.ports.desktop.request_close(window.handle) {
                tracing::debug!(%error, "asking a FO4Edit window to close failed");
            }
        }
        Ok(())
    }

    /// Read the script's log after FO4Edit has closed.
    ///
    /// The poll already saw the log, so a log that is gone now was removed by something else
    /// during the close delays. That is an error rather than an empty log, because an empty log
    /// would be judged as a failed script for the wrong reason.
    fn read_log(&self, log: &Path) -> Result<String> {
        if !self.ports.files.is_file(log) {
            return Err(Error::UnattendedLogMissing {
                path: log.to_path_buf(),
            });
        }
        self.ports.files.read_lossy(log)
    }
}

/// The plugin list FO4Edit loads (batch 534–535): the target, then the source, each active.
///
/// CRLF, as `echo` writes it. The space `echo` leaves before each `>` is dropped: it is an
/// artifact of the batch's syntax, not part of either plugin name.
fn plugins_txt_contents(plugin_file: &str, source_plugin: &str) -> String {
    format!("*{plugin_file}\r\n*{source_plugin}\r\n")
}

/// FO4Edit's arguments for one script run (batch 543).
///
/// Each argument is one argv entry with no quotes in it, for the reason given on the Creation
/// Kit episode's `operation_arg`: the batch's quotes only survive `cmd`'s tokenizer, and
/// `Command::arg` already keeps a path with spaces in one entry.
///
/// `-log:` is relative on purpose, and the episode starts FO4Edit with the space's temporary
/// directory as its working directory, so the log still lands at `%TEMP%\UnattendedScript.log`.
/// The PJM scripts cut the `-log:` value at 60 characters before saving to it, so an absolute
/// `%TEMP%` path is already cut for a Windows user name longer than about 11 characters, and the
/// log would go somewhere the poll never looks. Changing the working directory is safe because
/// xEdit resolves a bare `-Script:` name against its `Edit Scripts` folder, not the working
/// directory, and never changes directory itself except to restore it.
fn launch_args(
    script: XeditScript,
    plugin_file: &str,
    plugins_txt: &Path,
    data_dir_override: Option<&Path>,
) -> Vec<OsString> {
    let mut args = vec![OsString::from("-fo4"), OsString::from("-autoexit")];
    args.push(flag_with_path("-P:", plugins_txt));
    if let Some(data_dir) = data_dir_override {
        args.push(flag_with_path("-D:", data_dir));
    }
    args.push(OsString::from(format!("-Script:{}", script.file_name())));
    args.push(OsString::from(format!("-Mod:{plugin_file}")));
    args.push(OsString::from(format!(
        "-log:{}",
        logging::UNATTENDED_LOG_FILE_NAME
    )));
    args
}

/// `flag` followed directly by `path`, kept as an `OsString` so a non-UTF-8 path is not mangled.
fn flag_with_path(flag: &str, path: &Path) -> OsString {
    let mut arg = OsString::from(flag);
    arg.push(path);
    arg
}

/// The two fatal checks every script run shares (batch 560–563), in the batch's order.
///
/// Both stop the run whether or not it is interactive. The batch reaches `:RunScript` through
/// `Call`, so on a non-interactive run its `goto failed` returned to the caller and the build
/// carried on into Step 3 or Step 8 with an unmerged plugin. Both markers match case-insensitively
/// anywhere in the log, as `findstr /I` does.
fn check_shared_fatals(script: XeditScript, plugin_file: &str, log: &str) -> Result<()> {
    let missing_modules = format!(
        "Error: Missing [{plugin_file}] or [{}] modules",
        script.source_plugin()
    );
    if contains_ignore_ascii_case(log, &missing_modules) {
        return Err(Error::Fo4EditScriptMissingModules {
            script: script.file_name(),
        });
    }

    if !contains_ignore_ascii_case(log, "Completed: ") {
        return Err(Error::Fo4EditScriptFailed {
            script: script.file_name(),
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::io;
    use std::rc::Rc;
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::files::InMemoryFileSpace;
    use crate::tools::desktop::{DesktopAction, RecordingDesktopWindows, ScriptedWindow};
    use crate::tools::process::{
        ExitFlag, ProcessCallKind, RECORDED_PROCESS_ID, RecordedProcessCall,
        RecordingProcessRunner, ScriptedExit,
    };
    use crate::tools::wait::RecordingWait;

    const FO4EDIT: u32 = RECORDED_PROCESS_ID;
    const OTHER_PROCESS: u32 = 77;

    /// FO4Edit's windows while Module Selection is up, as the probe on #40 recorded them.
    const MAIN_FORM: WindowHandle = WindowHandle::from_raw(0x100, FO4EDIT);
    const MODULE_SELECTION: WindowHandle = WindowHandle::from_raw(0x200, FO4EDIT);

    /// A successful script run's log, with the marker the second shared fatal looks for.
    const COMPLETED_LOG: &str = "Merging CombinedObjects.esp\nCompleted: No Errors.\n";

    /// The delays of a run whose log appears after the first poll's dismissal: 10s before the
    /// launch, 5s after it, one poll wait, then the close sequence.
    const ONE_POLL_DELAYS: [u64; 6] = [10, 5, 5, 10, 15, 10];

    fn fo4edit_exe() -> PathBuf {
        PathBuf::from(r"C:\Tools\FO4Edit\FO4Edit.exe")
    }

    fn data_dir() -> PathBuf {
        PathBuf::from(r"D:\Games\Fallout4\Data")
    }

    fn session_log(files: &dyn FileSpace) -> PathBuf {
        files.temp_dir().join("MyMod.log")
    }

    fn plugins_txt(files: &dyn FileSpace) -> PathBuf {
        files.temp_dir().join("Plugins.txt")
    }

    fn log(files: &dyn FileSpace) -> PathBuf {
        logging::unattended_log_path(files)
    }

    /// Paths for a run without `-FO4:`, logging into `files`' temporary directory.
    fn paths(files: &dyn FileSpace) -> Fo4EditPaths {
        Fo4EditPaths {
            exe: fo4edit_exe(),
            data_dir_override: None,
            session_log: session_log(files),
        }
    }

    /// Paths for a run given `-FO4:`, so FO4Edit is pointed at the run's `Data`.
    fn paths_with_override(files: &dyn FileSpace) -> Fo4EditPaths {
        Fo4EditPaths {
            data_dir_override: Some(data_dir()),
            ..paths(files)
        }
    }

    /// Run the Step 2 merge for `MyMod.esp` over the supplied ports.
    fn merge_combined_objects(
        paths: &Fo4EditPaths,
        process: &dyn ProcessRunner,
        wait: &dyn Wait,
        desktop: &dyn DesktopWindows,
        files: &dyn FileSpace,
    ) -> Result<String> {
        paths
            .bind(Fo4EditPorts {
                process,
                wait,
                desktop,
                files,
            })
            .merge_combined_objects("MyMod.esp")
    }

    /// A FO4Edit that exits once it is asked to close, and the flag that makes it.
    fn fo4edit_exiting_on_close<'a>() -> (RecordingProcessRunner<'a>, ExitFlag) {
        let exit = ExitFlag::new();
        let process = RecordingProcessRunner::new().spawning(ScriptedExit::WhenFlagged {
            flag: exit.clone(),
            code: 0,
        });
        (process, exit)
    }

    /// FO4Edit with Module Selection up: a disabled main form and the dialog with its `OK`.
    fn at_module_selection<'a>() -> RecordingDesktopWindows<'a> {
        RecordingDesktopWindows::new()
            .with_window(
                ScriptedWindow::new(MAIN_FORM, "FO4Script 4.1.5q x64")
                    .with_class("TfrmMain")
                    .disabled(),
            )
            .with_window(
                ScriptedWindow::new(MODULE_SELECTION, "Module Selection")
                    .with_class("TfrmModuleSelect")
                    .with_button("OK"),
            )
    }

    /// FO4Edit's main form alone: Module Selection never appears.
    fn without_module_selection<'a>() -> RecordingDesktopWindows<'a> {
        RecordingDesktopWindows::new().with_window(
            ScriptedWindow::new(MAIN_FORM, "FO4Script 4.1.5q x64").with_class("TfrmMain"),
        )
    }

    /// Whether `action` is one the matching rung would send to Module Selection.
    fn is_click_on_module_selection(action: &DesktopAction) -> bool {
        matches!(action, DesktopAction::ClickButton { window, caption }
            if *window == MODULE_SELECTION && caption == "OK")
    }

    /// Install the two effects of a FO4Edit that behaves: `dismissed_by` takes Module Selection
    /// away and writes [`COMPLETED_LOG`] as the script's log, as xEdit does once its plugins are
    /// chosen, and any close request makes the process exit.
    fn behaving_fo4edit<'a>(
        desktop: RecordingDesktopWindows<'a>,
        files: &'a InMemoryFileSpace,
        exit: &ExitFlag,
        dismissed_by: impl Fn(&DesktopAction) -> bool + 'a,
    ) -> RecordingDesktopWindows<'a> {
        fo4edit_logging(desktop, files, exit, dismissed_by, COMPLETED_LOG)
    }

    /// [`behaving_fo4edit`], with the script leaving `contents` as its log.
    fn fo4edit_logging<'a>(
        desktop: RecordingDesktopWindows<'a>,
        files: &'a InMemoryFileSpace,
        exit: &ExitFlag,
        dismissed_by: impl Fn(&DesktopAction) -> bool + 'a,
        contents: &str,
    ) -> RecordingDesktopWindows<'a> {
        let log = log(files);
        let contents = contents.to_owned();
        let exit = exit.clone();
        desktop
            .on_action(move |action, script| {
                if dismissed_by(action) {
                    script.remove_window(MODULE_SELECTION);
                    files.add_file_with_contents(&log, contents.as_str());
                }
            })
            .on_action(move |action, _| {
                if matches!(action, DesktopAction::RequestClose { .. }) {
                    exit.set();
                }
            })
    }

    /// The window actions that are not close requests, in order: the dismissal ladder's.
    fn dismissal_actions(desktop: &RecordingDesktopWindows<'_>) -> Vec<DesktopAction> {
        desktop
            .actions()
            .into_iter()
            .filter(|action| !matches!(action, DesktopAction::RequestClose { .. }))
            .collect()
    }

    /// The windows sent a close request, in order.
    fn close_requests(desktop: &RecordingDesktopWindows<'_>) -> Vec<WindowHandle> {
        desktop
            .actions()
            .into_iter()
            .filter_map(|action| match action {
                DesktopAction::RequestClose { window } => Some(window),
                _ => None,
            })
            .collect()
    }

    /// A [`RecordingWait`] that writes `contents` as the script's log after poll wait `n`.
    ///
    /// For runs where no window action is what makes the log appear. Counted from the first
    /// poll wait, after the 10s and 5s launch delays.
    fn log_after_poll_waits(
        files: &Rc<InMemoryFileSpace>,
        n: usize,
        contents: &str,
    ) -> RecordingWait {
        let files = Rc::clone(files);
        let log = log(files.as_ref());
        let contents = contents.to_owned();
        let delays_seen = Cell::new(0_usize);
        RecordingWait::with_effect(move |_| {
            delays_seen.set(delays_seen.get() + 1);
            // Two launch delays come before the first poll wait.
            if delays_seen.get() == n + 2 {
                files.add_file_with_contents(&log, contents.as_str());
            }
        })
    }

    /// Run `f` with the console captured, and return what it printed through `tracing`.
    ///
    /// The subscriber is the thread's default only for the duration of `f`, so tests running in
    /// parallel on other threads neither see nor disturb it.
    fn capture_console<T>(f: impl FnOnce() -> T) -> (T, String) {
        #[derive(Clone, Default)]
        struct Buffer(Arc<Mutex<Vec<u8>>>);

        impl io::Write for Buffer {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(bytes);
                Ok(bytes.len())
            }

            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        let buffer = Buffer::default();
        let writer = buffer.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(move || writer.clone())
            .with_ansi(false)
            .without_time()
            .with_target(false)
            .with_max_level(tracing::Level::INFO)
            .finish();

        let result = tracing::subscriber::with_default(subscriber, f);
        let console = String::from_utf8(buffer.0.lock().unwrap().clone()).unwrap();
        (result, console)
    }

    const PRESS_OK: &str = "Press OK in FO4Edit's Module Selection window";
    const HINT: &str = "FO4Edit hasn't shown Module Selection";

    /// The whole episode, with Module Selection dismissed by its `OK` on the first poll.
    #[test]
    fn a_merge_runs_the_whole_episode_and_returns_the_log() {
        let files = InMemoryFileSpace::new();
        // A previous run's log, which must not end this run's poll: it says "Completed" too.
        files.add_file_with_contents(log(&files), "stale\nCompleted: No Errors.\n");
        let (process, exit) = fo4edit_exiting_on_close();
        let process = process.with_effects(&files, |space| {
            assert!(
                !space.is_file(&log(space)),
                "the stale log must be gone before FO4Edit is launched"
            );
        });
        let desktop = behaving_fo4edit(
            at_module_selection(),
            &files,
            &exit,
            is_click_on_module_selection,
        );
        let wait = RecordingWait::new();

        let returned =
            merge_combined_objects(&paths(&files), &process, &wait, &desktop, &files).unwrap();

        assert_eq!(returned, COMPLETED_LOG);
        // CRLF, as `echo` writes it, without the space `echo` leaves before `>`.
        assert_eq!(
            files.contents(&plugins_txt(&files)).unwrap(),
            b"*MyMod.esp\r\n*CombinedObjects.esp\r\n"
        );
        assert_eq!(wait.delays(), ONE_POLL_DELAYS);
        assert_eq!(
            desktop.actions(),
            vec![
                DesktopAction::ClickButton {
                    window: MODULE_SELECTION,
                    caption: "OK".to_owned(),
                },
                // Dismissal took Module Selection away, so one close round reaches what is left.
                DesktopAction::RequestClose { window: MAIN_FORM },
            ]
        );
        assert_eq!(
            files.read_lossy(&session_log(&files)).unwrap(),
            format!(
                "Running xEdit script Batch_FO4MergeCombinedObjectsAndCheck.pas against MyMod.esp\n\
                 Params -fo4 -autoexit -P:\"{}\"\n\
                 ====================================\n\
                 {COMPLETED_LOG}",
                plugins_txt(&files).display()
            )
        );
    }

    /// The banner reaches the session log before the first delay, so a FO4Edit that never
    /// comes back still leaves a record of what was attempted.
    #[test]
    fn the_session_log_banner_is_written_before_the_first_delay() {
        let files = Rc::new(InMemoryFileSpace::new());
        let banner_at_first_delay = Rc::new(RefCell::new(None));
        let wait = {
            let files = Rc::clone(&files);
            let banner = Rc::clone(&banner_at_first_delay);
            RecordingWait::with_effect(move |_| {
                if banner.borrow().is_none() {
                    *banner.borrow_mut() = files.read_lossy(&session_log(files.as_ref())).ok();
                }
            })
        };
        let (process, exit) = fo4edit_exiting_on_close();
        let desktop = behaving_fo4edit(
            at_module_selection(),
            &files,
            &exit,
            is_click_on_module_selection,
        );

        merge_combined_objects(&paths(&*files), &process, &wait, &desktop, files.as_ref()).unwrap();

        let banner = banner_at_first_delay.borrow().clone().unwrap();
        assert!(
            banner.starts_with("Running xEdit script "),
            "banner: {banner}"
        );
        assert!(
            banner.ends_with("====================================\n"),
            "banner: {banner}"
        );
    }

    /// One spawn, its arguments each one argv entry with no quotes, `-log:` relative, and the
    /// temporary directory as the working directory so that relative name lands in `%TEMP%`.
    #[test]
    fn fo4edit_is_launched_with_a_relative_log_from_the_temporary_directory() {
        let files = InMemoryFileSpace::new();
        let (process, exit) = fo4edit_exiting_on_close();
        let desktop = behaving_fo4edit(
            at_module_selection(),
            &files,
            &exit,
            is_click_on_module_selection,
        );
        let wait = RecordingWait::new();

        paths(&files)
            .bind(Fo4EditPorts {
                process: &process,
                wait: &wait,
                desktop: &desktop,
                files: &files,
            })
            .merge_combined_objects("My Mod.esp")
            .unwrap();

        assert_eq!(
            process.calls(),
            vec![RecordedProcessCall {
                exe: fo4edit_exe(),
                args: vec![
                    OsString::from("-fo4"),
                    OsString::from("-autoexit"),
                    flag_with_path("-P:", &plugins_txt(&files)),
                    OsString::from("-Script:Batch_FO4MergeCombinedObjectsAndCheck.pas"),
                    OsString::from("-Mod:My Mod.esp"),
                    OsString::from("-log:UnattendedScript.log"),
                ],
                cwd: files.temp_dir(),
                kind: ProcessCallKind::Spawn,
            }]
        );
    }

    /// With `-FO4:`, FO4Edit is pointed at the run's `Data`: an unquoted `-D:` argument, and the
    /// batch's quoted `%ModDir_%` in the session log.
    #[test]
    fn a_data_override_reaches_the_arguments_and_the_session_log() {
        let files = InMemoryFileSpace::new();
        let (process, exit) = fo4edit_exiting_on_close();
        let desktop = behaving_fo4edit(
            at_module_selection(),
            &files,
            &exit,
            is_click_on_module_selection,
        );
        let wait = RecordingWait::new();

        merge_combined_objects(
            &paths_with_override(&files),
            &process,
            &wait,
            &desktop,
            &files,
        )
        .unwrap();

        let args = process.calls().remove(0).args;
        assert_eq!(
            args[2..4],
            [
                flag_with_path("-P:", &plugins_txt(&files)),
                flag_with_path("-D:", &data_dir()),
            ]
        );
        let session = files.read_lossy(&session_log(&files)).unwrap();
        assert!(
            session.contains(&format!(
                "Params -fo4 -autoexit -P:\"{}\" -D:\"{}\"\n",
                plugins_txt(&files).display(),
                data_dir().display()
            )),
            "session log: {session}"
        );
    }

    /// Step 7's script, merging `Previs.esp`.
    #[test]
    fn merge_previs_runs_its_own_script_against_previs_esp() {
        let files = InMemoryFileSpace::new();
        let (process, exit) = fo4edit_exiting_on_close();
        let desktop = behaving_fo4edit(
            at_module_selection(),
            &files,
            &exit,
            is_click_on_module_selection,
        );
        let wait = RecordingWait::new();

        paths(&files)
            .bind(Fo4EditPorts {
                process: &process,
                wait: &wait,
                desktop: &desktop,
                files: &files,
            })
            .merge_previs("MyMod.esp")
            .unwrap();

        assert_eq!(
            files.contents(&plugins_txt(&files)).unwrap(),
            b"*MyMod.esp\r\n*Previs.esp\r\n"
        );
        assert!(process.calls()[0].args.contains(&OsString::from(
            "-Script:Batch_FO4MergePrevisandCleanRefr.pas"
        )));
        assert!(files.read_lossy(&session_log(&files)).unwrap().starts_with(
            "Running xEdit script Batch_FO4MergePrevisandCleanRefr.pas against MyMod.esp\n"
        ));
    }

    /// Module Selection still up after its `OK` was clicked: the next poll posts it ENTER, and
    /// that dismisses it, so the operator is never asked.
    #[test]
    fn module_selection_that_survives_the_click_is_posted_enter() {
        let files = InMemoryFileSpace::new();
        let (process, exit) = fo4edit_exiting_on_close();
        let desktop = behaving_fo4edit(at_module_selection(), &files, &exit, |action| {
            *action
                == DesktopAction::PostEnter {
                    window: MODULE_SELECTION,
                }
        });
        let wait = RecordingWait::new();

        let (result, console) = capture_console(|| {
            merge_combined_objects(&paths(&files), &process, &wait, &desktop, &files)
        });

        result.unwrap();
        assert_eq!(
            dismissal_actions(&desktop),
            vec![
                DesktopAction::ClickButton {
                    window: MODULE_SELECTION,
                    caption: "OK".to_owned(),
                },
                DesktopAction::PostEnter {
                    window: MODULE_SELECTION,
                },
            ]
        );
        assert!(!console.contains(PRESS_OK), "console: {console}");
    }

    /// A Module Selection that survives both rungs gets one instruction to the operator and
    /// nothing more, however long it stays. It was seen before 30 seconds, so no hint either.
    #[test]
    fn module_selection_that_survives_both_rungs_is_left_to_the_operator() {
        let files = Rc::new(InMemoryFileSpace::new());
        let (process, exit) = fo4edit_exiting_on_close();
        // The operator presses OK after poll wait 8: well past the hint's 30 seconds.
        let wait = log_after_poll_waits(&files, 8, COMPLETED_LOG);
        let desktop = behaving_fo4edit(at_module_selection(), &files, &exit, |_| false);

        let (result, console) = capture_console(|| {
            merge_combined_objects(&paths(&*files), &process, &wait, &desktop, files.as_ref())
        });

        result.unwrap();
        assert_eq!(
            dismissal_actions(&desktop),
            vec![
                DesktopAction::ClickButton {
                    window: MODULE_SELECTION,
                    caption: "OK".to_owned(),
                },
                DesktopAction::PostEnter {
                    window: MODULE_SELECTION,
                },
            ]
        );
        assert_eq!(console.matches(PRESS_OK).count(), 1, "console: {console}");
        assert!(console.contains("WARN"), "console: {console}");
        assert!(!console.contains(HINT), "console: {console}");
    }

    /// A click that finds no `OK` button is not an error: the ladder moves on, and the next
    /// poll's listing decides whether the dialog is still there.
    #[test]
    fn a_click_that_finds_no_ok_button_moves_the_ladder_on() {
        let files = InMemoryFileSpace::new();
        let (process, exit) = fo4edit_exiting_on_close();
        // The click finds no `OK` (none is scripted), so only ENTER can dismiss.
        let desktop = without_module_selection()
            .with_window(ScriptedWindow::new(MODULE_SELECTION, "Module Selection"));
        let desktop = behaving_fo4edit(desktop, &files, &exit, |action| {
            matches!(action, DesktopAction::PostEnter { .. })
        });
        let wait = RecordingWait::new();

        merge_combined_objects(&paths(&files), &process, &wait, &desktop, &files).unwrap();

        assert_eq!(
            dismissal_actions(&desktop),
            vec![
                DesktopAction::ClickButton {
                    window: MODULE_SELECTION,
                    caption: "OK".to_owned(),
                },
                DesktopAction::PostEnter {
                    window: MODULE_SELECTION,
                },
            ]
        );
    }

    /// Only this run's visible `Module Selection` is ever acted on. A developer message of the
    /// same process, a hidden window captioned `Module Selection`, and another xEdit's Module
    /// Selection get no click and no ENTER. Close requests reach this process's windows, hidden
    /// ones included, and never the other process.
    #[test]
    fn no_window_but_this_runs_module_selection_is_dismissed() {
        let message = WindowHandle::from_raw(0x300, FO4EDIT);
        let hidden_module_selection = WindowHandle::from_raw(0x400, FO4EDIT);
        let other_module_selection = WindowHandle::from_raw(0x500, OTHER_PROCESS);
        let files = Rc::new(InMemoryFileSpace::new());
        let (process, exit) = fo4edit_exiting_on_close();
        let wait = log_after_poll_waits(&files, 3, COMPLETED_LOG);
        let desktop = behaving_fo4edit(
            without_module_selection()
                .with_window(ScriptedWindow::new(message, "Message").with_button("OK"))
                .with_window(
                    ScriptedWindow::new(hidden_module_selection, "Module Selection")
                        .with_button("OK")
                        .hidden(),
                )
                .with_window(
                    ScriptedWindow::new(other_module_selection, "Module Selection")
                        .with_button("OK"),
                ),
            &files,
            &exit,
            |_| false,
        );

        merge_combined_objects(&paths(&*files), &process, &wait, &desktop, files.as_ref()).unwrap();

        assert_eq!(dismissal_actions(&desktop), vec![]);
        assert_eq!(
            close_requests(&desktop),
            vec![MAIN_FORM, message, hidden_module_selection]
        );
    }

    /// No Module Selection by 30 seconds after launch: one hint, on the poll after the fifth
    /// poll wait. A long merge also gets a "still waiting" line each minute.
    #[test]
    fn a_missing_module_selection_gets_one_hint_and_a_long_wait_says_so() {
        let files = Rc::new(InMemoryFileSpace::new());
        let (process, exit) = fo4edit_exiting_on_close();
        let wait = log_after_poll_waits(&files, 25, COMPLETED_LOG);
        let desktop = behaving_fo4edit(without_module_selection(), &files, &exit, |_| false);

        let (result, console) = capture_console(|| {
            merge_combined_objects(&paths(&*files), &process, &wait, &desktop, files.as_ref())
        });

        result.unwrap();
        assert_eq!(console.matches(HINT).count(), 1, "console: {console}");
        let still_waiting =
            "Still waiting for FO4Edit to finish Batch_FO4MergeCombinedObjectsAndCheck.pas";
        assert_eq!(
            console.matches(still_waiting).count(),
            2,
            "console: {console}"
        );
        assert!(
            console.contains(&format!("{still_waiting} (1 min)")),
            "console: {console}"
        );
        assert!(
            console.contains(&format!("{still_waiting} (2 min)")),
            "console: {console}"
        );
        // Nothing to dismiss, and nothing to ask the operator to press.
        assert_eq!(dismissal_actions(&desktop), vec![]);
        assert!(!console.contains(PRESS_OK), "console: {console}");
        // 25 poll waits between the launch delays and the close sequence.
        let delays = wait.delays();
        assert_eq!(delays.len(), 2 + 25 + 3);
        assert!(delays[2..27].iter().all(|delay| *delay == 5));
    }

    /// The hint is due on the poll after the fifth poll wait, and not before: a log that
    /// appears after the fifth wait ends the poll on that same iteration, so no hint. (A Module
    /// Selection seen before 30 seconds suppresses it too; see
    /// `module_selection_that_survives_both_rungs_is_left_to_the_operator`.)
    #[test]
    fn the_hint_is_not_due_until_the_poll_after_the_fifth_wait() {
        let files = Rc::new(InMemoryFileSpace::new());
        let (process, exit) = fo4edit_exiting_on_close();
        let wait = log_after_poll_waits(&files, 5, COMPLETED_LOG);
        let desktop = behaving_fo4edit(without_module_selection(), &files, &exit, |_| false);

        let (result, console) = capture_console(|| {
            merge_combined_objects(&paths(&*files), &process, &wait, &desktop, files.as_ref())
        });

        result.unwrap();
        assert!(!console.contains(HINT), "console: {console}");
    }

    /// FO4Edit exits before its script wrote the log: the run stops instead of polling forever,
    /// with no close request and no log folded into the session log.
    #[test]
    fn fo4edit_exiting_before_the_log_stops_the_run() {
        let files = InMemoryFileSpace::new();
        let process =
            RecordingProcessRunner::new().spawning(ScriptedExit::AfterPolls { polls: 2, code: 3 });
        let desktop = without_module_selection();
        let wait = RecordingWait::new();

        let error =
            merge_combined_objects(&paths(&files), &process, &wait, &desktop, &files).unwrap_err();

        assert!(
            matches!(
                error,
                Error::Fo4EditExitedEarly {
                    script: MERGE_COMBINED_OBJECTS_SCRIPT,
                    code: Some(3)
                }
            ),
            "error: {error:?}"
        );
        assert_eq!(desktop.actions(), vec![]);
        // Two polls found it running; the third found it gone, before any close delay.
        assert_eq!(wait.delays(), vec![10, 5, 5, 5]);
        let session = files.read_lossy(&session_log(&files)).unwrap();
        assert!(
            session.ends_with("====================================\n"),
            "session log: {session}"
        );
    }

    /// FO4Edit closes on the first request: one round, to every one of its top-level windows,
    /// then the remaining delays.
    #[test]
    fn fo4edit_closing_on_the_first_request_gets_one_round() {
        let helper = WindowHandle::from_raw(0x300, FO4EDIT);
        let files = InMemoryFileSpace::new();
        let (process, exit) = fo4edit_exiting_on_close();
        let desktop = behaving_fo4edit(
            at_module_selection().with_window(ScriptedWindow::new(helper, "").hidden()),
            &files,
            &exit,
            is_click_on_module_selection,
        );
        let wait = RecordingWait::new();

        merge_combined_objects(&paths(&files), &process, &wait, &desktop, &files).unwrap();

        assert_eq!(close_requests(&desktop), vec![MAIN_FORM, helper]);
        assert_eq!(wait.delays(), ONE_POLL_DELAYS);
    }

    /// FO4Edit ignores the first request and closes on the second: two rounds.
    #[test]
    fn fo4edit_closing_on_the_second_request_gets_two_rounds() {
        let files = InMemoryFileSpace::new();
        let exit = ExitFlag::new();
        let process = RecordingProcessRunner::new().spawning(ScriptedExit::WhenFlagged {
            flag: exit.clone(),
            code: 0,
        });
        let close_requests_seen = Cell::new(0);
        let desktop = {
            let log = log(&files);
            let files = &files;
            at_module_selection().on_action(move |action, script| {
                if is_click_on_module_selection(action) {
                    script.remove_window(MODULE_SELECTION);
                    files.add_file_with_contents(&log, COMPLETED_LOG);
                }
                if matches!(action, DesktopAction::RequestClose { .. }) {
                    close_requests_seen.set(close_requests_seen.get() + 1);
                    if close_requests_seen.get() == 2 {
                        exit.set();
                    }
                }
            })
        };
        let wait = RecordingWait::new();

        merge_combined_objects(&paths(&files), &process, &wait, &desktop, &files).unwrap();

        assert_eq!(close_requests(&desktop), vec![MAIN_FORM, MAIN_FORM]);
        assert_eq!(wait.delays(), ONE_POLL_DELAYS);
    }

    /// FO4Edit ignores both requests: the run stops naming its PID, after every delay, and
    /// FO4Edit is left running rather than killed (the double has no way to kill it).
    #[test]
    fn fo4edit_ignoring_both_requests_stops_the_run_naming_its_pid() {
        let files = InMemoryFileSpace::new();
        let process = RecordingProcessRunner::new().spawning(ScriptedExit::Never);
        let desktop = behaving_fo4edit(
            at_module_selection(),
            &files,
            &ExitFlag::new(),
            is_click_on_module_selection,
        );
        let wait = RecordingWait::new();

        let error =
            merge_combined_objects(&paths(&files), &process, &wait, &desktop, &files).unwrap_err();

        assert!(
            matches!(error, Error::Fo4EditStillRunning { pid: FO4EDIT }),
            "error: {error:?}"
        );
        assert_eq!(close_requests(&desktop), vec![MAIN_FORM, MAIN_FORM]);
        assert_eq!(wait.delays(), ONE_POLL_DELAYS);
    }

    /// A FO4Edit that has already exited by the close sequence (a future xEdit honouring
    /// `-autoexit`) gets no close request, but every delay still runs: they are MO2's.
    #[test]
    fn an_already_exited_fo4edit_gets_no_request_but_every_delay() {
        let files = InMemoryFileSpace::new();
        // The log appears as FO4Edit starts, and FO4Edit is gone by the first look at it.
        let process = RecordingProcessRunner::new()
            .spawning(ScriptedExit::AfterPolls { polls: 0, code: 0 })
            .with_effects(&files, |space| {
                space.add_file_with_contents(log(space), COMPLETED_LOG);
            });
        let desktop = at_module_selection();
        let wait = RecordingWait::new();

        merge_combined_objects(&paths(&files), &process, &wait, &desktop, &files).unwrap();

        assert_eq!(desktop.actions(), vec![]);
        assert_eq!(wait.delays(), vec![10, 5, 10, 15, 10]);
    }

    /// Run the Step 2 merge with FO4Edit leaving `contents` as its log, and return the result
    /// and the session log.
    fn merge_with_log(contents: &str) -> (Result<String>, String) {
        let files = InMemoryFileSpace::new();
        let (process, exit) = fo4edit_exiting_on_close();
        let desktop = fo4edit_logging(
            at_module_selection(),
            &files,
            &exit,
            is_click_on_module_selection,
            contents,
        );
        let wait = RecordingWait::new();

        let result = merge_combined_objects(&paths(&files), &process, &wait, &desktop, &files);
        (result, files.read_lossy(&session_log(&files)).unwrap())
    }

    /// Batch 560–561, matched case-insensitively and spelled out here by hand, so a narrowed
    /// marker in the episode cannot pass by building the test from the same string.
    #[test]
    fn missing_modules_in_the_log_stops_the_run() {
        let log = "[00:01] error: MISSING [mymod.ESP] or [combinedobjects.esp] Modules\n\
                   Completed: 1 errors.\n";

        let (result, session) = merge_with_log(log);

        let error = result.unwrap_err();
        assert!(
            matches!(
                error,
                Error::Fo4EditScriptMissingModules {
                    script: MERGE_COMBINED_OBJECTS_SCRIPT
                }
            ),
            "error: {error:?}"
        );
        // Folded first, as the batch `type`s the log before scanning it.
        assert!(session.ends_with(log), "session log: {session}");
    }

    /// Batch 562–563: no `Completed: ` anywhere in the log.
    #[test]
    fn a_log_without_completed_stops_the_run() {
        let (result, session) = merge_with_log("Merging CombinedObjects.esp\nAborted\n");

        let error = result.unwrap_err();
        assert!(
            matches!(
                error,
                Error::Fo4EditScriptFailed {
                    script: MERGE_COMBINED_OBJECTS_SCRIPT
                }
            ),
            "error: {error:?}"
        );
        assert!(session.ends_with("Aborted\n"), "session log: {session}");
    }

    /// `Completed: ` in another case still counts, and a missing-modules line naming other
    /// plugins is not this run's fatal.
    #[test]
    fn the_fatals_match_only_this_runs_markers_in_any_case() {
        let log =
            "Error: Missing [Other.esp] or [CombinedObjects.esp] modules\nCOMPLETED: 1 errors.\n";

        let (result, _) = merge_with_log(log);

        assert_eq!(result.unwrap(), log);
    }

    /// A log that was there when the poll ended and is gone when the episode reads it.
    #[test]
    fn a_log_that_vanishes_before_it_is_read_is_an_error() {
        let files = InMemoryFileSpace::new();
        let (process, exit) = fo4edit_exiting_on_close();
        let desktop = {
            let files = &files;
            behaving_fo4edit(
                at_module_selection(),
                files,
                &exit,
                is_click_on_module_selection,
            )
            .on_action(move |action, _| {
                if matches!(action, DesktopAction::RequestClose { .. }) {
                    files.remove_file(&log(files)).unwrap();
                }
            })
        };
        let wait = RecordingWait::new();

        let error =
            merge_combined_objects(&paths(&files), &process, &wait, &desktop, &files).unwrap_err();

        assert!(
            matches!(&error, Error::UnattendedLogMissing { path } if *path == log(&files)),
            "error: {error:?}"
        );
    }
}
