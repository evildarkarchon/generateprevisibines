# Win32 window calls live in a helper crate; the main crate keeps `unsafe_code = "forbid"`

Status: accepted. Amended 2026-10-03: the helper's API drops "post a key", because the FO4Edit window probe (#40) ranked `BM_CLICK` above a posted ENTER, and it finds a child window by class as well as caption. The helper is `crates/win32-windows`, and the seam is `DesktopWindows` in `src/tools/desktop.rs`. Both were specified in #55 and #57.

Driving FO4Edit needs Win32 window calls the batch reached through PowerShell. It needs them to find the Module Selection dialog by process and caption, to dismiss it, and to ask FO4Edit to close. PowerShell is a do-nothing stub under Wine, so it cannot stay out of process, and Wine is a hard requirement. Every candidate call is an `unsafe fn` in the `windows` crate: `EnumWindows`, `PostMessage`, `SendMessage` (`BM_CLICK`), `SendInput` and `SetForegroundWindow`. The main crate sets `unsafe_code = "forbid"`, which cannot be overridden in-crate. We put those calls in a separate workspace member crate that allows `unsafe` and exposes only a thin, policy-free safe API: list a process's top-level windows with their caption, class, owner, enabled and visible state; find a child window by class and caption; post `BM_CLICK` to a button; post `WM_CLOSE`; set the foreground window and read it back; send ENTER. The main crate keeps `forbid` and reaches the helper only through the production implementation of its internal window seam. Which window to target, the dismissal ladder and the close sequence stay in the main crate, under test.

This follows the precedent `Cargo.toml` already records for `chrono`: when `forbid` blocks an OS call, the call moves into a dependency rather than the lint being relaxed.

## Considered options

- **Relax the main crate to `deny`, with one `#[expect(unsafe_code)]` module.** Less ceremony, but the guarantee becomes a convention that any later `allow` can bypass. `forbid` enforces it mechanically, and that is the property worth keeping.
- **`winsafe`.** It makes enumeration and `SetForegroundWindow` safe, but still marks `PostMessage` and `SendMessage` `unsafe`, so targeted dismissal and the close request would still need `unsafe` somewhere.
- **An out-of-process PowerShell helper, as the batch uses.** A silent no-op under Wine: Wine's `powershell.exe` is a stub, and `WScript.Shell`'s `AppActivate`/`SendKeys` return `E_NOTIMPL`.

## Consequences

The helper has no logic worth unit-testing. Its correctness is established by the real-machine experiments the FO4Edit design ticket graduated, not by the crate's test suite. Anything that decides *what* to do with a window belongs in the main crate, where the recording window seam can put it under assertion.
