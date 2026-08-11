# External-Tool Episodes

## Purpose

This document enumerates every point at which `GeneratePrevisibines.bat` interacts with
something outside its own process. It is the **parity reference** for the Rust port: when a
slice in [future-features.md](future-features.md) claims to implement a step, this is the list
of observable behaviours it has to reproduce.

An **episode** is one interaction with something outside the process: a Creation Kit
invocation, an FO4Edit script run, an archive operation, a user prompt, a wall-clock wait, a
filesystem mutation, or a log read.

**Reader:** a maintainer implementing or reviewing one workflow step.
**After reading:** you know the exact command line, pre-checks, post-checks, log predicates,
severities and delays for that step, and which of them differ between build modes and archive
tools.

Line references are against the **V2.98** batch (`GeneratePrevisibines.bat`, gitignored,
556 lines, header `PJM V2.98 Jun 2026`). Every citation below was verified against that file.
Where this document contradicts the older prose in [workarounds.md](workarounds.md) or
[future-features.md](future-features.md), the citations here are the current ones — see
`.scratch/workflow-operation-ports/issues/11-sync-batch-v298-references.md`.

---

## Creation Kit — four operations, one invocation shape

All four go through the single `:RunCK` subroutine (444–465):

```
START "CK" /D"<fo4dir>" /wait "CreationKit.exe" -<Operation>:"<PluginNameExt_>" <qualifiers>
```

The plugin argument is **always the patch plugin** — never `CombinedObjects.esp` or
`Previs.esp`; those are outputs.

| Operation | Step | Qualifier | Expected output | Post-run log scan |
|---|---|---|---|---|
| `GeneratePrecombined` | 1 | `clean all` / `filtered all` — BuildMode-derived (263, 266) | `CombinedObjects.esp` (fixed) | `OUT OF HANDLE ARRAY ENTRIES` (270) → **fatal** (271) |
| `CompressPSG` | 4 | *(empty)* (293) | `<plugin> - Geometry.csg` | none |
| `BuildCDX` | 5 | *(empty)* (300) | `<plugin>.cdx` | none |
| `GeneratePreVisData` | 6 | `clean all` — **hardcoded, ignores BuildMode** (310) | `Previs.esp` (fixed) | `ERROR: visibility task did not complete.` (312) → **warning** (313) |

`:RunCK` itself enforces only two things, uniformly: the expected output file exists (462),
and a non-zero exit is downgraded to a warning (463). The log scans live at the call sites —
their predicate *and severity* differ per step. Both scans are also skipped entirely when the
CK log is absent (269, 311), which jumps straight to the next step rather than failing.

Step 1 has a second, mode-conditional output check (`<plugin> - Geometry.psg`, line 264) that
is not the `:RunCK` parameter.

Full episode chain per CK run: rename six ENB/ReShade DLLs to `*-PJMdisabled` (445–450) →
delete the CK log (451) → write header and `Start <time>` to the session log (452–454) →
`START /wait` (455) → capture `ERRORLEVEL` into `Err_` (456) → write `Ended <time>` (457) →
**wait 10s** (459) → append the CK log to the session log, or note it was missing (460–461) →
output-file check (462) → exit-code warning (463).

Note that the DLLs are *not* restored inside `:RunCK` — restoration happens once, at `:Done`
(353–358). That covers every exit path that *reaches* `:Done`: success via `:Fin`, `:Failed`
(366–368), and the three `Goto Done` hard stops (249, 254, 306). It does **not** cover
`:PauseAndExit` (361), which is entered by `goto` from eleven sites and only falls through
from `:Done` on the interactive path (360). All but one of those `goto PauseAndExit` sites sit
before any CK invocation, so nothing is renamed yet; the exception is step 2's precondition
(275), reachable only when step 1's identical check at 268 already passed — i.e. on a
resume-at-step-2 where this run never renamed anything. Leftover `*-PJMdisabled` files from an
earlier crashed run are not restored on that path.

## FO4Edit — two script runs, identical shape

Both through `:RunScript` (521–556).

| Step | Script | Source plugin | Required version | Call-site criterion |
|---|---|---|---|---|
| 2 | `Batch_FO4MergeCombinedObjectsAndCheck.pas` | `CombinedObjects.esp` | V1.5 (135) | `"Error: "` **present** → warning (278–279) |
| 7 | `Batch_FO4MergePrevisandCleanRefr.pas` | `Previs.esp` | V2.3 (134) | `"Completed: No Errors."` **absent** → warning (320–321) |

Note the polarity inversion between the two — one fails on a positive match, the other on a
negative one.

Shared fatal checks inside `:RunScript`: `Error: Missing [<target>] or [<source>] modules`
(551–552) and absence of `"Completed: "` (553–554).

