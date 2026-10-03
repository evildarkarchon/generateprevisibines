//! Thin, policy-free safe wrappers over the Win32 window calls `generateprevisibines` needs to
//! drive FO4Edit (ADR-0003).
//!
//! Every call here is an `unsafe fn` in the `windows` crate, and the main crate sets
//! `unsafe_code = "forbid"`, which cannot be overridden in-crate. So the calls live in this
//! separate crate, and the main crate reaches them only through the production adapter of its
//! internal `DesktopWindows` seam.
//!
//! The crate decides nothing. It has no captions, no retries and no dismissal ladder: which window
//! to target, in what order, and what counts as success all stay in the main crate, where a
//! recording double puts them under test. That is also why it has no unit tests of its own — its
//! behaviour against real windows was established by the FO4Edit window probe (issue #40) and is
//! re-checked by the manual integration checklist.
//!
//! The probe ranked `BM_CLICK` above a posted ENTER, so there is deliberately no "post a key";
//! ADR-0003 was amended to drop it from its original list.
//!
//! Off Windows the crate is empty.
#![cfg(windows)]

use std::io;

use windows::Win32::Foundation::{ERROR_INVALID_WINDOW_HANDLE, HWND, LPARAM, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, IsWindowEnabled, KEYBD_EVENT_FLAGS, KEYBDINPUT,
    KEYEVENTF_KEYUP, SendInput, VK_RETURN,
};
use windows::Win32::UI::WindowsAndMessaging::{
    BM_CLICK, EnumChildWindows, EnumWindows, GW_OWNER, GetClassNameW, GetForegroundWindow,
    GetWindow, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible,
    PostMessageW, SMTO_ABORTIFHUNG, SendMessageTimeoutW, SetForegroundWindow, WM_CLOSE, WM_GETTEXT,
};
use windows::core::BOOL;

/// Longest window class name Win32 allows (`WNDCLASS.lpszClassName`), plus the terminator.
const CLASS_NAME_CAPACITY: usize = 257;

/// Buffer for the `WM_GETTEXT` caption fallback, in UTF-16 code units, terminator included.
const SENT_TEXT_CAPACITY: usize = 512;

/// How long the `WM_GETTEXT` fallback waits on the window's thread before giving up.
const SENT_TEXT_TIMEOUT_MS: u32 = 500;

/// `SendInput`'s `cbsize`: the size of one `INPUT`.
// The assertion makes the narrowing cast provably lossless, at compile time.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    reason = "asserted to fit in an i32 just above the cast"
)]
const INPUT_SIZE: i32 = {
    assert!(size_of::<INPUT>() <= i32::MAX as usize);
    size_of::<INPUT>() as i32
};

/// A window handle, bound to the process that owned the window when the handle was read.
///
/// An `HWND` is an index into user32's handle table, not a pointer into memory, so it is kept as
/// a `usize`: comparable, hashable and `Send`. user32 validates every handle it is given, so any
/// value is memory-safe to pass. But it recycles handles: once a window is destroyed, its number
/// can name a later window of any process. So each handle carries the id of the process that
/// owned its window, and every function here that acts on a window first checks that the window
/// still belongs to that process, failing as for a destroyed window when it does not. A stale
/// handle therefore fails or finds nothing rather than reaching another program's window.
///
/// Equality compares the process too, so a recycled handle never equals the one it replaced.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Hwnd {
    raw: usize,
    pid: u32,
}

impl Hwnd {
    /// Rebuild a handle from values previously read through [`Hwnd::raw`] and [`Hwnd::pid`].
    #[must_use]
    pub const fn from_raw(raw: usize, pid: u32) -> Self {
        Self { raw, pid }
    }

    /// The raw handle value.
    #[must_use]
    pub const fn raw(self) -> usize {
        self.raw
    }

    /// The id of the process that owned the window when the handle was read.
    #[must_use]
    pub const fn pid(self) -> u32 {
        self.pid
    }

    /// The `windows` crate's view of this handle, with no ownership check.
    fn to_win32(self) -> HWND {
        // A handle carries no provenance: it is never dereferenced, only handed back to user32.
        HWND(std::ptr::without_provenance_mut(self.raw))
    }

    /// Bind a handle returned by Win32 to the process that owns its window now.
    ///
    /// `None` for the null handle Win32 uses for "none", and for a window destroyed before its
    /// owner could be read.
    fn from_win32(hwnd: HWND) -> Option<Self> {
        if hwnd.is_invalid() {
            return None;
        }
        let raw = hwnd.0.addr();
        let pid = process_id_of(raw);
        // 0 is what `process_id_of` reports for a destroyed window; no live window has it.
        (pid != 0).then_some(Self { raw, pid })
    }

