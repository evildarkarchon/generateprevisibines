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
//! Every action is a message posted to one named window, so nothing this crate does can reach a
//! window it was not given. ENTER in particular is posted, never typed: `SendInput` takes no
//! target and types into whichever window has the foreground when the key is processed, and no
//! Win32 call makes "check the foreground, then type" atomic, so a program that takes the
//! foreground in between would receive the key. The probe (issue #40) found a posted ENTER
//! dismisses Module Selection as reliably as `BM_CLICK`, with the dialog in the background.
//!
//! Off Windows the crate is empty.
#![cfg(windows)]

use std::io;

use windows::Win32::Foundation::{ERROR_INVALID_WINDOW_HANDLE, HWND, LPARAM, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{IsWindowEnabled, VK_RETURN};
use windows::Win32::UI::WindowsAndMessaging::{
    BM_CLICK, EnumChildWindows, EnumWindows, GUITHREADINFO, GW_OWNER, GetClassNameW,
    GetGUIThreadInfo, GetWindow, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId,
    IsChild, IsWindowVisible, PostMessageW, SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_CLOSE,
    WM_GETTEXT, WM_KEYDOWN, WM_KEYUP,
};
use windows::core::BOOL;

/// Longest window class name Win32 allows (`WNDCLASS.lpszClassName`), plus the terminator.
const CLASS_NAME_CAPACITY: usize = 257;

/// Buffer for the `WM_GETTEXT` caption fallback, in UTF-16 code units, terminator included.
const SENT_TEXT_CAPACITY: usize = 512;

/// How long the `WM_GETTEXT` fallback waits on the window's thread before giving up.
const SENT_TEXT_TIMEOUT_MS: u32 = 500;

/// `WM_KEYDOWN`'s `lparam` for ENTER: repeat count 1, scan code `0x1C`. The values a real key
/// press carries, and the ones the probe posted.
const ENTER_DOWN_LPARAM: isize = 0x001C_0001;

/// `WM_KEYUP`'s `lparam` for ENTER: as for the key-down, plus the previous-state (bit 30) and
/// transition-state (bit 31) flags every key-up carries.
const ENTER_UP_LPARAM: isize = 0xC01C_0001;

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
    post_message(button, BM_CLICK, WPARAM(0), LPARAM(0))
}

/// Post `WM_CLOSE` to `window`, asking it to close. Returns without waiting for the window to
/// act on the request, which it is free to refuse.
pub fn post_close(window: Hwnd) -> io::Result<()> {
    post_message(window, WM_CLOSE, WPARAM(0), LPARAM(0))
}

/// Post one ENTER key press — `WM_KEYDOWN`, then `WM_KEYUP`, for `VK_RETURN` — to `window`'s
/// focused control, or to `window` itself when focus is not inside it. Returns without waiting
/// for the window to process the key.
///
/// Both messages go to `window` or one of its own descendants, never to whichever window has
/// the foreground, so the key can reach no other program whatever the user is doing meanwhile.
/// The focused control is the one a typed key would reach; for Module Selection that is its
/// module tree, which is where the probe (issue #40) posted the ENTER that dismissed it.
///
/// A posted key does not change the keyboard state, so the receiver sees the real modifier
/// keys: a physically held Ctrl turns Module Selection's ENTER into a single-module load. And
/// if the key-up cannot be posted after the key-down was, nothing is left held down. An error
/// means `window` no longer belongs to the process it was read from, or a post failed (for
/// instance because UIPI blocks posting to a higher-integrity process).
pub fn post_enter(window: Hwnd) -> io::Result<()> {
    let target = focused_control(window)?.unwrap_or(window);
    post_message(
        target,
        WM_KEYDOWN,
        WPARAM(VK_RETURN.0.into()),
        LPARAM(ENTER_DOWN_LPARAM),
    )?;
    post_message(
        target,
        WM_KEYUP,
        WPARAM(VK_RETURN.0.into()),
        LPARAM(ENTER_UP_LPARAM),
    )
}

/// The window with keyboard focus on `window`'s thread, when that is `window` itself or one of
/// its descendants in the same process; `Ok(None)` when focus is anywhere else or nowhere.
///
/// Fails when `window` no longer belongs to the process it was read from.
fn focused_control(window: Hwnd) -> io::Result<Option<Hwnd>> {
    let hwnd = window.checked()?;
    // SAFETY: `GetWindowThreadProcessId` accepts any handle and, with no out-pointer, only
    // returns the thread id; a window destroyed since the check gives 0.
    let thread = unsafe { GetWindowThreadProcessId(hwnd, None) };
    if thread == 0 {
        return Err(invalid_window_handle());
    }

    let mut info = GUITHREADINFO {
        cbSize: u32::try_from(size_of::<GUITHREADINFO>()).expect("GUITHREADINFO fits in a u32"),
        ..Default::default()
    };
    // SAFETY: `info` is a live local of the size its `cbSize` states, which the call fills in.
    // A thread that has since exited makes the call fail, which reads as "no focus" below.
    if unsafe { GetGUIThreadInfo(thread, &raw mut info) }.is_err() {
        return Ok(None);
    }

    // The focus window is bound to its process like any other handle, so a focus in another
    // process (a thread attached with `AttachThreadInput`) is never a target, and the post
    // re-checks that binding.
    Ok(Hwnd::from_win32(info.hwndFocus).filter(|&focus| {
        focus.pid == window.pid
            && (focus == window
                // SAFETY: `IsChild` accepts any pair of handles and only reads the window tree.
                || unsafe { IsChild(hwnd, focus.to_win32()) }.as_bool())
    }))
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

/// Post `message` to `window`, once it is confirmed to still belong to the process it was read
/// from.
///
/// Only for messages whose parameters are plain values: a posted message is handled after this
/// returns, so a pointer in it could dangle.
fn post_message(window: Hwnd, message: u32, wparam: WPARAM, lparam: LPARAM) -> io::Result<()> {
    let hwnd = window.checked()?;
    // SAFETY: the messages this crate posts (`BM_CLICK`, `WM_CLOSE`, `WM_KEYDOWN`, `WM_KEYUP`)
    // carry a key code and key flags at most, never a pointer, so nothing the receiver reads can
    // dangle. A stale handle makes the call fail, which is returned.
    unsafe { PostMessageW(Some(hwnd), message, wparam, lparam) }?;
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
