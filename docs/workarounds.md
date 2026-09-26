# Required Workarounds

These workarounds are **necessary** due to external tool limitations - DO NOT "FIX" THESE.

Line references are against the **V2.98** batch (`GeneratePrevisibines.bat`, gitignored,
556 lines, header `PJM V2.98 Jun 2026`) — the same file [episodes.md](episodes.md) cites.

## 1. PowerShell Keystroke Automation (batch lines 534-547)

- FO4Edit has no true headless mode
- Must send ENTER keystroke to Module Selection dialog (line 537: `AppActivate` the xEdit
  process, then `AppActivate('Module Selection')` and `SendKeys('{ENTER}')`)
- Must force-close window after completion (despite `-autoexit` flag on line 534):
  `CloseMainWindow()` at 544, then `TaskKill /IM` at 547 as a last-ditch kill
- **Rust must replicate this using Windows SendInput API**

## 2. MO2 Timing Delays (batch lines 191, 430, 459, 533, 536, 540, 543, 546, 549)

- Mod Organizer 2 virtual file system introduces real timing issues
- 5-15 second delays are required for VFS synchronization
- Per-site durations are tabulated in [episodes.md](episodes.md) § *Waits (MO2 VFS sync)*;
  in short: 5s after the seed copy (191), 5s after an Archive2 extract (430), 10s after every
  Creation Kit exit (459), and around each xEdit run 10s before launch (533), 5s before the
  keystroke (536), then 10s / 15s / 10s at 543, 546 and 549. The 5s at 540 is **not** part of
  that sequence — it is one iteration of the unbounded `:loop` poll (539-541) awaiting the
  unattended log, so its total cost scales with how long the script takes
- **Keep these delays; they're not arbitrary**

## 3. DLL Renaming (batch lines 445-450, 353-358)

- CreationKit crashes with ENB/ReShade DLLs loaded
- Must rename d3d11.dll, d3d10.dll, d3d9.dll, dxgi.dll, enbimgui.dll and d3dcompiler_46e.dll
  to `.dll-PJMdisabled` — six files, disabled on entry to `:RunCK` (445-450)
- Must restore after CK exits. Restoration is **not** inside `:RunCK`; it happens once at
  `:Done` (353-358), which every exit path reaching `:Done` shares — see
  [episodes.md](episodes.md) § *Creation Kit* for the paths it does not cover
- **Preserve this exactly**

## 4. Archive2 Extract-Repack (batch lines 404-410, 428-435)

- Archive2.exe has no append functionality (the batch says so itself at line 413)
- Must extract, add files, re-archive: `:Extract` (404-410) runs
  `Archive2.exe "<archive>" -e=. -q`; `:AddToArchive2` (428-435) then waits 5s, deletes the
  BA2, and repacks `meshes\precombined,vis`
- **This is an Archive2.exe limitation, not inefficient code**