The launch flag set (534) is:

```
START "xEdit" /B <xEdit.exe> -fo4 -autoexit -P:"%TEMP%\Plugins.txt" <ModDir_> ^
    -Script:<script.pas> -Mod:<target plugin> -log:"%TEMP%\UnattendedScript.log"
```

The `-D:<dir>\Data` flag is **not per-call** — `ModDir_` is set once at line 489 when `-FO4:`
is passed, so both runs get it or neither does.

Full episode chain per run: write `%TEMP%\Plugins.txt` (`*<target>`, `*<source>` — 525–526) →
delete `%TEMP%\UnattendedScript.log` (528) → **wait 10s** (533) → `START /B` (534: async, no
`/wait`) → **wait 5s** (536) → PowerShell `AppActivate(<xEdit process>)`, `Start-Sleep 1`,
`AppActivate('Module Selection')`, `SendKeys {ENTER}` (537) → **poll every 5s** until the
unattended log appears (539–541) → **wait 10s** (543) → `CloseMainWindow()` (544) →
**wait 15s** (546) → `TaskKill /IM` (547) → **wait 10s** (549) → append log to session log
(550) → scan (551–554).

## Archive — verbs, and where the tools diverge

| Verb | Archive2 | BSArch |
|---|---|---|
| pack one folder | `Archive2.exe <folder> -c="<archive>" [-compression=XBox] -f=General -q`, cwd = `Data` (396) | `BSArch.exe Pack "<fo4>\BSArchTemp" "<archive>" -mt -fo4 -z`, cwd = `Data` (388) — packs a *staging dir* |
| pack multiple folders | `meshes\precombined,vis` — comma-separated (433) | implicit; staging dir already holds both |
| extract | `Archive2.exe "<archive>" -e=. -q` (407) | **never invoked** — `:Extract` (404–410) has no BSArch branch |
| append | **not supported** → extract-repack | **also not a verb** — see below |
| delete source folder | `RD /S /Q` (286, 434, 329) | never — folders were `MOVE`d into staging (387, 420) |

**BSArch has no native append.** It runs the *identical* `Pack` command in steps 3 and 8. The
difference is that step 3's BSArch path **moves** the precombined meshes into
`<fo4>\BSArchTemp\Meshes` (386–387) rather than deleting them (contrast line 286,
Archive2-only), so step 8 adds `vis` to the same tree (419–420) and repacks wholesale.
Archive2 recovers prior content *from the archive*; BSArch retains it *on disk* across steps.

On failure, both BSArch paths move the staged folder back before `goto failed` (391, 424) —
note the two targets differ (`Data\Meshes` vs `Data`), mirroring the differing `MOVE`
destinations above.

Archive2's step-8 workaround ([workarounds.md](workarounds.md) §4, V2.98 lines 428–435):
extract into `Data` (429) → **wait 5s** (430) → delete the BA2 (431) → re-scan
`meshes\precombined\*.nif` (432) → repack `meshes\precombined,vis` (433) → `RD` the
precombined folder (434).

That re-scan is a branch, not an assertion: if the extract produced no precombined `.nif`
files, control falls through to `:ArchiveOnly` (432 → 436) and the archive is rebuilt from
`vis` alone, silently dropping the precombines.

**Two divergences worth designing around:**

- **Xbox compression is Archive2-only.** `Arch2Quals_=-compression=XBox` (382) reaches the
  Archive2 command line (396) but only the *log header* on the BSArch path (383, 417) — it
  never reaches the BSArch command (388, 421). So `-xbox -bsarch` silently produces
  non-Xbox archives. Decide deliberately in Phase B whether to replicate or fix.
  `Arch2Quals_` is also only ever assigned inside `:Archive` (381–382), and the BSArch step-8
  path (417) does not call `:Archive` — so on a resume directly at step 8 even that log header
  prints empty.
- **BSArch staging state is cross-step.** `<fo4>\BSArchTemp` is cleared in exactly two
  places: line 256, inside the Step 1 preamble, and line 426, after a *successful* BSArch
  step-8 pack. So resuming at step 3 or 8 with `-bsarch` inherits whatever a run that stopped
  between step 3 and a successful step 8 left there — including the partially-moved tree from
  a failed pack (422–425, which returns only the one folder it moved).

## Waits (MO2 VFS sync)

