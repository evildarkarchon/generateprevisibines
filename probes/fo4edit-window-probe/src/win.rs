//! Thin Win32 wrappers for the probe. Every `unsafe` in the probe lives here.
//!
//! These wrappers report raw API results (bools, error codes) rather than deciding anything:
//! the probe's job is to record what Windows did, not to hide failures behind policy.

use std::collections::HashMap;
use std::ffi::c_void;
use std::mem::size_of;

use serde::Serialize;
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, WPARAM};
use windows::Win32::System::Console::GetConsoleWindow;
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput,
    VK_RETURN,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumChildWindows, EnumWindows, GUITHREADINFO, GW_OWNER, GWL_EXSTYLE, GWL_STYLE,
    GetClassNameW, GetDlgCtrlID, GetForegroundWindow, GetGUIThreadInfo, GetWindow,
    GetWindowLongW, GetWindowTextW, GetWindowThreadProcessId, IsWindow, IsWindowVisible,
    PostMessageW, SMTO_ABORTIFHUNG, SPI_GETFOREGROUNDLOCKTIMEOUT,
    SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SendMessageTimeoutW, SetForegroundWindow,
    SystemParametersInfoW, WM_GETTEXT, WS_DISABLED,
};
use windows::core::BOOL;

/// Raw window handle value, kept as an integer so it is `Send`, comparable and serialisable.
pub type Hwnd = isize;

fn to_hwnd(h: Hwnd) -> HWND {
    HWND(h as *mut c_void)
}

/// Snapshot of one window's identity and state at the moment it was read.
#[derive(Debug, Clone, Serialize)]
pub struct WindowInfo {
    /// Handle as hex, for reading reports.
    pub hwnd: String,
    #[serde(skip)]
    pub raw: Hwnd,
    pub class: String,
    pub title: String,
    pub pid: u32,
    pub thread_id: u32,
    /// `GetWindow(GW_OWNER)`, hex, if the window has an owner.
    pub owner: Option<String>,
    pub visible: bool,
    /// `WS_DISABLED` is set — what .NET's `CloseMainWindow()` checks before posting.
    pub disabled: bool,
    pub style: String,
    pub ex_style: String,
    /// `GetDlgCtrlID`; only meaningful for child windows.
    pub ctrl_id: i32,
}

/// `GetGUIThreadInfo` for one GUI thread: which of its windows is active and focused.
#[derive(Debug, Clone, Serialize)]
pub struct GuiThreadInfo {
    pub ok: bool,
    pub error: Option<String>,
    pub flags: String,
    pub active: Option<WindowInfo>,
    pub focus: Option<WindowInfo>,
    pub capture: Option<String>,
}

/// One process from a Toolhelp snapshot.
#[derive(Debug, Clone, Serialize)]
pub struct ProcInfo {
    pub pid: u32,
    pub parent_pid: u32,
    pub exe: String,
}

/// Formats a handle as `0x…` hex.
pub fn hex(h: Hwnd) -> String {
    format!("{h:#x}")
}

