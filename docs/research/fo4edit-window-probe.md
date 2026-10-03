# FO4Edit window probe on Windows: results

Results for [Probe FO4Edit's windows and dismissal on Windows](https://github.com/evildarkarchon/generateprevisibines/issues/40)
(map #23). The open facts F1–F6 refer to
[the FO4Edit window research](https://github.com/evildarkarchon/generateprevisibines/blob/research/fo4edit-window-automation/docs/research/fo4edit-window-automation.md#open-facts).

**Setup.** Run 2026-10-02 on Windows 11 Pro 10.0.26300 with `FO4Edit64.exe`, whose main form
caption reads `FO4Script 4.1.5q x64`. The probe is in
[`probes/fo4edit-window-probe`](../../probes/fo4edit-window-probe). Its raw JSON reports are in
[`probes/fo4edit-window-probe/reports`](../../probes/fo4edit-window-probe/reports).

Each scenario did the following:
- launched FO4Edit with the batch's flags (`-fo4 -autoexit -P: -Script: -Mod: -log:`);
- used a probe script shaped like PJM's, against a stub plugin that already existed in `Data`;
- tried one dismissal method while Module Selection was **not** the foreground window;
- ran the FO4Edit design's close sequence;
- checked the plugin on disk after FO4Edit exited.

The human ran the matrix from T3 Code's integrated terminal (`pwsh` on ConPTY) through
`cargo run`.

## Results

| Scenario | Dialog foreground at attempt? | Dismissed by method | Close | Exit after first `WM_CLOSE` | Plugin saved |
|---|---|---|---|---|---|
| Posted ENTER, focus away | no | **yes, <100 ms** | all top-level (25) | 2.0 s, code 0 | yes |
| `BM_CLICK` on OK, focus away | no | **yes, <100 ms** | all top-level (25) | 1.9 s, code 0 | yes |
| `WM_COMMAND(BN_CLICKED)`, focus away | no | **yes, <100 ms** | visible top-level (2) | 2.1 s, code 0 | yes |
| `SetForegroundWindow` + `SendInput`, console focused | no | **no**: `SetForegroundWindow` returned `false`, so no input was sent | `CloseMainWindow` emulation (1) | 2.2 s, code 0 | yes |
| `SetForegroundWindow` + `SendInput`, focus away | no | **no**: same as above | all top-level (25) | 2.4 s, code 0 | yes |
| Posted ENTER, console focused | no | **yes, <100 ms** | all top-level (25) | 1.9 s, code 0 | yes |
| `BM_CLICK` on OK, console focused | no | **yes, <100 ms** | all top-level (25) | 1.8 s, code 0 | yes |

"Plugin saved" means three things: the marker record reached `GPProbe.esp` on disk, no
`GPProbe.esp.save.*` file was left behind, and the exit code was 0. Every run also wrote
backups to `Data\FO4Edit Backups\GPProbe.esp.backup.<timestamp>` (usually two per run).

## Answers

### F1 / F2: targeted dismissal works with Module Selection inactive

Each of these closed Module Selection with `mrOk` within the 100 ms polling interval, whether
or not the console was focused:

- `BM_CLICK` posted to the OK button;
- `WM_COMMAND(BN_CLICKED)` posted to the dialog;
- `WM_KEYDOWN`/`WM_KEYUP` `VK_RETURN` posted to the focused `TVirtualStringTree`.

FO4Edit then loaded, ran the script and wrote the log. The Microsoft caveat ("BM_CLICK might
fail if the dialog box is not active") did not show up here.

### F3: windows, classes and owner chain (4.1.5q x64)

- **Module Selection:** class `TfrmModuleSelect`, caption `Module Selection`, owned by the main
  form. While it is modal, it is the thread's active window, and its focus is a
  `TVirtualStringTree`.
- **OK button:** class `TButton`, caption `OK`, enabled.
- **Main form:** class `TfrmMain`, caption **`FO4Script 4.1.5q x64`** in script mode, unowned.
  It is **`WS_DISABLED` while Module Selection is modal** and enabled again by the time the log
  appears.
- **`TApplication` window:** unowned, `WS_VISIBLE` style, empty title. It is also disabled
  during the modal.
- **Hidden helpers** make up the rest of the PID's 25 top-level windows: `TPUtilWindow`,
  `GDI+ Hook Window Class`, `IME`, `MSCTFIME UI`, `tooltips_class32`, and a hidden
  `TfrmFileSelect` "Master/Plugin Selection".
- **.NET `MainWindowHandle`** picks `TfrmMain`. During the modal, `CloseMainWindow()` would
  return `false` without posting. At the batch's close moment (log + 10 s) it would post.
- **Side fact:** the batch's first `AppActivate('FO4Edit64')` can never match. The main
  caption is `FO4Script …`, which neither starts nor ends with the exe stem. Only the second
  call, `AppActivate('Module Selection')`, does anything.

### F4: `CloseMainWindow()` "sometimes fails"

**Not reproduced.** In all 7 runs, at the batch's line-553 moment the main window was enabled
and .NET would have posted `WM_CLOSE`, and the emulation closed FO4Edit in 2.2 s.

The only state observed that makes it fail is a disabled main window while a modal dialog is
up. Candidates for that are Module Selection never dismissed, or an error dialog.
**Inference:** the batch's "sometimes fails" is a modal dialog still being open.

### F5: who writes `-log:`, and when

**The PJM scripts write it, not xEdit.** Both `Batch_FO4MergeCombinedObjectsandCheck.pas` (V1.5)
and `Batch_FO4MergePreVisandCleanRefr.pas` (V2.3) collect lines in a `TStringList` and call
`Logfile.SaveToFile(logname)` once, at the end of `Initialize`.

- **The log means "merge done in memory", not "saved".** In every run the plugin on disk did
  **not** yet contain the marker when the log appeared. It was saved only by the close path.
- **xEdit's save is two-phase** (4.1.5q `TfrmMain.SaveChanged`, shutdown rename):
  1. A plugin that already exists on disk is written as `<plugin>.save.<timestamp>`.
  2. The rename over the real file is queued and done while the process shuts down.

  **Inference:** a kill after the log appears can leave the real plugin unchanged, with a stray
  `.save.*` beside it.
- **Both scripts cut `-mod:` and `-log:` values at 60 characters**
  (`copy(str, j+5, 60)`). `%TEMP%\UnattendedScript.log` is 54 characters on this machine. A
  user profile path about 6 characters longer would make the script write the log to a
  truncated path, and the batch's poll would never end.
- **xEdit writes `<plugins-file stem>.fo4viewsettings`** next to the `-P:` file. In the batch
  that is `%TEMP%\Plugins.fo4viewsettings`.

### F6 (Rust-child half): rung C does not pass the foreground rules here

`SetForegroundWindow(Module Selection)` returned `false` in both focus states. The foreground
stayed on the terminal host, so the probe sent no input.

Conditions:
- the probe's chain was `fo4edit-window-probe ← cargo ← rustup ← pwsh ← T3 Code`;
- the foreground process (T3 Code) is an ancestor but not the parent;
- `SPI_GETFOREGROUNDLOCKTIMEOUT` was `2147483647` ms, which is effectively never.

FO4Edit's own Module Selection never took the foreground either.

**Caveat:** this was not run from Windows Terminal or conhost, and not with the exe launched
directly. In those setups the tool is still not the foreground process, so §2.1's
"started by the foreground process" and "received the last input" conditions still fail.
**Inference:** rung C is unreliable on Windows, though harmless, because the probe checks the
foreground before sending input.

## Decision-relevant summary

1. **First rung: `BM_CLICK` on Module Selection's `OK` `TButton`.** Posted ENTER works equally
   well here, but `BM_CLICK` is preferred:
   - it does not depend on physical key state (a held Ctrl turns ENTER into
     `DoSingleModuleLoad`);
   - it needs no `GetGUIThreadInfo` focus lookup;
   - a disabled OK (a module-list error) makes it a no-op, just as for ENTER;
   - it is the same action as the human fallback ("press OK").
2. **Rung C does not work from a Rust console child on this machine.** The rung is safe because
   it is gated on the foreground check. On Windows it is effectively unreachable once rung A
   works.
3. **`WM_CLOSE` to every top-level window of this run's PID** closes FO4Edit in about 2 s with
   exit code 0, and the merged plugin is saved, with no `.save.*` left behind. Posting only to
   the visible windows, or only to the .NET main window, works too.
