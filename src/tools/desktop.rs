//! The `DesktopWindows` seam: reading and acting on the desktop's top-level windows.
//!
//! An internal seam, private to the tools layer like `ProcessRunner` and `Wait`, and absent from
//! what a Workflow Operation is handed. The FO4Edit episode needs it to dismiss this run's Module
//! Selection dialog and to ask this run's FO4Edit to close — the work the batch did with
//! PowerShell `AppActivate`/`SendKeys` and a machine-wide close. Which window to target, the
//! dismissal ladder and the close sequence belong to that episode, not here: this seam only
//! reports windows and carries out single actions, so the rule "no input ever goes to any other
//! window" can be asserted against [`RecordingDesktopWindows`] rather than hoped for.
//!
//! The production adapter reaches the Win32 calls through the `generateprevisibines-win32-windows`
//! helper crate, because every one of them is an `unsafe fn` and this crate forbids `unsafe`
//! (ADR-0003). Off Windows there is no helper: the adapter reports no windows and its actions do
//! nothing, so the dismissal ladder never fires and the "press OK" instruction covers the user.
//! The tool targets Windows and Wine, so off Windows is a build-and-test platform only.

#[cfg(windows)]
use generateprevisibines_win32_windows as win32;

use crate::error::Result;

/// An opaque handle to one window, valid for as long as that window exists.
///
/// Only a `DesktopWindows` implementation hands these out; callers compare them and pass them
/// back, and never read what is inside.
///
/// It holds the window's handle number and the id of the process that owned the window when it
/// was read. Windows recycles handle numbers, so the number alone could come to name another
/// program's window; with the process id, the production adapter refuses to act on it instead,
/// and two handles are equal only if both match.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct WindowHandle {
    raw: usize,
    pid: u32,
}

impl WindowHandle {
    /// Wrap a raw window-handle value and the id of the process that owns its window.
    ///
    /// For the production adapter, which round-trips the helper crate's handle, and for tests,
    /// which make up handles for their scripted windows.
    #[cfg(any(windows, test))]
    pub(crate) const fn from_raw(raw: usize, pid: u32) -> Self {
        Self { raw, pid }
    }
}

/// One top-level window, as it stood when [`DesktopWindows::top_level_windows`] read it.
///
/// The helper crate also reports each window's owner. It is left out here because the FO4Edit
/// episode picks Module Selection by its exact caption and closes every window of the process,
/// so nothing in this crate decides anything by ownership.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WindowSnapshot {
    pub(crate) handle: WindowHandle,
    /// The window's title text; empty when it has none.
    pub(crate) caption: String,
    pub(crate) class_name: String,
    pub(crate) visible: bool,
    pub(crate) enabled: bool,
}

/// Read the desktop's top-level windows and act on one window at a time.
///
/// Every action names its target and is delivered to that window alone. There is deliberately
/// no way to type a key: typed input goes to whichever window has the foreground when it is
/// processed, which no check made beforehand can pin down, so ENTER is
/// [posted](Self::post_enter) instead.
///
/// Implementors must be `Debug` so the adapters that hold a `DesktopWindows` can keep deriving
/// `Debug`.
pub(crate) trait DesktopWindows: std::fmt::Debug {
    /// Every top-level window belonging to process `pid`, hidden ones included, in Z order.
    ///
    /// Empty when the process has no windows or does not exist. Never an error: a listing that
    /// fails reads as "no windows", which leaves the caller polling rather than acting.
    fn top_level_windows(&self, pid: u32) -> Vec<WindowSnapshot>;

    /// Click the button captioned `caption` inside `window`, by posting it `BM_CLICK`.
    ///
    /// Returns `Ok(false)` when `window` has no such button. `Ok(true)` means only that the
    /// click was posted — whether the window acted on it is for a later
    /// [`top_level_windows`](Self::top_level_windows) to show. An error means `window` no longer
    /// exists or the post failed.
    fn click_button(&self, window: WindowHandle, caption: &str) -> Result<bool>;

