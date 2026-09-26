# Key Behaviors to Preserve

## From Original Script

Line references are against the **V2.98** batch (`GeneratePrevisibines.bat`, gitignored,
556 lines, header `PJM V2.98 Jun 2026`) — the same file [episodes.md](episodes.md) cites.

- **Reserved Plugin Names**: "previs", "combinedobjects", "xprevispatch" are forbidden (`:CheckPluginName` lines 168-176)
- **Clean Mode Space Check**: Plugin names cannot contain spaces in clean mode (lines 155-157, error path `:SpaceInName` 177-180)
- **8-Step Resume**: Users can restart from any step 1-8 after failures (`:GetStep` lines 208-229)
- **CKPE Config Checking**: Must validate `bBSPointerHandleExtremly=true` setting exists
- **Multiple Config Locations**: CKPE may use .toml, .ini, or fallout4_test.ini with different setting names
- **Version Display**: Show version info for FO4Edit, Fallout4.exe, CreationKit, CKPE
- **Error Messages**: Match original error messages for user familiarity

## Host Support

- **Wine/Proton is a supported host.** V2.98's header advertises "Support for Wine", and the
  port keeps it. The batch implements that support as a `WHERE /Q reg.exe` probe (line 21) whose
  `RegErr_` result guards both registry lookups (lines 38 and 49).
- The port does **not** reproduce the probe itself. `WHERE /Q reg.exe` only proves the binary is
  on `PATH`, so a Wine prefix carrying a stub `reg.exe` passes it and then answers nothing
  useful. `src/discovery.rs` instead treats *any* unanswerable registry — missing key, missing
  registry, blank value — as the batch's empty result:
  - **xEdit** (batch 38) falls through to the same "FO4Edit/xEdit directory not found…" error as
    `:xEditCheckFail`.
  - **Fallout 4** (batch 49) leaves the directory unset and falls through to a message carrying
    both of batch line 63's remedies: run `Fallout4Launcher.exe` once, or pass `--FO4 <DIR>`.
    Line 63's `Exist` test on `Fallout4.exe` itself has no counterpart yet, so the port's wording
    reports the undetermined *directory*; a stale or typo'd directory is caught one step later by
    the `CreationKit.exe` check (batch line 80).
- This also covers plain Windows where `Fallout4Launcher.exe` has never been run: the HKLM key
  is simply absent, and the user gets the two remedies rather than a raw registry error.

## UX Expectations

- Interactive prompts using CHOICE-style Y/N confirmations
- Ability to run non-interactively with plugin name parameter
- Clear step-by-step progress messages
- Comprehensive logging to temp file
- Command-line parameters: `-clean`/`-filtered`/`-xbox`, `-bsarch`, `-FO4:<dir>`, `<plugin.esp>`
