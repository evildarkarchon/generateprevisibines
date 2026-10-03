# Required Workarounds

These workarounds are **necessary** due to external tool limitations - DO NOT "FIX" THESE.

Line references are against the **V2.99** batch (`GeneratePrevisibines.bat`, committed at
the repo root, 564 lines, header `PJM V2.99 Aug 2026`) — the same file [episodes.md](episodes.md)
cites. None of the four workarounds changed between V2.98 and V2.99; only the line numbers
moved.

## 1. PowerShell Keystroke Automation (batch lines 543-556)

- FO4Edit has no true headless mode
- Must send ENTER keystroke to Module Selection dialog (line 546: `AppActivate` the xEdit
  process, then `AppActivate('Module Selection')` and `SendKeys('{ENTER}')`)
- Must force-close window after completion (despite `-autoexit` flag on line 543):
  `CloseMainWindow()` at 553, then `TaskKill /IM` at 556 as a last-ditch kill
- **Rust must replicate the dismiss and the close, but not by `SendInput`.** Under Wine
  the batch's PowerShell route is a silent no-op. `SendInput` is untargeted and subject to the
  foreground lock, so the port posts its ENTER to the dialog instead. xEdit saves the merged plugin only when it closes, so a hard kill would lose
  the merge. The ported mechanism (a dismissal ladder aimed at this run's Module Selection
  window, plus close requests scoped to this run's PID, never a kill) is decided on #28 and
  listed in [episodes.md](episodes.md) § *FO4Edit*. The Win32 calls live in a helper crate:
  [ADR-0003](adr/0003-win32-window-calls-in-a-helper-crate.md)

## 2. MO2 Timing Delays (batch lines 197, 439, 468, 542, 545, 549, 552, 555, 558)

- Mod Organizer 2 virtual file system introduces real timing issues
- 5-15 second delays are required for VFS synchronization
- Per-site durations are tabulated in [episodes.md](episodes.md) § *Waits (MO2 VFS sync)*;
  in short: 5s after the seed copy (197), 5s after an Archive2 extract (439), 10s after every
  Creation Kit exit (468), and around each xEdit run 10s before launch (542), 5s before the
  keystroke (545), then 10s / 15s / 10s at 552, 555 and 558. The 5s at 549 is **not** part of
  that sequence — it is one iteration of the unbounded `:loop` poll (548-550) awaiting the
  unattended log, so its total cost scales with how long the script takes
- **Keep these delays; they're not arbitrary**
- The port adds **5s before every BSArch pack** (Steps 3 and 8), which the batch's BSArch path
  never had. BSArch doesn't write to `Data` itself, but the files it packs are moved out of MO2's
  virtual `Data` into the `<fo4>\ArchiveWork\staging` folder. If they haven't settled when BSArch reads that folder,
  the archive comes out incomplete (decided on #29).

## 3. DLL Renaming (batch lines 454-459, 362-367)

- CreationKit crashes with ENB/ReShade DLLs loaded
- Must rename d3d11.dll, d3d10.dll, d3d9.dll, dxgi.dll, enbimgui.dll and d3dcompiler_46e.dll
  to `.dll-PJMdisabled` — six files, disabled on entry to `:RunCK` (454-459)
- Must restore after CK exits. Restoration is **not** inside `:RunCK`; it happens once at
  `:Done` (362-367), which every exit path reaching `:Done` shares — see
  [episodes.md](episodes.md) § *Creation Kit* for the paths it does not cover
- **Preserve this exactly**

## 4. Archive2 Extract-Repack (batch lines 413-419, 437-444)

- Archive2.exe has no append functionality (the batch says so itself at line 422)
- Must extract, add files, re-archive: `:Extract` (413-419) runs
  `Archive2.exe "<archive>" -e=. -q`; `:AddToArchive2` (437-444) then waits 5s, deletes the
  BA2, and repacks `meshes\precombined,vis`
- **This is an Archive2.exe limitation, not inefficient code**
- The port keeps extract → 5s → repack, and applies the same rebuild to BSArch through `unpack`
  (decided on #29, [ADR-0004](adr/0004-plugin-archive-is-the-only-cross-step-archive-state.md)).
  It changes only *where* the repack writes: into a work folder beside `Data`. The old archive is
  replaced only after the new one exists.