/// Reads a window's identity and state. Text comes from `GetWindowTextW`, falling back to a
/// `WM_GETTEXT` sent with a timeout, because `GetWindowText` does not send `WM_GETTEXT` across
/// processes and some controls keep their caption only behind that message.
pub fn info(h: Hwnd) -> WindowInfo {
    let hwnd = to_hwnd(h);
    let mut pid = 0u32;
    // SAFETY: plain Win32 queries on a window handle; a stale handle only yields empty results.
    unsafe {
        let thread_id = GetWindowThreadProcessId(hwnd, Some(&mut pid));
        let mut class_buf = [0u16; 256];
        let class_len = GetClassNameW(hwnd, &mut class_buf);
        let class = String::from_utf16_lossy(&class_buf[..class_len.max(0) as usize]);
        let mut title_buf = [0u16; 512];
        let title_len = GetWindowTextW(hwnd, &mut title_buf);
        let mut title = String::from_utf16_lossy(&title_buf[..title_len.max(0) as usize]);
        if title.is_empty() {
            let mut result = 0usize;
            let sent = SendMessageTimeoutW(
                hwnd,
                WM_GETTEXT,
                WPARAM(title_buf.len()),
                LPARAM(title_buf.as_mut_ptr() as isize),
                SMTO_ABORTIFHUNG,
                500,
                Some(&mut result),
            );
            if sent.0 != 0 && result > 0 {
                title = String::from_utf16_lossy(&title_buf[..result.min(title_buf.len())]);
            }
        }
        let owner = GetWindow(hwnd, GW_OWNER).ok().filter(|o| !o.is_invalid()).map(|o| hex(o.0 as isize));
        let style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
        let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
        WindowInfo {
            hwnd: hex(h),
            raw: h,
            class,
            title,
            pid,
            thread_id,
            owner,
            visible: IsWindowVisible(hwnd).as_bool(),
            disabled: style & WS_DISABLED.0 != 0,
            style: format!("{style:#010x}"),
            ex_style: format!("{ex_style:#010x}"),
            ctrl_id: GetDlgCtrlID(hwnd),
        }
    }
}

unsafe extern "system" fn collect(hwnd: HWND, lparam: LPARAM) -> BOOL {
    // SAFETY: `lparam` is the `&mut Vec<Hwnd>` passed by `top_level`/`children`, alive for the
    // whole synchronous enumeration.
    let out = unsafe { &mut *(lparam.0 as *mut Vec<Hwnd>) };
    out.push(hwnd.0 as isize);
    BOOL(1)
}

/// Every top-level window on the desktop, in `EnumWindows` (Z) order.
pub fn top_level() -> Vec<Hwnd> {
    let mut out: Vec<Hwnd> = Vec::new();
    // SAFETY: the callback only pushes into `out`, which outlives the call.
    unsafe {
        let _ = EnumWindows(Some(collect), LPARAM(&mut out as *mut Vec<Hwnd> as isize));
    }
    out
}

/// Top-level windows owned by `pid`, in Z order.
pub fn top_level_of(pid: u32) -> Vec<WindowInfo> {
    top_level().into_iter().map(info).filter(|w| w.pid == pid).collect()
}

/// Every descendant of `parent` (`EnumChildWindows` recurses).
pub fn children(parent: Hwnd) -> Vec<WindowInfo> {
    let mut out: Vec<Hwnd> = Vec::new();
    // SAFETY: as in `top_level`.
    unsafe {
        let _ = EnumChildWindows(
            Some(to_hwnd(parent)),
            Some(collect),
            LPARAM(&mut out as *mut Vec<Hwnd> as isize),
        );
    }
    out.into_iter().map(info).collect()
}

/// .NET's `Process.MainWindowHandle` heuristic: the first top-level window of `pid` in
/// `EnumWindows` order that has no owner and is visible.
pub fn dotnet_main_window(pid: u32) -> Option<WindowInfo> {
    top_level_of(pid).into_iter().find(|w| w.owner.is_none() && w.visible)
}

/// Whether the handle still names a window.
pub fn exists(h: Hwnd) -> bool {
    // SAFETY: `IsWindow` accepts any value.
    unsafe { IsWindow(Some(to_hwnd(h))).as_bool() }
}

/// The current foreground window, if any.
pub fn foreground() -> Option<WindowInfo> {
    // SAFETY: no arguments.
    let h = unsafe { GetForegroundWindow() };
    (!h.is_invalid()).then(|| info(h.0 as isize))
}