    /// This handle as Win32 takes it, after checking that its window still exists and still
    /// belongs to the process the handle was read from.
    ///
    /// Fails with `ERROR_INVALID_WINDOW_HANDLE` otherwise — including when the number has been
    /// recycled for another process's window. The check and the caller's use of the handle are
    /// separate calls, so a window destroyed and recycled in between is not caught; but that
    /// span is a few instructions, where an unchecked handle is exposed for as long as the caller
    /// keeps it.
    fn checked(self) -> io::Result<HWND> {
        if process_id_of(self.raw) == self.pid {
            Ok(self.to_win32())
        } else {
            Err(invalid_window_handle())
        }
    }
}

/// One top-level window, as it stood when it was read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TopLevelWindow {
    pub handle: Hwnd,
    /// The window's title text; empty when it has none.
    pub caption: String,
    pub class_name: String,
    /// The owner window (`GW_OWNER`), if the window has one.
    pub owner: Option<Hwnd>,
    pub visible: bool,
    pub enabled: bool,
}

/// List the top-level windows that belong to process `pid`, in `EnumWindows` (Z) order.
///
/// Hidden windows are included, with `visible: false`. A `pid` that owns no windows — including
/// one that names no process, and 0, the idle process — gives an empty list.
pub fn top_level_windows(pid: u32) -> io::Result<Vec<TopLevelWindow>> {
    // 0 is the idle process, which owns no windows (and the id `process_id_of` reports for a
    // destroyed one, which `collect_handle` already drops), so there is nothing to enumerate.
    if pid == 0 {
        return Ok(Vec::new());
    }

    let mut handles: Vec<Hwnd> = Vec::new();
    // SAFETY: `collect_handle` only pushes into the `Vec<Hwnd>` whose address is passed as
    // `lparam`. `EnumWindows` invokes the callback synchronously on this thread and has
    // returned before `handles` is touched again, so the vector is alive, and accessed by
    // nothing else, for every use of that address.
    unsafe { EnumWindows(Some(collect_handle), vec_lparam(&mut handles)) }?;

    Ok(handles
        .into_iter()
        .filter(|&handle| handle.pid == pid)
        .map(|handle| TopLevelWindow {
            handle,
            caption: caption(handle),
            class_name: class_name(handle),
            owner: owner(handle),
            // SAFETY: `IsWindowVisible` accepts any handle; a stale one reads as not visible.
            visible: unsafe { IsWindowVisible(handle.to_win32()) }.as_bool(),
            // SAFETY: `IsWindowEnabled` accepts any handle; a stale one reads as not enabled.
            enabled: unsafe { IsWindowEnabled(handle.to_win32()) }.as_bool(),
        })
        .collect())
}

/// Find a descendant of `parent` whose class is `class_name` and whose caption is `caption`.
///
/// Searches every descendant, not only direct children (`EnumChildWindows` recurses), because a
/// dialog's buttons often sit on a panel. Only descendants of `parent`'s own process count. Both
/// strings must match exactly; the first match in enumeration order wins. Returns `Ok(None)`
/// when no descendant matches, and an error when `parent` is no longer a window of the process
/// it was read from.
pub fn find_child(parent: Hwnd, class_name: &str, caption: &str) -> io::Result<Option<Hwnd>> {
    let parent_hwnd = parent.checked()?;

    let mut handles: Vec<Hwnd> = Vec::new();
    // SAFETY: as in `top_level_windows`: the callback only pushes into `handles`, which nothing
    // else touches until this synchronous enumeration has returned. The return value carries no
    // meaning (per the Win32 documentation), so it is not read. A parent destroyed since the
    // ownership check just enumerates nothing.
    let _ = unsafe {
        EnumChildWindows(
            Some(parent_hwnd),
            Some(collect_handle),
            vec_lparam(&mut handles),
        )
    };

    // A child hosted from another process is not the parent's control, and is never sent
    // `WM_GETTEXT`. Class next: it is read without a message, so only controls of the right
    // class are ever sent `WM_GETTEXT`.
    Ok(handles.into_iter().find(|&handle| {
        handle.pid == parent.pid
            && self::class_name(handle) == class_name
            && control_text(handle) == caption
    }))
}

/// Post `BM_CLICK` to `button`, as if the user clicked it. Returns without waiting for the
/// button's window to process the click.
pub fn post_button_click(button: Hwnd) -> io::Result<()> {
    post_message(button, BM_CLICK)
}

/// Post `WM_CLOSE` to `window`, asking it to close. Returns without waiting for the window to
/// act on the request, which it is free to refuse.
pub fn post_close(window: Hwnd) -> io::Result<()> {
    post_message(window, WM_CLOSE)
}