    /// Post one ENTER key press to `window`: to its focused control, as a typed key would reach,
    /// or to `window` itself when focus is not inside it. Works whether or not `window` has the
    /// foreground, and never reaches any other window.
    ///
    /// Returns once the key is posted; whether the window acted on it is for a later
    /// [`top_level_windows`](Self::top_level_windows) to show. An error means `window` no longer
    /// exists or the post failed.
    fn post_enter(&self, window: WindowHandle) -> Result<()>;

    /// Ask `window` to close, by posting it `WM_CLOSE`. Returns once the request is posted, not
    /// once the window has closed; it may refuse. An error means `window` no longer exists or
    /// the post failed.
    fn request_close(&self, window: WindowHandle) -> Result<()>;
}

/// The production `DesktopWindows`: Win32 calls through the helper crate on Windows, and inert
/// everywhere else.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct SystemDesktopWindows;

/// The VCL class of a Delphi push button, which is what FO4Edit's dialog buttons are.
///
/// Held here rather than in the helper crate, which takes no position on what to look for.
#[cfg(windows)]
const BUTTON_CLASS: &str = "TButton";

#[cfg(windows)]
impl DesktopWindows for SystemDesktopWindows {
    fn top_level_windows(&self, pid: u32) -> Vec<WindowSnapshot> {
        match win32::top_level_windows(pid) {
            Ok(windows) => windows
                .into_iter()
                .map(|window| WindowSnapshot {
                    handle: from_helper(window.handle),
                    caption: window.caption,
                    class_name: window.class_name,
                    visible: window.visible,
                    enabled: window.enabled,
                })
                .collect(),
            Err(error) => {
                // Reads as "no windows yet": the caller keeps polling, and its own one-time
                // hint and "press OK" instruction still reach the user. Logged at debug because
                // a caller polls every few seconds and the user can do nothing about it.
                tracing::debug!(pid, %error, "listing top-level windows failed");
                Vec::new()
            }
        }
    }

    fn click_button(&self, window: WindowHandle, caption: &str) -> Result<bool> {
        let Some(button) = win32::find_child(to_helper(window), BUTTON_CLASS, caption)? else {
            return Ok(false);
        };
        win32::post_button_click(button)?;
        Ok(true)
    }

    fn post_enter(&self, window: WindowHandle) -> Result<()> {
        Ok(win32::post_enter(to_helper(window))?)
    }

    fn request_close(&self, window: WindowHandle) -> Result<()> {
        Ok(win32::post_close(to_helper(window))?)
    }
}

/// Off Windows there are no windows to see and nothing to act on (see the module docs).
#[cfg(not(windows))]
impl DesktopWindows for SystemDesktopWindows {
    fn top_level_windows(&self, _pid: u32) -> Vec<WindowSnapshot> {
        Vec::new()
    }

    fn click_button(&self, _window: WindowHandle, _caption: &str) -> Result<bool> {
        Ok(false)
    }

    fn post_enter(&self, _window: WindowHandle) -> Result<()> {
        Ok(())
    }

    fn request_close(&self, _window: WindowHandle) -> Result<()> {
        Ok(())
    }
}

/// The helper crate's handle for `window`.
#[cfg(windows)]
fn to_helper(window: WindowHandle) -> win32::Hwnd {
    win32::Hwnd::from_raw(window.raw, window.pid)
}

/// This seam's handle for the helper crate's `handle`.
#[cfg(windows)]
fn from_helper(handle: win32::Hwnd) -> WindowHandle {
    WindowHandle::from_raw(handle.raw(), handle.pid())
}

#[cfg(test)]
pub(crate) use recording::{DesktopAction, RecordingDesktopWindows, ScriptedWindow};

#[cfg(test)]
mod recording {
    use std::cell::RefCell;
    use std::fmt;
    use std::io;

    use super::{DesktopWindows, WindowHandle, WindowSnapshot};
    use crate::error::Result;

