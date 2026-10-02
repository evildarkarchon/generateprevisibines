# Driving and closing FO4Edit's windows on Windows and Wine/Proton

Research for [#25](https://github.com/evildarkarchon/generateprevisibines/issues/25) (map #23).
**Facts only.** The FO4Edit design ticket decides what to do with them.

Researched 2026-10-02. Every claim cites the source that owns it. Where something could not
be established from a source, it says so and points at an entry in [Open facts](#open-facts).
"Inference" marks a conclusion drawn from cited facts rather than stated by a source.

## Pinned sources

| Source | Version pinned | Why this version |
|---|---|---|
| xEdit (`TES5Edit/TES5Edit`) | tag [`xedit-4.1.5q`](https://github.com/TES5Edit/TES5Edit/tree/fd1e36020b2b5b6217e553dc0038983146a2e2dd) (`fd1e360`) | The V2.99 batch header names "xEdit 4.1.5q" (`GeneratePrevisibines.bat` line 15) |
| xEdit dev branch | `dev-4.1.6` @ [`9fb0168`](https://github.com/TES5Edit/TES5Edit/tree/9fb016884bec138ea6c7b872cec831537d464c3e) | Shows one behaviour change after 4.1.5q (§1.1) |
| Wine | tag [`wine-11.18`](https://github.com/wine-mirror/wine/tree/wine-11.18) (GitHub mirror of gitlab.winehq.org) | Latest dev release |
| Proton's Wine | `ValveSoftware/wine` branch `proton_11.0` @ `dc26e61` (`VERSION` = "Wine version 11.0") | Base of Proton 11.0-2, the latest Proton release ([releases](https://github.com/ValveSoftware/Proton/releases)) |
| .NET | `dotnet/runtime` @ `c94f652`; .NET Framework [reference source](https://github.com/microsoft/referencesource) | `Process.CloseMainWindow` as called from PowerShell 7 / Windows PowerShell 5.1 |
| Rust std | `rust-lang/rust` @ `dba8825` | `Child::kill` on Windows |
| `windows` crate | 0.62.2 ([docs](https://microsoft.github.io/windows-docs-rs/)) | Current release |
| `winsafe` crate | 0.0.29 ([docs.rs](https://docs.rs/winsafe/latest/winsafe/)) | Safe-wrapper comparison |
| winetricks | `Winetricks/winetricks` @ `f3890f6` | `powershell` / `wsh57` verbs |

**Batch line numbers.** The committed `GeneratePrevisibines.bat` is **V2.99** (564 lines).
`:RunScript` is lines 530–564 there. `docs/episodes.md` and `docs/workarounds.md` cite V2.98,
which runs nine lines earlier (V2.99 543 = V2.98 534, and so on). This file uses V2.99 numbers.

## Summary (most decision-relevant first)

1. **At xEdit 4.1.5q, a `-Script:` run silently ignores both `-autoexit` and `-autoload`.**
   `-Script:` selects the `tmScript` tool mode, and 4.1.5q parses those two switches only
   inside `if wbToolMode = tmEdit`. That is the source-level reason for the batch's
   "Close the xEdit window as it does not Autoclose" comment. An unreleased `dev-4.1.6` commit
   fixes it; once released, `-autoload` would skip the Module Selection dialog entirely (§1.1).
2. **ENTER on Module Selection is handled by a form-level `OnKeyDown` (with `KeyPreview`), not
   by a default button.** `btnOK` has no `Default = True`. ENTER runs `SimulateLoad` and then
   `btnOK.Click`, and only if `btnOK.Enabled`. If the module list has an error, ENTER does
   nothing and the batch's log poll never ends (§1.2).
3. **Other modal dialogs can come before Module Selection.** The developer message appears every
   14 days for non-patrons, and its *default* button is **"Open Patreon"**. A "What's New" dialog
   appears after an xEdit upgrade. An untargeted ENTER goes to whichever of these is active
   (§1.3).
4. **`SendKeys` and `SendInput` are untargeted.** They feed the system input stream, which
   delivers to the foreground thread's focus window. If the console has focus, the ENTER goes
   to the console. `AppActivate` calls `SetForegroundWindow` internally, so it is under the
   same foreground lock rules. It has no documented bypass, and the batch discards its return
   value (§2).
5. **Under Wine/Proton the batch's keystroke step cannot work as written.** Wine's
   `powershell.exe` is a do-nothing stub. Wine's `WScript.Shell.AppActivate` and `SendKeys`
   are stubs returning `E_NOTIMPL`, unchanged on Proton 11.0. Wine's UIA has no Invoke pattern
   for Win32 buttons. Wine does implement `SendInput`, `PostMessage`, `BM_CLICK`,
   `EnumWindows`, `TerminateProcess` and `taskkill` (with WM_CLOSE semantics when `/F` is not
   given). Since Wine 10.20 it also enforces its own foreground-steal restriction (§6).
6. **`CloseMainWindow()` posts `WM_CLOSE` to one heuristically chosen window.** It returns
   `false` without posting when that window is disabled, which is what happens while a modal
   dialog is up. **The batch's `TaskKill /IM` has no `/F`**, so it is also a graceful request,
   not a kill. It targets every process with that image name (§4).
7. **Rust:** `std::process::Command::spawn` gives the async spawn and keeps the handle.
   `Child::kill` is `TerminateProcess(h, 1)`. `std` has no window APIs. Every relevant `windows`
   crate function is `unsafe fn`, and this crate sets `unsafe_code = "forbid"`, which cannot be
   overridden inside the crate (§5).

---

## 0. What the batch does today (V2.99 lines 530–564)

- **543:** `START "xEdit" /B %1 -fo4 -autoexit -P:"%LocPlugins_%" %ModDir_% -Script:%2 -Mod:%3 -log:"%UnattenedLogfile_%"`
  launches xEdit without waiting.
- **546:** `Powershell -command "& {$wshell = New-Object -ComObject wscript.shell;$wshell.AppActivate('%xEditProc_%');Start-Sleep -s 1;$wshell.AppActivate('Module Selection');$wshell.SendKeys('{ENTER}')}" >nul`.
  `%xEditProc_%` is `%~n1`, the xEdit file name without its extension (line 532).
  - `WshShell.AppActivate` matches the **window title**: exact match first, then prefix, then
    suffix. It also accepts a process ID
    ([AppActivate Method](https://learn.microsoft.com/en-us/previous-versions/windows/internet-explorer/ie-developer/windows-scripting/wzcddbek(v=vs.84))).
    So `AppActivate('FO4Edit')` finds a window whose title begins or ends with the exe stem.
    It does not look up a process name.
  - Both `AppActivate` calls return a Boolean that the script never reads
    ([same page](https://learn.microsoft.com/en-us/previous-versions/windows/internet-explorer/ie-developer/windows-scripting/wzcddbek(v=vs.84))).
    Output goes to `>nul`. A failure is therefore invisible.
- **548–550:** poll every 5 s until the unattended log exists. There is no upper bound.
- **553:** `powershell (ps %xEditProc_%).CloseMainWindow()`. `ps` is `Get-Process -Name`. With
  several matching processes, member-access enumeration calls `CloseMainWindow()` on each one
  ([about_Member-Access_Enumeration](https://learn.microsoft.com/powershell/module/microsoft.powershell.core/about/about_member-access_enumeration)).
- **556:** `TaskKill /IM %xEditProc_%.exe` has **no `/f`**. See §4.3.

---

## 1. xEdit side (source: `xedit-4.1.5q` unless noted)

### 1.1 `-Script:` puts xEdit in `tmScript` mode, where 4.1.5q ignores `-autoexit` and `-autoload`

- `wbFindCmdLineParam(aSwitch, …)` matches an argument of the form `-<switch>:<value>`
  (case-insensitive) ([`Core/wbCommandLine.pas` L38–64](https://github.com/TES5Edit/TES5Edit/blob/fd1e36020b2b5b6217e553dc0038983146a2e2dd/Core/wbCommandLine.pas#L38-L64)).
- `CheckForcedMode` reads `-script:` into `xeScriptToRun`. For a plain `.pas` extension it sets
  `wbForcedModes := ',script'`
  ([`xEdit/xeInit.pas` L712–728](https://github.com/TES5Edit/TES5Edit/blob/fd1e36020b2b5b6217e553dc0038983146a2e2dd/xEdit/xeInit.pas#L712-L728)).
  `DetectAppMode` then picks the first tool mode for which `wbFindCmdLineParam(s, p)` matches or
  `wbForcedModes` contains the name. For the batch's command line that is `'script'`
  ([L684–698](https://github.com/TES5Edit/TES5Edit/blob/fd1e36020b2b5b6217e553dc0038983146a2e2dd/xEdit/xeInit.pas#L684-L698)),
  which becomes `wbToolMode := tmScript` ([L779–781](https://github.com/TES5Edit/TES5Edit/blob/fd1e36020b2b5b6217e553dc0038983146a2e2dd/xEdit/xeInit.pas#L779-L781)).
- In 4.1.5q, `FindCmdLineSwitch('autoload')` (L1229) and `FindCmdLineSwitch('autoexit')`
  (L1232) sit inside `if wbToolMode = tmEdit then begin … end` (L1217–1259)
  ([source](https://github.com/TES5Edit/TES5Edit/blob/fd1e36020b2b5b6217e553dc0038983146a2e2dd/xEdit/xeInit.pas#L1217-L1259)).
  **In a `-Script:` run, both switches are never read.**
- The script-mode end path already supports auto-exit: `tmrGeneratorTimer` runs `DoRunScript`
  and then `if xeAutoExit then tmrShutdown.Enabled := True`
  ([`xeMainForm.pas` L17989–18002](https://github.com/TES5Edit/TES5Edit/blob/fd1e36020b2b5b6217e553dc0038983146a2e2dd/xEdit/xeMainForm.pas#L17989-L18002)).
  `tmrShutdownTimer` just calls `Close`
  ([L17686](https://github.com/TES5Edit/TES5Edit/blob/fd1e36020b2b5b6217e553dc0038983146a2e2dd/xEdit/xeMainForm.pas#L17686)).
  With `xeAutoExit` never set, xEdit only logs "You can close this application now."
  ([`DoRunScript`, L5039–5086](https://github.com/TES5Edit/TES5Edit/blob/fd1e36020b2b5b6217e553dc0038983146a2e2dd/xEdit/xeMainForm.pas#L5039-L5086))
  and stays open. This matches the batch comment at line 551.
- xEdit's changelog says `-autoexit` works with Script mode
  ([`whatsnew.md` L1354–1356](https://github.com/TES5Edit/TES5Edit/blob/fd1e36020b2b5b6217e553dc0038983146a2e2dd/whatsnew.md#L1354-L1356), 4.0.2 section).
  It also says `-autoload` "will not show the Module Selection dialog and just load all modules
  that are active according to plugins.txt"
  ([L2274–2276](https://github.com/TES5Edit/TES5Edit/blob/fd1e36020b2b5b6217e553dc0038983146a2e2dd/whatsnew.md#L2274-L2276), 4.0.0 section).
  The 4.1.5q code does not do this for `tmScript`.
- **Unreleased fix:** `dev-4.1.6` commit
  [`41ab0d5`](https://github.com/TES5Edit/TES5Edit/commit/41ab0d5bfdc1a12b0001e9a027c13824516d0c47)
  (2026-09-06, ElminsterAU) moves both switches under `wbToolMode in [tmEdit, tmScript]`. Its
  message reads: "*a script run started with -script: always opened the module selection
  dialog and waited after the script although the generator timer already closes the editor
  when autoexit is set*". As of this research there is no tag after `xedit-4.1.5q` (tags list).
  The latest GitHub *release* is 4.1.5f.
- With `-autoload` honoured (dev), the startup path fills the module list from
  `wbModulesByLoadOrder.SimulateLoad` and never creates `TfrmModuleSelect`. The exception is
  when **Ctrl is physically held** (`GetAsyncKeyState(VK_CONTROL) >= 0`)
  ([4.1.5q `xeMainForm.pas` L5455–5470](https://github.com/TES5Edit/TES5Edit/blob/fd1e36020b2b5b6217e553dc0038983146a2e2dd/xEdit/xeMainForm.pas#L5455-L5470)).
  `-P:` sets the plugins file that "active" is read from
  ([`xeInit.pas` L562](https://github.com/TES5Edit/TES5Edit/blob/fd1e36020b2b5b6217e553dc0038983146a2e2dd/xEdit/xeInit.pas#L562)).
- `tmScript` is in `wbAutoModes` and not in `wbPluginModes`
  ([`Core/wbInterface.pas` L4928–4950](https://github.com/TES5Edit/TES5Edit/blob/fd1e36020b2b5b6217e553dc0038983146a2e2dd/Core/wbInterface.pas#L4928-L4950)).
  So without `-autoload` the Module Selection dialog is always shown in script mode.

### 1.2 The Module Selection dialog

Sources: [`xeModuleSelectForm.dfm`](https://github.com/TES5Edit/TES5Edit/blob/fd1e36020b2b5b6217e553dc0038983146a2e2dd/xEdit/xeModuleSelectForm.dfm)
and [`xeModuleSelectForm.pas`](https://github.com/TES5Edit/TES5Edit/blob/fd1e36020b2b5b6217e553dc0038983146a2e2dd/xEdit/xeModuleSelectForm.pas).
Both are byte-identical at 4.1.5q and dev HEAD.

- It is a VCL form `frmModuleSelect: TfrmModuleSelect`, `Caption = 'Module Selection'`,
  `KeyPreview = True`, `OnKeyDown = FormKeyDown` (dfm L1–14). The startup path shows it with
  `ShowModal` and owner `Self` (the main form)
  ([`xeMainForm.pas` L5469](https://github.com/TES5Edit/TES5Edit/blob/fd1e36020b2b5b6217e553dc0038983146a2e2dd/xEdit/xeMainForm.pas#L5469)).
- `btnOK: TButton` has `Caption = 'OK'`, `ModalResult = 1`, `OnClick = btnOKClick`, and **no
  `Default` property** (dfm L29–38). `btnCancel` is `Visible = False` (dfm L171–180).
- `FormKeyDown` (pas L467–510): on `VK_RETURN`, unless the filter edit or preset combo has
  focus:
  - `if (ssCtrl in Shift) or (Length(SelectedModules)=0) then DoSingleModuleLoad`
  - otherwise `SimulateLoad; if btnOK.Enabled then btnOK.Click`.

  So ENTER does its work through VCL key dispatch and `KeyPreview`, not through
  dialog-manager default-button handling.
- `btnOK.Enabled := Error = ''` after validation, and an error panel is shown otherwise
  (pas L793–798). `btnOKClick` sets `ModalResult := mrNone` if the button is disabled
  (pas L676–682). **Inference:** if the plugins file yields a validation error, neither ENTER
  nor a click closes the dialog. xEdit stays modal and the batch's unbounded poll (548–550)
  never ends.
- `FormShow` focuses the module tree (`vstModules.SetFocus`) (pas L521–531). So at the time of
  the batch's ENTER, focus inside the form is on a `TVirtualStringTree`, a third-party custom
  control. Its key events reach `FormKeyDown` through `KeyPreview`.

### 1.3 Modal dialogs that can appear before Module Selection

- **Developer message:** `if not wbPatron or not xeAutoLoad then ShowDeveloperMessage`
  ([L5363–5364](https://github.com/TES5Edit/TES5Edit/blob/fd1e36020b2b5b6217e553dc0038983146a2e2dd/xEdit/xeMainForm.pas#L5363-L5364)).
  For non-patrons it shows when 14 days have passed since it last appeared, or when its
  version changes
  ([L17338–17352](https://github.com/TES5Edit/TES5Edit/blob/fd1e36020b2b5b6217e553dc0038983146a2e2dd/xEdit/xeMainForm.pas#L17338-L17352)).
  In that dialog, `btnOK` is `Caption = 'Open Patreon'`, `Default = True`, and `Close` starts
  disabled until a 5 s timer fires
  ([`xeDeveloperMessageForm.dfm` L92–147](https://github.com/TES5Edit/TES5Edit/blob/fd1e36020b2b5b6217e553dc0038983146a2e2dd/xEdit/xeDeveloperMessageForm.dfm#L92-L147)).
  `mrOk` then clicks the Patreon / Ko-fi / PayPal link.
- **What's New:** shown modally when the stored version is older, `and not xeAutoLoad`
  ([L5343](https://github.com/TES5Edit/TES5Edit/blob/fd1e36020b2b5b6217e553dc0038983146a2e2dd/xEdit/xeMainForm.pas#L5343)).
- **Inference:** a title-blind or focus-blind ENTER can dismiss one of these instead, or open
  a browser, and leave Module Selection waiting.

### 1.4 How xEdit closes, and what that means for `WM_CLOSE`

- `xEdit.dpr` sets `Application.MainFormOnTaskbar := True`
  ([L136](https://github.com/TES5Edit/TES5Edit/blob/fd1e36020b2b5b6217e553dc0038983146a2e2dd/xEdit.dpr#L136)).
  **Inference:** the main form is then the process's unowned top-level window, which is what
  .NET's main-window heuristic picks (§4.1). Confirming the actual owner chain is
  [Open fact F3](#open-facts).
- `TfrmMain.FormClose` waits for the background loader. It then calls `SaveChanged` and aborts
  the close if that returns `>= srAbort`, and calls `SaveLogs`
  ([L6489–6560](https://github.com/TES5Edit/TES5Edit/blob/fd1e36020b2b5b6217e553dc0038983146a2e2dd/xEdit/xeMainForm.pas#L6489-L6560)).
  In `wbAutoModes`, including `tmScript`, `SaveChanged` does **not** show the "Save changed
  files" dialog
  ([L16323](https://github.com/TES5Edit/TES5Edit/blob/fd1e36020b2b5b6217e553dc0038983146a2e2dd/xEdit/xeMainForm.pas#L16323)).
  **Inference:** a `WM_CLOSE` reaching the main form after the script finishes should close
  xEdit without a prompt. The source does not show why the batch says `CloseMainWindow`
  "sometimes fails" ([Open fact F4](#open-facts)).
- xEdit itself reads a session log path only from `-R:`
  ([`xeInit.pas` L612](https://github.com/TES5Edit/TES5Edit/blob/fd1e36020b2b5b6217e553dc0038983146a2e2dd/xEdit/xeInit.pas#L612)).
  `-log:` does not appear in `xeInit.pas` or `xeMainForm.pas`. **Inference:** the
  `UnattendedScript.log` the batch polls for is written by the PJM `.pas` scripts, not by
  xEdit's own close path. Not verified ([Open fact F5](#open-facts)).

### 1.5 Does Module Selection honour `WM_KEYDOWN` / `BM_CLICK` / `WM_COMMAND` / UIA Invoke?

The xEdit source settles what xEdit does once VCL dispatches an event (§1.2). It does not
settle how VCL routes injected messages. That depends on Embarcadero's VCL runtime
(`Vcl.Controls`, `Vcl.StdCtrls`). Its source ships with Delphi, but it is not published at a
URL that can be cited here, and the Embarcadero docwiki returned HTTP 403 to fetches.

| Mechanism | What is established | What is not |
|---|---|---|
| `PostMessage(WM_KEYDOWN, VK_RETURN)` to the focused tree | Posted keyboard messages skip the `WH_KEYBOARD` hook and do not update key state, so `GetKeyState`/`GetAsyncKeyState` see the real state ([Chen 2005](https://devblogs.microsoft.com/oldnewthing/20050530-11/?p=35513), [Chen 2025](https://devblogs.microsoft.com/oldnewthing/20250319-00/?p=110979)). Benign for the plain-ENTER branch, but a real held Ctrl changes the branch (§1.2). | Whether VCL's `KeyPreview` path fires for a posted (not input-queue) `WM_KEYDOWN`: F1 |
| `BM_CLICK` to `btnOK` | Generates `WM_LBUTTONDOWN`/`UP` on the button and `BN_CLICKED` to its parent. "If the button is in a dialog box and the dialog box is not active, the BM_CLICK message might fail" ([BM_CLICK](https://learn.microsoft.com/en-us/windows/win32/controls/bm-click)). The suggested remedy, `SetActiveWindow`, only works for windows "attached to the calling thread's message queue" ([SetActiveWindow](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setactivewindow)), so it is unavailable cross-process without `AttachThreadInput`. **This path skips `FormKeyDown`'s `SimulateLoad` call**, though `FormShow` already ran it (pas L528). | Whether VCL `TButton` turns the parent's `BN_CLICKED` into `Click` → `ModalResult` for a synthetic click on an inactive dialog: F2 |
| `WM_COMMAND`/`BN_CLICKED` posted to the form | Standard Win32 notification shape | Whether VCL's form `WM_COMMAND` handler forwards it to the `TButton`: F2 |
| UIA `Invoke` on `btnOK` | Windows supplies UIA proxies for standard Win32 controls, chosen by class name, with an MSAA proxy fallback for any window ([Proxy factory mapping](https://learn.microsoft.com/windows/win32/winauto/uiauto-clientsideprovider#proxy-factory-mapping); [IAccessible proxies](https://learn.microsoft.com/windows/win32/winauto/iaccessible-proxies)). The Oleacc proxies are class-name based and say subclassed controls are an exception ([same](https://learn.microsoft.com/windows/win32/winauto/iaccessible-proxies#what-information-is-exposed)). `Invoke` "should return immediately without blocking. However, this behavior depends on the implementation" ([IUIAutomationInvokePattern::Invoke](https://learn.microsoft.com/windows/win32/api/uiautomationclient/nf-uiautomationclient-iuiautomationinvokepattern-invoke)). | VCL buttons are superclassed `BUTTON` controls registered under a VCL class name (widely observed as `TButton`, but not from a citable source). Whether the proxy recognises it and exposes Invoke, and what Invoke does internally: F2 |
| Finding the dialog | `Caption = 'Module Selection'`. Owned by the main form (§1.2) | Its window class name (expected `TfrmModuleSelect`) and owner chain: F3 |

---

## 2. Windows foreground rules, and how `AppActivate` / `SendKeys` / `SendInput` relate

### 2.1 `SetForegroundWindow`

All quoted from [SetForegroundWindow](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setforegroundwindow).
A process can set the foreground window only if:

- **all** of these hold:
  - it is a desktop app;
  - the foreground process has not called `LockSetForegroundWindow`;
  - no menus are active;
- **and at least one** of these holds:
  - the foreground lock time-out (`SPI_GETFOREGROUNDLOCKTIMEOUT`) has expired;
  - the caller *is* the foreground process;
  - the caller *was started by* the foreground process;
  - there is no foreground window;
  - the caller received the last input event;
  - the caller or the foreground process is being debugged.

The page also says:

- "It is possible for a process to be denied the right … even if it meets these conditions."
- "An application cannot force a window to the foreground while the user is working with
  another window. Instead, Windows flashes the taskbar button."

`SetForegroundWindow` makes the target's input queue the foreground queue immediately. The
activation inside the target is **asynchronous** when the target is in another thread group.
Chen's recommended way to wait for it is `SendMessageTimeout(WM_NULL)`, not `AttachThreadInput`
([Chen 2016](https://devblogs.microsoft.com/oldnewthing/20161118-00/?p=94745)).

### 2.2 `AllowSetForegroundWindow`, `LockSetForegroundWindow`

- `AllowSetForegroundWindow(pid)` only works if the **caller** can already set the foreground.
  The grant is lost at the next user input not directed at that process, or at the next
  `AllowSetForegroundWindow` call for a different process
  ([AllowSetForegroundWindow](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-allowsetforegroundwindow)).
- `LockSetForegroundWindow` lets the foreground process disable others' `SetForegroundWindow`.
  The system re-enables it when the user presses ALT or the system itself changes the
  foreground
  ([LockSetForegroundWindow](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-locksetforegroundwindow)).

### 2.3 `AttachThreadInput`

- It shares input state (key state, focus window) between two threads. It fails if either
  thread has no message queue or a journal-record hook is installed. Key state is reset after
  the call. It cannot cross desktops
  ([AttachThreadInput](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-attachthreadinput)).
- Microsoft does not document it as a way around the foreground rules. Chen calls it a "Get
  Into the Same Jail" card: if the target stops responding, the attached caller stops too, and
  `SetForegroundWindow` can hang
  ([Chen 2016](https://devblogs.microsoft.com/oldnewthing/20161118-00/?p=94745);
  [Chen 2008 "dangers of attaching input queues"](https://devblogs.microsoft.com/oldnewthing/20080801-00/?p=21393)).

### 2.4 `SendInput`, and the console-has-focus case

- `SendInput` inserts events into the system keyboard/mouse input stream. It takes **no target
  window**
  ([SendInput](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-sendinput)).
- "The system posts keyboard messages to the message queue of the foreground thread that
  created the window with the keyboard focus"
  ([Keyboard Input Overview](https://learn.microsoft.com/windows/win32/inputdev/about-keyboard-input#keyboard-focus-and-activation)).
  **So when the console window has focus, a synthesized ENTER goes to the console, not to
  xEdit.** `SendInput` reaches a window this process does not own only if that window is the
  foreground focus window at the moment the event is processed.
- `SendInput` is subject to UIPI: it can only inject into processes at equal or lower integrity.
  A UIPI block is not reported by the return value or by `GetLastError`. It does not reset
  current key state, so a physically held key can interfere ([SendInput](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-sendinput)).
- Chen recommends UI Automation first and `SendInput` second, not posted messages
  ([Chen 2025](https://devblogs.microsoft.com/oldnewthing/20250319-00/?p=110979)).

### 2.5 `WScript.Shell.AppActivate` / `SendKeys`

- `AppActivate` "changes the focus to the named application or window". It returns `False` if
  the window is not brought to the foreground, or is brought there without keyboard focus. The
  method "can return **False** when an internal call to SetForegroundWindow succeeds but an
  internal call to SetFocus fails"
  ([AppActivate Method](https://learn.microsoft.com/en-us/previous-versions/windows/internet-explorer/ie-developer/windows-scripting/wzcddbek(v=vs.84))).
  **It is built on `SetForegroundWindow` and inherits §2.1.** Microsoft documents no bypass.
- `SendKeys` "sends one or more keystrokes to the active window (as if typed on the keyboard)",
  with `{ENTER}` or `~` for ENTER
  ([SendKeys Method](https://learn.microsoft.com/en-us/previous-versions/windows/internet-explorer/ie-developer/windows-scripting/8c6yea83(v=vs.84))).
  It is untargeted, like `SendInput`. Its internal mechanism (`SendInput` vs. `keybd_event`)
  is not documented.
- Whether the batch's `powershell.exe` actually *qualifies* under §2.1 is not documented. It is
  a child of `cmd.exe`, and its console window is hosted by conhost or Windows Terminal. The
  qualifying rules are "started by the foreground process" and "received the last input". In
  practice this decides whether the batch's `AppActivate` works: [Open fact F6](#open-facts).

---

## 3. Alternatives that do not need focus

These are listed in §1.5 against xEdit specifically. The general facts:

- **Posted keyboard messages** skip the input queue and the `WH_KEYBOARD` hook. They leave key
  state unchanged. Microsoft calls this unreliable
  ([Chen 2005](https://devblogs.microsoft.com/oldnewthing/20050530-11/?p=35513);
  [Chen 2025](https://devblogs.microsoft.com/oldnewthing/20250319-00/?p=110979)).
- **`PostMessage`** is subject to UIPI: a process can only post to processes at lower or equal
  integrity. A UIPI block sets last error 5
  ([PostMessageW](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-postmessagew)).
  Pointers cannot travel in posted system messages (same page). `WM_CLOSE`, `BM_CLICK` and
  `WM_KEYDOWN` carry none.
- **`BM_CLICK`** caveats: see §1.5.
- **UI Automation Invoke:** buttons that perform a command support the Invoke pattern
  ([Button Control Type](https://learn.microsoft.com/windows/win32/winauto/uiauto-supportbuttoncontroltype#required-control-patterns)).
  Win32 controls get it through client-side proxies, and an MSAA proxy is the fallback for any
  window ([proxy table](https://learn.microsoft.com/windows/win32/winauto/uiauto-clientsideprovider#proxy-factory-mapping)).

---

## 4. Closing: `CloseMainWindow`, `WM_CLOSE`, `taskkill`, termination

### 4.1 What `Process.CloseMainWindow()` does

- Documented: it sends "a close message to its main window". It returns `false` "if the
  associated process does not have a main window or if the main window is disabled (for
  example if a modal dialog is being shown)". It "does not force the application to quit",
  and behaves like closing the main window from the system menu
  ([Process.CloseMainWindow](https://learn.microsoft.com/en-us/dotnet/api/system.diagnostics.process.closemainwindow)).
- Implementation, identical in .NET (PowerShell 7) and .NET Framework (Windows PowerShell 5.1):
  - read `MainWindowHandle`;
  - `if (GetWindowLong(h, GWL_STYLE) & WS_DISABLED) return false;`
  - `PostMessage(h, WM_CLOSE, 0, 0); return true;`
  - Sources: [`Process.Windows.cs` L589–609](https://github.com/dotnet/runtime/blob/c94f6527f3cefcaa86dbe1669aa045588d8de18d/src/libraries/System.Diagnostics.Process/src/System/Diagnostics/Process.Windows.cs#L589-L609);
    [referencesource `Process.cs` L1239–1246](https://github.com/microsoft/referencesource/blob/master/System/services/monitoring/system/diagnosticts/Process.cs#L1239-L1246).

  `true` means only "posted".
- `MainWindowHandle` is the **first** top-level window, found with `EnumWindows` and filtered by
  `GetWindowThreadProcessId == pid`, that has **no owner** (`GetWindow(GW_OWNER) == 0`) and is
  **visible**
  ([`ProcessManager.Windows.cs` L355–379](https://github.com/dotnet/runtime/blob/c94f6527f3cefcaa86dbe1669aa045588d8de18d/src/libraries/System.Diagnostics.Process/src/System/Diagnostics/ProcessManager.Windows.cs#L355-L379)).
  "There is no formal definition of a 'main window'… It's a synthetic property"
  ([Chen 2022](https://devblogs.microsoft.com/oldnewthing/20220124-00/?p=106192)).
- **Win32 equivalent:** `EnumWindows` + `GetWindowThreadProcessId` + `GetWindow(GW_OWNER)` +
  `IsWindowVisible` + `GetWindowLong(GWL_STYLE)` + `PostMessage(WM_CLOSE)`.
  - `EnumWindows` covers top-level windows only, and on Windows 8+ only those of desktop apps
    ([EnumWindows](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-enumwindows)).
  - Child controls need `EnumChildWindows`.
- **Inference for xEdit:** while Module Selection (or any `ShowModal` form) is up, VCL has the
  main form disabled, so `CloseMainWindow` returns `false` and posts nothing. Neither the VCL
  modal-disable nor the actual xEdit window states have been observed: F3.

### 4.2 Termination

- `TerminateProcess` "unconditionally" ends the process. It is **asynchronous**: "If you need
  to be sure the process has terminated, call WaitForSingleObject". It needs
  `PROCESS_TERMINATE`. After exit, further calls fail with `ERROR_ACCESS_DENIED`
  ([TerminateProcess](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-terminateprocess)).
- No save or close handlers run, which means no xEdit `FormClose` → `SaveChanged`/`SaveLogs`
  (§1.4). **Inference:** a kill that lands before xEdit's own close can drop xEdit's auto-save
  of modified plugins.

### 4.3 `taskkill /IM` vs. by PID

- `/im <imagename>` selects every process with that image name. `/pid` selects one PID. `/f`
  "Specifies that processes be forcefully ended"
  ([taskkill](https://learn.microsoft.com/en-us/windows-server/administration/windows-commands/taskkill)).
  Microsoft does not document what a non-`/f` local taskkill does. Wine's implementation posts
  `WM_CLOSE` to every top-level window of each matched process (§6).
- **The batch's line 556 has no `/F`.** Its "last ditch kill" is therefore another close
  request, not `TerminateProcess`. With `/IM` it also hits any other `FO4Edit.exe` the user has
  open. Killing by the PID of the spawned child (Rust `Child::id()` or the handle, §5) is
  scoped to this run.

---

## 5. Rust process side

- `Command::spawn` returns a `Child` without waiting. That is the async start `START /B`
  provides.
  - `Child` has no `Drop` that kills or waits: "it will continue to run even after the `Child`
    handle goes out of scope".
  - `id()` returns the OS PID.
  - `try_wait()` polls without blocking.
  - `kill()` "forces the child process to exit", and returns `Ok(())` if it already exited.
  - `Child` implements `AsRawHandle` / `AsHandle` / `IntoRawHandle` / `From<Child> for
    OwnedHandle` on Windows.
  - All from [`std::process::Child`](https://doc.rust-lang.org/std/process/struct.Child.html).
- On Windows, `Child::kill` is `TerminateProcess(handle, 1)`. It treats `ERROR_ACCESS_DENIED`
  on an already-exited process as success
  ([`library/std/src/sys/process/windows.rs`](https://github.com/rust-lang/rust/blob/dba8825fe50879b22129271fb865944e384f7cce/library/std/src/sys/process/windows.rs)).
  `std` has **no** graceful-close or window API.
- **Window enumeration and messaging need a Win32 binding.** In the `windows` crate 0.62.2,
  every one of these is an `unsafe fn`:
  - [`EnumWindows`](https://microsoft.github.io/windows-docs-rs/doc/windows/Win32/UI/WindowsAndMessaging/fn.EnumWindows.html)
    (`pub unsafe fn EnumWindows(lpenumfunc: WNDENUMPROC, lparam: LPARAM) -> Result<()>`)
  - [`PostMessageW`](https://microsoft.github.io/windows-docs-rs/doc/windows/Win32/UI/WindowsAndMessaging/fn.PostMessageW.html)
  - [`SendInput`](https://microsoft.github.io/windows-docs-rs/doc/windows/Win32/UI/Input/KeyboardAndMouse/fn.SendInput.html)
  - [`IUIAutomationInvokePattern::Invoke`](https://microsoft.github.io/windows-docs-rs/doc/windows/Win32/UI/Accessibility/struct.IUIAutomationInvokePattern.html)

  The `EnumWindows` callback is itself an `extern "system"` function pointer.
- **This crate forbids unsafe code.** `Cargo.toml` sets `[lints.rust] unsafe_code = "forbid"`.
  The lint flags any `unsafe` block
  ([unsafe-code lint](https://doc.rust-lang.org/rustc/lints/listing/allowed-by-default.html)).
  `forbid` "can not be overridden to be anything lower than an error"
  ([lint levels](https://doc.rust-lang.org/rustc/lints/levels.html)). Lints apply per crate,
  so `unsafe` inside a *dependency* does not trip it.
  - The `winsafe` crate (0.0.29) wraps some calls as safe:
    [`EnumWindows`](https://docs.rs/winsafe/latest/winsafe/fn.EnumWindows.html) (closure-based),
    `HWND::GetWindowThreadProcessId`, `SetForegroundWindow`, `GetClassName`, `GetWindowText`,
    `GetWindow` and `IsWindowVisible`.
  - It still marks `HWND::PostMessage` and `SendMessage` `unsafe`
    ([HWND](https://docs.rs/winsafe/latest/winsafe/struct.HWND.html)).
  - `docs/technical.md` currently lists "`windows` or `winapi`".

---

## 6. Wine / Proton support

Source: Wine `wine-11.18` (W) and Proton's `proton_11.0` branch (P), checked where noted.

| Mechanism | Wine/Proton status | Evidence |
|---|---|---|
| `powershell.exe` | **Stub.** Logs `FIXME stub`, consumes `-command -` stdin, returns 0. It runs nothing, so the batch's lines 546 and 553 are silent no-ops. Same in P. | [W `programs/powershell/main.c` L23–48](https://github.com/wine-mirror/wine/blob/wine-11.18/programs/powershell/main.c#L23-L48) |
| PowerShell via winetricks | `winetricks powershell_core` installs PowerShell 7.4.11 MSI. `winetricks powershell` adds a third-party wrapper (ProjectSynchro) as `powershell.exe`. | [winetricks `src/winetricks` @ f3890f6, `load_powershell_core` / `load_powershell`](https://github.com/Winetricks/winetricks/blob/f3890f670867b5ffbc3938726db45c0f7d16c8ba/src/winetricks) |
| `WScript.Shell.AppActivate` / `SendKeys` | **Stubs returning `E_NOTIMPL`** in W and P. | [W `dlls/wshom.ocx/shell.c` L1821–1831](https://github.com/wine-mirror/wine/blob/wine-11.18/dlls/wshom.ocx/shell.c#L1821-L1831) |
| …with `winetricks wsh57` | That verb overrides `jscript scrrun vbscript cscript.exe wscript.exe` and registers seven DLLs. **`wshom.ocx` (which implements `WScript.Shell`) is in neither list.** Whether native `wshom.ocx` takes over is unverified (F7). | [winetricks `load_wsh57`](https://github.com/Winetricks/winetricks/blob/f3890f670867b5ffbc3938726db45c0f7d16c8ba/src/winetricks) |
| `SendInput` | Implemented: keyboard/mouse inputs go to the wineserver as injected hardware messages and are delivered to the Wine foreground/focus window. `INPUT_HARDWARE` returns `ERROR_CALL_NOT_IMPLEMENTED`. | [W `dlls/win32u/input.c` L759–805](https://github.com/wine-mirror/wine/blob/wine-11.18/dlls/win32u/input.c#L759-L805) |
| `SetForegroundWindow` | Implemented. Since **Wine 10.20** (commit `f5944829dc` "server: Forbid background process window reactivation"; refined in 11.1) the server rejects it with `ACCESS_DENIED` when all of these hold: (a) the window has already been made foreground once, (b) the caller is not the foreground process or its child, (c) the caller's last input is older than the foreground's. Every window may be made foreground once. Before 10.20 there was no such check. Present in P (`server/queue.c` L3970). | [W `dlls/win32u/input.c` L2326–2373](https://github.com/wine-mirror/wine/blob/wine-11.18/dlls/win32u/input.c#L2326-L2373); [W `server/queue.c` L1958–1962, L3791–3823](https://github.com/wine-mirror/wine/blob/wine-11.18/server/queue.c#L3791-L3823); [W `server/window.c` L791–802](https://github.com/wine-mirror/wine/blob/wine-11.18/server/window.c#L791-L802); [commit history](https://github.com/wine-mirror/wine/commits/wine-11.18/server/queue.c) |
| `AllowSetForegroundWindow` / `LockSetForegroundWindow` | **No-ops returning `TRUE`** ("FIXME: If Win98/2000 style SetForegroundWindow behavior is implemented, then fix this function"). | [W `dlls/user32/win.c` L777–797](https://github.com/wine-mirror/wine/blob/wine-11.18/dlls/user32/win.c#L777-L797) |
| `AttachThreadInput` | Not separately inspected. Wine's server tracks per-thread `thread_input` objects (`server/queue.c`), but the behaviour was not checked. | F8 |
| `PostMessage`, `EnumWindows`, `GetWindowThreadProcessId` | Implemented. Wine's own `taskkill` relies on all three. | [W `programs/taskkill/taskkill.c` L118–132](https://github.com/wine-mirror/wine/blob/wine-11.18/programs/taskkill/taskkill.c#L118-L132) |
| `BM_CLICK` | Implemented in both builtin button classes: `SendMessage(WM_LBUTTONDOWN)`, then `WM_LBUTTONUP`. LBUTTONDOWN takes capture and focus. LBUTTONUP sends `BN_CLICKED` to the parent if the pushed state is set. | [W `dlls/user32/button.c` L264–300, L428–431](https://github.com/wine-mirror/wine/blob/wine-11.18/dlls/user32/button.c#L264-L300); same code in `dlls/comctl32_v6/button.c` |
| UI Automation Invoke | **Not available for Win32 buttons.** Wine's MSAA→UIA provider returns only `LegacyIAccessible` from `GetPatternProvider` (others: `FIXME Unimplemented patternId`), and its `DoDefaultAction` is a stub. The base HWND provider's `GetPatternProvider` is a stub. Wine's oleacc marks the `Button` window class as a stub. | [W `dlls/uiautomationcore/uia_provider.c` L637–650, L1250–1254, L1648–1651](https://github.com/wine-mirror/wine/blob/wine-11.18/dlls/uiautomationcore/uia_provider.c#L637-L650); [W `dlls/oleacc/client.c` L873–874](https://github.com/wine-mirror/wine/blob/wine-11.18/dlls/oleacc/client.c#L873-L874) |
| `taskkill` | Implemented, with `/IM` and `/PID` (treated identically) plus `/T` and `/F`. **Without `/F` it posts `WM_CLOSE` to every top-level window of each matched process.** With `/F` it calls `TerminateProcess(process, 1)`. Same in P. | [W `programs/taskkill/taskkill.c` L118–132, L296–330, L351, L425–436, L493–496](https://github.com/wine-mirror/wine/blob/wine-11.18/programs/taskkill/taskkill.c#L296-L330) |
| `TerminateProcess` / Rust `Child::kill` | Implemented (used by Wine `taskkill /F`). | as above |

**Wine-specific caveat on focus.** When the Rust binary runs from a host terminal (a native
Linux window, not a Wine window), Wine's idea of the "foreground" is separate from the host
window manager's. Wine sets a NULL foreground input when switching to the desktop window (commit
`dc07b689a1`). Whether xEdit's dialog is the Wine foreground, which decides where `SendInput`
lands, is not established from source: F9.

---

## Open facts

Each could become a `task` ticket. Each names the experiment that would settle it.

| ID | Question | Experiment on a real machine |
|---|---|---|
| F1 | Does a **posted** `WM_KEYDOWN`/`WM_KEYUP` `VK_RETURN` to the focused `TVirtualStringTree` (or to the form) reach `TfrmModuleSelect.FormKeyDown` through VCL `KeyPreview`? | Start FO4Edit 4.1.5q with the batch's flags. Once "Module Selection" is up, run a tiny tool that finds it (`EnumWindows` + title), finds the focused child (`GetGUIThreadInfo`), and posts the key pair. Observe whether loading starts. Repeat with the console focused. |
| F2 | Do `BM_CLICK` to `btnOK`, `WM_COMMAND(BN_CLICKED)` to the form, and UIA `Invoke` on the OK button each close the dialog with `mrOk` **while the dialog is not the active window**? | Same setup with the console focused. Enumerate children with `EnumChildWindows`, pick the one titled "OK", and try each mechanism separately. For UIA, also record whether the Invoke pattern is offered at all (Inspect.exe or `IUIAutomation::ElementFromHandle`). |
| F3 | Window classes and owner chain: the main form's and dialog's `GetClassName`; `GetWindow(GW_OWNER)`; `WS_DISABLED` on the main form while the dialog is modal; which window .NET picks as `MainWindowHandle` before and after the dialog. | Spy++ or a small `EnumWindows` dump of the xEdit PID at each stage. |
| F4 | Why does `CloseMainWindow()` "sometimes fail" at line 553 when `tmScript` saves without prompting? Candidates are an error or message dialog still modal, the background loader still running, or the main window chosen differently. | Run both PJM scripts and dump window list and states (F3 tool) at the moment line 553 would fire. Record `CloseMainWindow()`'s return value and whether the process exits within 15 s. |
| F5 | Who writes `-log:"…UnattendedScript.log"`: the PJM `.pas` scripts, or xEdit? And at what point relative to script completion? | Read `Batch_FO4MergeCombinedObjectsAndCheck.pas` / `Batch_FO4MergePrevisandCleanRefr.pas` (not in this repo), or watch file creation time against xEdit's message log. |
| F6 | On Windows 11, does the batch's `AppActivate('Module Selection')` from `powershell.exe` launched by `cmd.exe` actually succeed (return `True`)? Under conhost and under Windows Terminal? Does a Rust console process spawned the same way pass §2.1's rules? | Print `AppActivate`'s return value and `GetForegroundWindow()` after the call, under both terminal hosts, with and without the user typing in between. |
| F7 | Under Wine with `winetricks wsh57`, does `New-Object -ComObject WScript.Shell` resolve to native or builtin `wshom.ocx`? | In a prefix with `wsh57` and `powershell`, call `AppActivate` and check for the `E_NOTIMPL` FIXME in `WINEDEBUG=fixme+wshom` output. |
| F8 | Wine `AttachThreadInput` behaviour across processes. | Read `NtUserAttachThreadInput` / server `attach_thread_input`, or test. |
| F9 | Under Wine/Proton, launched from a host terminal, is xEdit's Module Selection the Wine foreground when it appears? Does a `SendInput` ENTER from the Rust process reach it? Does `SetForegroundWindow` on it succeed given the 10.20+ restriction? | Run under Wine 11.x and Proton 11.0 from a Linux terminal. Log `GetForegroundWindow()` and the `SetForegroundWindow` result (`WINEDEBUG=+win`). |
| F10 | Will the next xEdit release contain commit `41ab0d5`? If so, are `-autoload` + `-autoexit` in a `-Script:` run a full replacement for the ENTER and close steps? That needs the same modules loaded as the dialog's checked set, the Ctrl-held exception noted, and the developer message (non-patron) still shown. | Build or obtain `dev-4.1.6` and run the batch command line with `-autoload`, on Windows and Wine. |