/// `GetGUIThreadInfo` for `thread_id`.
pub fn gui_thread_info(thread_id: u32) -> GuiThreadInfo {
    let mut gti = GUITHREADINFO { cbSize: size_of::<GUITHREADINFO>() as u32, ..Default::default() };
    // SAFETY: `gti` is a correctly sized, initialised out-parameter.
    let result = unsafe { GetGUIThreadInfo(thread_id, &mut gti) };
    let opt = |h: HWND| (!h.is_invalid()).then(|| info(h.0 as isize));
    match result {
        Ok(()) => GuiThreadInfo {
            ok: true,
            error: None,
            flags: format!("{:#x}", gti.flags.0),
            active: opt(gti.hwndActive),
            focus: opt(gti.hwndFocus),
            capture: (!gti.hwndCapture.is_invalid()).then(|| hex(gti.hwndCapture.0 as isize)),
        },
        Err(e) => GuiThreadInfo {
            ok: false,
            error: Some(e.to_string()),
            flags: String::new(),
            active: None,
            focus: None,
            capture: None,
        },
    }
}

/// `PostMessageW`; `Err` carries the Win32 error text (UIPI blocks show up as access denied).
pub fn post(h: Hwnd, msg: u32, wparam: usize, lparam: isize) -> Result<(), String> {
    // SAFETY: the messages the probe posts carry no pointers.
    unsafe { PostMessageW(Some(to_hwnd(h)), msg, WPARAM(wparam), LPARAM(lparam)) }
        .map_err(|e| e.to_string())
}

/// `SetForegroundWindow`'s raw result.
pub fn set_foreground(h: Hwnd) -> bool {
    // SAFETY: plain call on a window handle.
    unsafe { SetForegroundWindow(to_hwnd(h)).as_bool() }
}

/// Injects ENTER down + up into the system input stream; returns how many events were inserted.
pub fn send_input_enter() -> u32 {
    let key = |flags: KEYBD_EVENT_FLAGS| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT { wVk: VK_RETURN, wScan: 0, dwFlags: flags, time: 0, dwExtraInfo: 0 },
        },
    };
    let inputs = [key(KEYBD_EVENT_FLAGS(0)), key(KEYEVENTF_KEYUP)];
    // SAFETY: `inputs` is a valid slice of initialised `INPUT`s.
    unsafe { SendInput(&inputs, size_of::<INPUT>() as i32) }
}

/// `SPI_GETFOREGROUNDLOCKTIMEOUT` in milliseconds, if readable.
pub fn foreground_lock_timeout_ms() -> Option<u32> {
    let mut value = 0u32;
    // SAFETY: SPI_GETFOREGROUNDLOCKTIMEOUT writes one DWORD into `value`.
    unsafe {
        SystemParametersInfoW(
            SPI_GETFOREGROUNDLOCKTIMEOUT,
            0,
            Some(&mut value as *mut u32 as *mut c_void),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    }
    .ok()
    .map(|()| value)
}

/// This process's console window (conhost's `ConsoleWindowClass`, or ConPTY's
/// `PseudoConsoleWindow` under Windows Terminal), if it has one.
pub fn console_window() -> Option<WindowInfo> {
    // SAFETY: no arguments.
    let h = unsafe { GetConsoleWindow() };
    (!h.is_invalid()).then(|| info(h.0 as isize))
}

/// All processes from a Toolhelp snapshot, keyed by PID.
pub fn processes() -> HashMap<u32, ProcInfo> {
    let mut map = HashMap::new();
    // SAFETY: the snapshot handle is closed below; `entry` is correctly sized.
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return map };
        let mut entry = PROCESSENTRY32W { dwSize: size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut ok = Process32FirstW(snap, &mut entry).is_ok();
        while ok {
            let len = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
            map.insert(
                entry.th32ProcessID,
                ProcInfo {
                    pid: entry.th32ProcessID,
                    parent_pid: entry.th32ParentProcessID,
                    exe: String::from_utf16_lossy(&entry.szExeFile[..len]),
                },
            );
            ok = Process32NextW(snap, &mut entry).is_ok();
        }
        let _ = CloseHandle(snap);
    }
    map
}