/// Ask Windows to make `window` the foreground window.
///
/// Returns whether Windows granted the request. The foreground rules often refuse it to a
/// background process, so `false` is an ordinary answer rather than an error. A window that no
/// longer belongs to the process it was read from is refused without asking.
#[must_use]
pub fn set_foreground(window: Hwnd) -> bool {
    let Ok(hwnd) = window.checked() else {
        return false;
    };
    // SAFETY: `SetForegroundWindow` accepts any handle; a stale one is refused.
    unsafe { SetForegroundWindow(hwnd) }.as_bool()
}

/// The current foreground window, or `None` when there is none (for instance while focus is
/// changing).
#[must_use]
pub fn foreground_window() -> Option<Hwnd> {
    // SAFETY: `GetForegroundWindow` takes no arguments and only reads state.
    Hwnd::from_win32(unsafe { GetForegroundWindow() })
}

/// Send one ENTER key press to `window`, but only if `window` is the foreground window.
///
/// `SendInput` takes no target: it types into whichever window has keyboard focus. So this
/// reads the foreground window immediately before sending, and returns `Ok(false)` without
/// typing anything when it is not `window` (or `window`'s number now names another process's
/// window). That keeps the check and the send in one call, a few instructions apart; no Win32
/// call makes them atomic, so a program that takes the foreground in that span still receives
/// the key. Returns `Ok(true)` once both events are inserted, and an error when `window` no
/// longer exists or Windows inserts fewer than both events, for instance because input is
/// blocked.
pub fn send_enter_to(window: Hwnd) -> io::Result<bool> {
    window.checked()?;
    if foreground_window() != Some(window) {
        return Ok(false);
    }
    send_enter()?;
    Ok(true)
}

/// Send one ENTER key press — key down, then key up — through `SendInput`, to whichever window
/// has keyboard focus.
///
/// If only the key-down is inserted, the key-up is sent on its own so ENTER is not left held
/// down; the original failure is returned either way.
fn send_enter() -> io::Result<()> {
    let key = |flags: KEYBD_EVENT_FLAGS| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VK_RETURN,
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    let inputs = [key(KEYBD_EVENT_FLAGS(0)), key(KEYEVENTF_KEYUP)];

    // SAFETY: `inputs` is a fully initialised array of keyboard `INPUT`s and `INPUT_SIZE` is
    // the size of one element, as `SendInput` requires.
    let inserted = unsafe { SendInput(&inputs, INPUT_SIZE) };
    if inserted as usize == inputs.len() {
        return Ok(());
    }
    // Read before the key-up retry below can overwrite it.
    let error = io::Error::last_os_error();
    if inserted == 1 {
        // Only the key-down went in. Release the key, or the system would treat ENTER as held
        // until something else sends a key-up. Best effort: if this fails too, there is nothing
        // more to try, and the original failure is what gets reported.
        // SAFETY: as above; the slice holds the one fully initialised key-up `INPUT`.
        let _ = unsafe { SendInput(&inputs[1..], INPUT_SIZE) };
    }
    // Input blocked by UIPI sets no last error, which would otherwise read as "The operation
    // completed successfully".
    if error.raw_os_error() == Some(0) {
        return Err(io::Error::other(format!(
            "SendInput inserted {inserted} of {} key events",
            inputs.len()
        )));
    }
    Err(error)
}

/// `EnumWindows`/`EnumChildWindows` callback: push each handle into the `Vec<Hwnd>` whose
/// address is `lparam`, and keep enumerating.
///
/// # Safety
///
/// `lparam` must come from [`vec_lparam`] on a `Vec<Hwnd>` that stays alive, and is not
/// otherwise accessed, for as long as the enumeration runs.
unsafe extern "system" fn collect_handle(hwnd: HWND, lparam: LPARAM) -> BOOL {
    // SAFETY: the caller's contract: `lparam` is the exposed address of a live, exclusively
    // borrowed `Vec<Hwnd>`.
    let handles = unsafe {
        &mut *std::ptr::with_exposed_provenance_mut::<Vec<Hwnd>>(lparam.0.cast_unsigned())
    };
    handles.extend(Hwnd::from_win32(hwnd));
    BOOL::from(true)
}

/// Pass `handles` to an enumeration callback as its `LPARAM`.
///
/// The address is exposed so [`collect_handle`] can turn the integer back into a pointer.
fn vec_lparam(handles: &mut Vec<Hwnd>) -> LPARAM {
    LPARAM(
        std::ptr::from_mut(handles)
            .expose_provenance()
            .cast_signed(),
    )
}

/// The id of the process that created the window `raw` names now, or 0 when it names none.
fn process_id_of(raw: usize) -> u32 {
    let mut pid = 0u32;
    // SAFETY: `pid` is a live local the call writes through; any handle is accepted, and a stale
    // one leaves `pid` at 0, which is never a process that owns windows.
    unsafe {
        GetWindowThreadProcessId(
            HWND(std::ptr::without_provenance_mut(raw)),
            Some(&raw mut pid),
        )
    };
    pid
}

