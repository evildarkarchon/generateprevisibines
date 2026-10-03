# Key Behaviors to Preserve

## From Original Script

Line references are against the **V2.99** batch (`GeneratePrevisibines.bat`, committed at
the repo root, 564 lines, header `PJM V2.99 Aug 2026`) — the same file [episodes.md](episodes.md)
cites.

- **Reserved Plugin Names**: "previs", "combinedobjects", "xprevispatch" are forbidden (`:CheckPluginName` lines 174-182)
- **Space Check**: Plugin names cannot contain spaces in clean or xbox mode — only filtered mode skips the check (lines 161-163, error path `:SpaceInName` 183-186). V2.98 checked clean mode only; the port's `validation` follows V2.99 (since #34)
- **8-Step Resume**: Users can restart from any step 1-8 after failures (`:GetStep` lines 214-235)
- **CKPE Config Checking**: Must validate `bBSPointerHandleExtremly=true` setting exists
- **Multiple Config Locations**: CKPE may use .toml, .ini, or fallout4_test.ini with different setting names
- **Version Display**: Show version info for FO4Edit, Fallout4.exe, CreationKit, CKPE
- **Error Messages**: Match original error messages for user familiarity

## Host Support

- **Wine/Proton is a supported host.** The batch header has advertised "Support for Wine" since
  V2.98 (V2.99 line 15), and the port keeps it. The batch implements that support as a `WHERE /Q reg.exe` probe (line 21) whose
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
- **Run endings**: a stopped Workflow Run prints `ERROR - <error>`, `Build of Patch <name> failed.` (batch 376) and `See Log at <log>` (368), in that order, and appends the first two to the session log (an additive divergence: the batch only echoes them). A completed run ends on `See Log at`. Build Warnings print and log as `WARNING - <text>` when raised. Errors raised before a Workflow Run exists print `ERROR - …` only, since there is no session log yet
- **Exit codes** (new; the batch set none that meant anything): `0` when the run reached the end of its runnable steps, warnings and a deliberate intake exit included; `1` for any stop, before or during the run; `2` for command-line usage errors
- Command-line parameters: `-clean`/`-filtered`/`-xbox`, `-bsarch`, `-FO4:<dir>`, `<plugin.esp>`