    /// One action a caller took through [`RecordingDesktopWindows`], with its target.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub(crate) enum DesktopAction {
        /// [`DesktopWindows::click_button`].
        ClickButton {
            window: WindowHandle,
            caption: String,
        },
        /// [`DesktopWindows::post_enter`].
        PostEnter { window: WindowHandle },
        /// [`DesktopWindows::request_close`].
        RequestClose { window: WindowHandle },
    }

    /// One scripted top-level window: what [`DesktopWindows::top_level_windows`] reports for it
    /// and which buttons a click can find inside it. The process that owns it is the one its
    /// handle names.
    ///
    /// Starts visible and enabled, with an empty class name and no buttons.
    #[derive(Debug, Clone)]
    pub(crate) struct ScriptedWindow {
        snapshot: WindowSnapshot,
        buttons: Vec<String>,
    }

    impl ScriptedWindow {
        /// A visible, enabled window with handle `handle`, captioned `caption`.
        #[must_use]
        pub(crate) fn new(handle: WindowHandle, caption: &str) -> Self {
            Self {
                snapshot: WindowSnapshot {
                    handle,
                    caption: caption.to_owned(),
                    class_name: String::new(),
                    visible: true,
                    enabled: true,
                },
                buttons: Vec::new(),
            }
        }

        /// Report `class_name` as the window's class.
        #[must_use]
        pub(crate) fn with_class(mut self, class_name: &str) -> Self {
            self.snapshot.class_name = class_name.to_owned();
            self
        }

        /// Report the window as hidden.
        #[must_use]
        pub(crate) fn hidden(mut self) -> Self {
            self.snapshot.visible = false;
            self
        }

        /// Report the window as disabled.
        #[must_use]
        pub(crate) fn disabled(mut self) -> Self {
            self.snapshot.enabled = false;
            self
        }

        /// Give the window a button captioned `caption`, which
        /// [`DesktopWindows::click_button`] can then find. A window without it answers a click
        /// with "no such button".
        #[must_use]
        pub(crate) fn with_button(mut self, caption: &str) -> Self {
            self.buttons.push(caption.to_owned());
            self
        }
    }

    /// The desktop as a [`RecordingDesktopWindows`] currently reports it.
    ///
    /// Effects receive it mutably, which is how the script changes in response to actions:
    /// Module Selection disappears once its `OK` is clicked or it is posted ENTER, the process's
    /// windows go away once it is asked to close, another dialog appears.
    #[derive(Debug, Default)]
    pub(crate) struct DesktopScript {
        windows: Vec<ScriptedWindow>,
    }

    impl DesktopScript {
        /// Add `window` above (after) every window already scripted.
        pub(crate) fn add_window(&mut self, window: ScriptedWindow) {
            self.windows.push(window);
        }

        /// Destroy the window `handle`.
        pub(crate) fn remove_window(&mut self, handle: WindowHandle) {
            self.windows
                .retain(|window| window.snapshot.handle != handle);
        }

        /// Show or hide the window `handle`. Does nothing if no such window is scripted.
        pub(crate) fn set_visible(&mut self, handle: WindowHandle, visible: bool) {
            if let Some(window) = self.window_mut(handle) {
                window.snapshot.visible = visible;
            }
        }

        fn window(&self, handle: WindowHandle) -> Option<&ScriptedWindow> {
            self.windows
                .iter()
                .find(|window| window.snapshot.handle == handle)
        }

        fn window_mut(&mut self, handle: WindowHandle) -> Option<&mut ScriptedWindow> {
            self.windows
                .iter_mut()
                .find(|window| window.snapshot.handle == handle)
        }
    }

    /// An effect run after every recorded action. See [`RecordingDesktopWindows::on_action`].
    type Effect<'a> = Box<dyn Fn(&DesktopAction, &mut DesktopScript) + 'a>;

    /// A test [`DesktopWindows`] that answers from a script, records every action in order, and
    /// touches no real window.
    ///
    /// - [`top_level_windows`] reports the scripted windows of the requested process, in the
    ///   order they were scripted.
    /// - [`click_button`] answers `true` when the target window was scripted
    ///   [`with_button`](ScriptedWindow::with_button) for that caption, and `false` otherwise.
    /// - [`post_enter`] and [`request_close`] succeed for any scripted window and change nothing
    ///   themselves; an effect decides what the key or the close request does.
    /// - An action aimed at a window that is not scripted (never was, or has been removed) fails
    ///   with an error, the way Win32 does for a destroyed window. Handles match on process as
    ///   well as number, so a handle whose number now belongs to another process's scripted
    ///   window counts as not scripted, as the production adapter treats a recycled one.
    ///
    /// Every action is recorded, failed ones included, and every [`on_action`](Self::on_action)
    /// effect runs on it in installation order. The action's own answer comes from the script as
    /// it stood when the action arrived, before its effects.
    ///
    /// Interior mutability, like the other recording adapters: the caller under test holds the
    /// double by shared reference across the whole episode.
    ///
    /// [`top_level_windows`]: DesktopWindows::top_level_windows
    /// [`click_button`]: DesktopWindows::click_button
    /// [`post_enter`]: DesktopWindows::post_enter
    /// [`request_close`]: DesktopWindows::request_close
    #[derive(Default)]
    pub(crate) struct RecordingDesktopWindows<'a> {
        script: RefCell<DesktopScript>,
        actions: RefCell<Vec<DesktopAction>>,
        effects: Vec<Effect<'a>>,
    }

    impl<'a> RecordingDesktopWindows<'a> {
        /// A desktop with no windows.
        #[must_use]
        pub(crate) fn new() -> Self {
            Self::default()
        }

        /// Script `window`, above (after) every window already scripted.
        #[must_use]
        pub(crate) fn with_window(self, window: ScriptedWindow) -> Self {
            self.script.borrow_mut().add_window(window);
            self
        }

        /// Run `effect` after every action, with the action and the script.
        ///
        /// The effect changes what later calls see by mutating the script, and reaches the rest
        /// of the test's world through what it captures: it can write the unattended log into an
        /// `InMemoryFileSpace` when Module Selection is dismissed, or set a recording process's
        /// `ExitFlag` when FO4Edit is asked to close. It decides for itself which actions it
        /// cares about. Installing several runs them in installation order.
        ///
        /// The effect runs while the script is borrowed, so it must not reach this double itself;
        /// it cannot capture it anyway, since the double does not exist until `on_action` returns.
        #[must_use]
        pub(crate) fn on_action(
            mut self,
            effect: impl Fn(&DesktopAction, &mut DesktopScript) + 'a,
        ) -> Self {
            self.effects.push(Box::new(effect));
            self
        }

        /// The actions recorded so far, in call order.
        #[must_use]
        pub(crate) fn actions(&self) -> Vec<DesktopAction> {
            self.actions.borrow().clone()
        }

        /// Run every effect on `action`, then record it.
        ///
        /// The order is invisible to the effects, which cannot see the action list; recording
        /// last just lets the list take ownership of the action.
        fn record(&self, action: DesktopAction) {
            {
                let mut script = self.script.borrow_mut();
                for effect in &self.effects {
                    effect(&action, &mut script);
                }
            }
            self.actions.borrow_mut().push(action);
        }
    }

    impl DesktopWindows for RecordingDesktopWindows<'_> {
        fn top_level_windows(&self, pid: u32) -> Vec<WindowSnapshot> {
            self.script
                .borrow()
                .windows
                .iter()
                .filter(|window| window.snapshot.handle.pid == pid)
                .map(|window| window.snapshot.clone())
                .collect()
        }

        fn click_button(&self, window: WindowHandle, caption: &str) -> Result<bool> {
            // Answered from the script as it stood when the click arrived, before any effect of
            // the click itself changes it.
            let found = self
                .script
                .borrow()
                .window(window)
                .map(|scripted| scripted.buttons.iter().any(|button| button == caption));
            self.record(DesktopAction::ClickButton {
                window,
                caption: caption.to_owned(),
            });
            found.ok_or_else(|| no_such_window(window))
        }

        fn post_enter(&self, window: WindowHandle) -> Result<()> {
            let exists = self.script.borrow().window(window).is_some();
            self.record(DesktopAction::PostEnter { window });
            if exists {
                Ok(())
            } else {
                Err(no_such_window(window))
            }
        }

        fn request_close(&self, window: WindowHandle) -> Result<()> {
            let exists = self.script.borrow().window(window).is_some();
            self.record(DesktopAction::RequestClose { window });
            if exists {
                Ok(())
            } else {
                Err(no_such_window(window))
            }
        }
    }

    impl fmt::Debug for RecordingDesktopWindows<'_> {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            // The effects are not `Debug`; how many are installed is what a failing assertion
            // needs to see.
            f.debug_struct("RecordingDesktopWindows")
                .field("script", &self.script)
                .field("actions", &self.actions)
                .field("effects", &self.effects.len())
                .finish()
        }
    }

    /// The error for an action aimed at a window that is not scripted, standing in for Win32's
    /// `ERROR_INVALID_WINDOW_HANDLE`.
    fn no_such_window(window: WindowHandle) -> crate::error::Error {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!("no scripted window {window:?}"),
        )
        .into()
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{
        DesktopAction, DesktopWindows, RecordingDesktopWindows, ScriptedWindow,
        SystemDesktopWindows, WindowHandle, WindowSnapshot,
    };
    use crate::files::{FileSpace, InMemoryFileSpace};
    use crate::tools::process::{
        ExitFlag, ProcessRunner, RECORDED_PROCESS_ID, RecordingProcessRunner, ScriptedExit,
    };

    const FO4EDIT: u32 = RECORDED_PROCESS_ID;
    const OTHER_PROCESS: u32 = 77;

    const MAIN_FORM: WindowHandle = WindowHandle::from_raw(0x100, FO4EDIT);
    const MODULE_SELECTION: WindowHandle = WindowHandle::from_raw(0x200, FO4EDIT);
    const HELPER: WindowHandle = WindowHandle::from_raw(0x300, FO4EDIT);
    const OTHER_WINDOW: WindowHandle = WindowHandle::from_raw(0x400, OTHER_PROCESS);

    /// FO4Edit's windows while Module Selection is modal, as the probe (issue #40) recorded them:
    /// a disabled main form, the dialog with its `OK` button, and a hidden helper.
    fn fo4edit_during_module_selection<'a>() -> RecordingDesktopWindows<'a> {
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
            .with_window(ScriptedWindow::new(HELPER, "").hidden())
            .with_window(ScriptedWindow::new(OTHER_WINDOW, "xEdit"))
    }

    /// The system adapter must answer, not fail or panic, for a process that does not exist —
    /// a FO4Edit that has already gone, from the episode's point of view.
    #[test]
    fn system_desktop_reports_no_windows_for_a_pid_that_does_not_exist() {
        // Windows process ids are multiples of 4, so an odd id names no process.
        let no_such_process = u32::MAX;

        assert_eq!(
            SystemDesktopWindows.top_level_windows(no_such_process),
            vec![]
        );
    }

    #[test]
    fn recording_desktop_lists_only_the_requested_process_windows_in_script_order() {
        let desktop = fo4edit_during_module_selection();

        let snapshot = |handle, caption: &str, class_name: &str, visible, enabled| WindowSnapshot {
            handle,
            caption: caption.to_owned(),
            class_name: class_name.to_owned(),
            visible,
            enabled,
        };
        assert_eq!(
            desktop.top_level_windows(FO4EDIT),
            vec![
                snapshot(MAIN_FORM, "FO4Script 4.1.5q x64", "TfrmMain", true, false),
                snapshot(
                    MODULE_SELECTION,
                    "Module Selection",
                    "TfrmModuleSelect",
                    true,
                    true
                ),
                snapshot(HELPER, "", "", false, true),
            ]
        );
        assert_eq!(desktop.top_level_windows(12345), vec![]);
        // Reading the desktop is not an action.
        assert_eq!(desktop.actions(), vec![]);
    }

    #[test]
    fn recording_desktop_finds_only_scripted_buttons() {
        let desktop = fo4edit_during_module_selection();

        assert!(desktop.click_button(MODULE_SELECTION, "OK").unwrap());
        assert!(!desktop.click_button(MODULE_SELECTION, "Cancel").unwrap());
        assert!(!desktop.click_button(MAIN_FORM, "OK").unwrap());
    }

    #[test]
    fn recording_desktop_fails_an_action_on_a_window_that_does_not_exist() {
        let gone = WindowHandle::from_raw(0xdead, FO4EDIT);
        let desktop = fo4edit_during_module_selection();

        assert!(desktop.click_button(gone, "OK").is_err());
        assert!(desktop.request_close(gone).is_err());
        assert!(desktop.post_enter(gone).is_err());

        // Failed actions are still recorded: the caller did attempt them.
        assert_eq!(
            desktop.actions(),
            vec![
                DesktopAction::ClickButton {
                    window: gone,
                    caption: "OK".to_owned(),
                },
                DesktopAction::RequestClose { window: gone },
                DesktopAction::PostEnter { window: gone },
            ]
        );
    }

    /// Windows recycles handle numbers: once Module Selection is gone, its number can name
    /// another program's window. A handle saved from FO4Edit must then fail like a destroyed
    /// window rather than reach that program.
    #[test]
    fn a_handle_whose_number_was_recycled_by_another_process_is_not_acted_on() {
        let recycled = WindowHandle::from_raw(MODULE_SELECTION.raw, OTHER_PROCESS);
        let desktop = fo4edit_during_module_selection().on_action(move |action, script| {
            if matches!(action, DesktopAction::ClickButton { .. }) {
                script.remove_window(MODULE_SELECTION);
                script.add_window(ScriptedWindow::new(recycled, "Save As").with_button("OK"));
            }
        });
        assert!(desktop.click_button(MODULE_SELECTION, "OK").unwrap());

        assert_ne!(recycled, MODULE_SELECTION);
        assert!(desktop.click_button(MODULE_SELECTION, "OK").is_err());
        assert!(desktop.post_enter(MODULE_SELECTION).is_err());
        assert!(desktop.request_close(MODULE_SELECTION).is_err());
    }

    #[test]
    fn recording_desktop_records_every_action_with_its_target_in_order() {
        let desktop = fo4edit_during_module_selection();

        desktop.click_button(MODULE_SELECTION, "OK").unwrap();
        desktop.post_enter(MODULE_SELECTION).unwrap();
        desktop.request_close(MAIN_FORM).unwrap();
        desktop.request_close(HELPER).unwrap();

        assert_eq!(
            desktop.actions(),
            vec![
                DesktopAction::ClickButton {
                    window: MODULE_SELECTION,
                    caption: "OK".to_owned(),
                },
                DesktopAction::PostEnter {
                    window: MODULE_SELECTION,
                },
                DesktopAction::RequestClose { window: MAIN_FORM },
                DesktopAction::RequestClose { window: HELPER },
            ]
        );
    }

    #[test]
    fn an_effect_can_dismiss_module_selection_when_it_is_posted_enter() {
        let desktop = fo4edit_during_module_selection().on_action(|action, script| {
            if *action
                == (DesktopAction::PostEnter {
                    window: MODULE_SELECTION,
                })
            {
                script.remove_window(MODULE_SELECTION);
            }
        });

        // ENTER posted elsewhere in FO4Edit dismisses nothing.
        desktop.post_enter(MAIN_FORM).unwrap();
        assert!(module_selection_is_listed(&desktop));

        desktop.post_enter(MODULE_SELECTION).unwrap();
        assert!(!module_selection_is_listed(&desktop));
    }

    #[test]
    fn an_effect_can_dismiss_module_selection_when_its_ok_is_clicked() {
        let desktop = fo4edit_during_module_selection().on_action(|action, script| {
            if let DesktopAction::ClickButton { window, caption } = action
                && *window == MODULE_SELECTION
                && caption == "OK"
            {
                script.remove_window(MODULE_SELECTION);
            }
        });

        // A click that finds nothing dismisses nothing.
        assert!(!desktop.click_button(MODULE_SELECTION, "Cancel").unwrap());
        assert!(module_selection_is_listed(&desktop));

        // The click is answered from the desktop it arrived at, then the effect changes it.
        assert!(desktop.click_button(MODULE_SELECTION, "OK").unwrap());
        assert!(!module_selection_is_listed(&desktop));
    }

    #[test]
    fn without_an_effect_module_selection_stays_after_a_click() {
        // Models a disabled OK: the click is posted, and the dialog stays.
        let desktop = fo4edit_during_module_selection();

        assert!(desktop.click_button(MODULE_SELECTION, "OK").unwrap());

        assert!(module_selection_is_listed(&desktop));
    }

    #[test]
    fn an_effect_can_hide_a_window() {
        let desktop = fo4edit_during_module_selection().on_action(|action, script| {
            if let DesktopAction::PostEnter { window } = action {
                script.set_visible(*window, false);
            }
        });

        desktop.post_enter(MODULE_SELECTION).unwrap();

        let module_selection = desktop
            .top_level_windows(FO4EDIT)
            .into_iter()
            .find(|window| window.handle == MODULE_SELECTION)
            .unwrap();
        assert!(!module_selection.visible);
    }

    #[test]
    fn an_effect_can_add_a_window() {
        let dialog = WindowHandle::from_raw(0x500, FO4EDIT);
        let desktop = fo4edit_during_module_selection().on_action(move |action, script| {
            if matches!(action, DesktopAction::ClickButton { .. }) {
                script.add_window(ScriptedWindow::new(dialog, "Message"));
            }
        });

        desktop.click_button(MODULE_SELECTION, "OK").unwrap();

        let last = desktop.top_level_windows(FO4EDIT).pop().unwrap();
        assert_eq!((last.handle, last.caption.as_str()), (dialog, "Message"));
    }

    /// The two cross-seam effects the FO4Edit episode tests rely on: dismissing Module Selection
    /// writes the unattended log into the file space, and a close request makes the recording
    /// process exit.
    #[test]
    fn effects_can_write_a_file_and_make_a_recorded_process_exit() {
        let space = InMemoryFileSpace::new();
        let log = PathBuf::from(r"C:\Users\me\AppData\Local\Temp\UnattendedScript.log");
        let exit = ExitFlag::new();
        let runner = RecordingProcessRunner::new().spawning(ScriptedExit::WhenFlagged {
            flag: exit.clone(),
            code: 0,
        });
        let desktop = {
            let space = &space;
            let log = log.clone();
            let exit = exit.clone();
            fo4edit_during_module_selection()
                .on_action(move |action, script| {
                    if let DesktopAction::ClickButton { window, .. } = action {
                        script.remove_window(*window);
                        space.add_file_with_contents(&log, "Completed: No Errors.\n");
                    }
                })
                .on_action(move |action, _| {
                    if matches!(action, DesktopAction::RequestClose { .. }) {
                        exit.set();
                    }
                })
        };
        let mut fo4edit = runner
            .spawn(
                std::path::Path::new("FO4Edit.exe"),
                &[],
                std::path::Path::new("Temp"),
            )
            .unwrap();

        assert!(!space.is_file(&log));
        desktop.click_button(MODULE_SELECTION, "OK").unwrap();
        assert!(space.is_file(&log));

        assert!(fo4edit.try_wait().unwrap().is_none());
        desktop.request_close(MAIN_FORM).unwrap();
        assert!(fo4edit.try_wait().unwrap().unwrap().success());
    }

    fn module_selection_is_listed(desktop: &RecordingDesktopWindows<'_>) -> bool {
        desktop
            .top_level_windows(FO4EDIT)
            .iter()
            .any(|window| window.handle == MODULE_SELECTION)
    }
}