| Duration | After which action | Line | Notes |
|---|---|---|---|
| 5s | seed copy of `xPrevisPatch.esp` — **only if** not yet visible | 191 | conditional |
| **10s** | **every Creation Kit exit** (all 4 operations) | 459 | ×4 clean, ×2 filtered/xbox |
| 10s | **before** launching xEdit | 533 | pre-launch |
| 5s | after launching xEdit, before `SendKeys` | 536 | lets the dialog appear |
| 1s | between the two `AppActivate` calls | 537 | inside the PowerShell one-liner |
| 5s | each poll iteration awaiting the unattended log | 540 | unbounded loop |
| 10s | after the log appears, before `CloseMainWindow()` | 543 | |
| 15s | after `CloseMainWindow()`, before `TaskKill` | 546 | |
| 10s | after `TaskKill`, before reading the log | 549 | |
| 5s | after Archive2 extract, before deleting the BA2 | 430 | Archive2 only |

Fixed cost: ~50s per xEdit run (×2) plus polling; 10s per CK run (×4 clean, ×2 otherwise).

## User prompts

Every prompt is reached only on the interactive path, so a non-interactive run never blocks on
input — but by two different mechanisms. Four prompts sit behind an explicit `NoPrompt_` test
that bypasses them; the other three are unreachable because the only route to them is through
a prompt of the first kind.

| Prompt | Line | Why non-interactive skips it | Non-interactive behaviour |
|---|---|---|---|
| `Enter Patch Plugin name (return to exit)` | 152 | `NoPrompt_` test at 150 | `PauseAndExit` |
| `Plugin does not exist, Rename xPrevisPatch.esp to this? [Y/N]` | 187 | `NoPrompt_` test at 185 | error, back to `:GetPlugin` → exit |
| `Plugin already exists, Use It? [Y], Exit [N], Rerun from failed step [C]` | 204 | `NoPrompt_` test at 203 | assumes **Y** — straight to step 1 |
| `Restart at step (1 - 8 or 0 to exit)` | 220 | `:GetStep` is entered only from 206 (**C**), or re-entered from 234 / 241 | unreachable |
| `Precombine directory … needs to be empty. Clean it? [Y/N]` | 233 | `:RePrecomb` is a `:GetStep` target only | unreachable |
| `Previs directory (Data\vis) needs to be empty. Clean it? [Y/N]` | 240 | `:RePreVis` is a `:GetStep` target only | unreachable |
| `Remove working files [Y]?` | 346 | `NoPrompt_` test at 345 | assumes **Y** — always cleans |

The two "clean it?" prompts are the only places the workflow deletes `Data\meshes\Precombined`
(235) or `Data\vis` (242) on the user's behalf at intake, and they exist only on the resume
path (`:RePrecomb`, `:RePreVis`).

## Per-step notes not captured above

- **Intake**: resume menu lists steps 4–5 only in clean mode (213–216); an unrecognised
  choice loops back to `:CheckPluginExists` (229).
- **Step 1 preamble**: precombined dir non-empty → `:Done` (247–249, non-resume entry only);
  archive already exists → back to `:GetPlugin` (251), i.e. re-prompt interactively and
  `PauseAndExit` otherwise; `Data\vis` non-empty → `:Done` (252–254). None of these three is
  a `:failed`. Then `RD` of `<fo4>\BSarchTemp` (256) — *only reached via step-1 entry* —
  delete `CombinedObjects.esp` (257), `- Geometry.psg` (258), and the session log (259).
- **Step 2 precondition**: no precombined meshes → `PauseAndExit`, **not** `failed` (275).
- **Step 3**: no precombined meshes → **silently skip to step 4**, not an error (283); in
  filtered/xbox mode step 4 then immediately forwards to step 6 (290).
- **Step 5**: the only step that runs CK with **zero** pre-checks (297–300 — only the
  build-mode gate).
- **Step 6**: non-resume entry with a non-empty `Data\vis` is a hard stop via `:Done`, not a
  `failed` (304–306).
- **Step 7 preconditions**: no `.uvd` files → `failed` (316); no `Previs.esp` → `failed` (317).
  Contrast step 8's warning for the same missing `.uvd` files.
- **Step 8**: no `.uvd` files → **warning** and jump to `:Fin`, not a failure (326). Missing
  `- Main.ba2` → plain `:Archive vis` (415, 436–437). On the BSArch path that fallback stages
  `vis` under `BSArchTemp\Meshes` (386–387, which hardcodes `\Meshes` for its step-3 caller),
  so the resulting BA2 holds `Meshes\vis\*.uvd` rather than `vis\*.uvd` — a latent bug in the
  batch, not a behaviour to replicate.
- **Finish**: created-files manifest lists `.csg` / `.cdx` only in clean mode (337–340). The
  "Remove working files [Y]?" prompt is **skipped when non-interactive** — meaning
  non-interactive runs always clean (345). DLL restore runs on every exit path that reaches
  `:Done`, including `:Failed` (366–368 → `:Done`, 353–358) — but not on the `:PauseAndExit`
  paths; see the `:RunCK` note above for why that is nearly always harmless.