/// `window`'s class name; empty when the handle is stale.
fn class_name(window: Hwnd) -> String {
    let mut buffer = [0u16; CLASS_NAME_CAPACITY];
    // SAFETY: `GetClassNameW` writes at most `buffer.len()` units, terminator included, into a
    // buffer this function owns. A stale handle writes nothing and returns 0.
    let length = unsafe { GetClassNameW(window.to_win32(), &mut buffer) };
    utf16_prefix(&buffer, length)
}

/// `window`'s owner window (`GW_OWNER`), if it has one.
fn owner(window: Hwnd) -> Option<Hwnd> {
    // SAFETY: `GetWindow` accepts any handle. "No owner" comes back as an error, which reads the
    // same as "unknown" here.
    unsafe { GetWindow(window.to_win32(), GW_OWNER) }
        .ok()
        .and_then(Hwnd::from_win32)
}

/// A top-level window's caption; empty when it has none.
///
/// `GetWindowTextW` reads the caption Windows stores for a top-level window, across processes,
/// without sending the window a message — so a poll that lists a busy FO4Edit's windows never
/// waits on FO4Edit's UI thread.
fn caption(window: Hwnd) -> String {
    let hwnd = window.to_win32();

    // SAFETY: `GetWindowTextLengthW` accepts any handle; a stale one reports 0.
    let reported = unsafe { GetWindowTextLengthW(hwnd) };
    // The reported length can overestimate but never underestimates, so it is a safe capacity.
    let mut buffer = vec![0u16; usize::try_from(reported).unwrap_or(0) + 1];
    // SAFETY: `GetWindowTextW` writes at most `buffer.len()` units, terminator included, into a
    // buffer this function owns.
    let length = unsafe { GetWindowTextW(hwnd, &mut buffer) };
    utf16_prefix(&buffer, length)
}

/// A child control's text, such as a button's caption; empty when it has none or does not
/// answer in time.
///
/// `GetWindowTextW` cannot read a control in another process (Win32 documents that it returns
/// the stored caption only for top-level windows there), so this sends `WM_GETTEXT`, the
/// documented way, with a timeout so a hung window cannot stall the caller.
fn control_text(window: Hwnd) -> String {
    // On the heap, not the stack, so a failed send can leak it (below) rather than free it.
    let mut buffer = vec![0u16; SENT_TEXT_CAPACITY].into_boxed_slice();
    let mut copied = 0usize;
    // SAFETY: `WM_GETTEXT`'s `wparam` is the buffer's capacity and its `lparam` the buffer's
    // address. Windows copies at most that many units into the buffer, which stays allocated
    // for as long as the receiver could write to it: until this call returns when it succeeds,
    // and forever when it fails (see below). `copied` is a live local written by this call
    // itself before it returns, never by the receiver. The timeout and `SMTO_ABORTIFHUNG` bound
    // how long a hung window can hold the call.
    let sent = unsafe {
        SendMessageTimeoutW(
            window.to_win32(),
            WM_GETTEXT,
            WPARAM(buffer.len()),
            // Exposed, not just `addr()`: user32 turns the integer back into a pointer.
            LPARAM(buffer.as_mut_ptr().expose_provenance().cast_signed()),
            SMTO_ABORTIFHUNG,
            SENT_TEXT_TIMEOUT_MS,
            Some(&raw mut copied),
        )
    };
    if sent.0 == 0 {
        // A message that timed out is not withdrawn: the window can still handle it later.
        // Across processes the reply is copied back only while this call waits, but within this
        // process `WM_GETTEXT` is not marshalled, so a late handler writes straight into the
        // buffer. Leaking its 1 KiB is the price of never freeing memory under that writer.
        Box::leak(buffer);
        return String::new();
    }
    String::from_utf16_lossy(&buffer[..copied.min(buffer.len())])
}

/// Post `message`, with no parameters, to `window`, once it is confirmed to still belong to the
/// process it was read from.
fn post_message(window: Hwnd, message: u32) -> io::Result<()> {
    let hwnd = window.checked()?;
    // SAFETY: the messages this crate posts (`BM_CLICK`, `WM_CLOSE`) carry no pointers, so
    // nothing the receiver reads can dangle. A stale handle makes the call fail, which is
    // returned.
    unsafe { PostMessageW(Some(hwnd), message, WPARAM(0), LPARAM(0)) }?;
    Ok(())
}

/// The first `length` units of `buffer` as a string; empty for a non-positive `length`.
fn utf16_prefix(buffer: &[u16], length: i32) -> String {
    let length = usize::try_from(length).unwrap_or(0).min(buffer.len());
    String::from_utf16_lossy(&buffer[..length])
}

/// The error Win32 itself reports for a handle that names no window.
fn invalid_window_handle() -> io::Error {
    io::Error::from_raw_os_error(ERROR_INVALID_WINDOW_HANDLE.0.cast_signed())
}
