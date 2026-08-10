# 10 — Persist the external-tool episode enumeration as `docs/episodes.md`

Status: ready-for-agent
Blocked by: none

Independent of Phase A, but it is the parity reference Phase B depends on and the material
`docs/future-features.md` currently only gestures at. Line references are against the
**V2.98** batch on disk.

## Work

Transcribe the enumeration below into `docs/episodes.md`, keeping the table shape. Link it
from `docs/future-features.md` (replacing the flag detail moved there by issue `08`) and from
`AGENTS.md`'s reference-documentation list. Verify each citation against the file on disk
while transcribing — do not trust this ticket blindly.

An **episode** is one interaction with something outside the process: a Creation Kit
invocation, an FO4Edit script run, an archive operation, a user prompt, a wall-clock wait, a
filesystem mutation, or a log read.

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
| `GeneratePrecombined` | 1 | `clean all` / `filtered all` — BuildMode-derived | `CombinedObjects.esp` (fixed) | `OUT OF HANDLE ARRAY ENTRIES` → **fatal** |
| `CompressPSG` | 4 | *(empty)* | `<plugin> - Geometry.csg` | none |
| `BuildCDX` | 5 | *(empty)* | `<plugin>.cdx` | none |
| `GeneratePreVisData` | 6 | `clean all` — **hardcoded, ignores BuildMode** | `Previs.esp` (fixed) | `ERROR: visibility task did not complete.` → **warning** |

`:RunCK` itself enforces only two things, uniformly: the expected output file exists (462),
and a non-zero exit is downgraded to a warning (463). The log scans live at the call sites —
their predicate *and severity* differ per step.

Step 1 has a second, mode-conditional output check (`<plugin> - Geometry.psg`, line 264) that
is not the `:RunCK` parameter.

## FO4Edit — two script runs, identical shape

Both through `:RunScript` (521–555).

| Step | Script | Source plugin | Required version | Call-site criterion |
|---|---|---|---|---|
| 2 | `Batch_FO4MergeCombinedObjectsAndCheck.pas` | `CombinedObjects.esp` | V1.5 (135) | `"Error: "` **present** → warning (278–279) |
| 7 | `Batch_FO4MergePrevisandCleanRefr.pas` | `Previs.esp` | V2.3 (134) | `"Completed: No Errors."` **absent** → warning (320–321) |

Note the polarity inversion between the two — one fails on a positive match, the other on a
negative one.

Shared fatal checks inside `:RunScript`: `Error: Missing [<target>] or [<source>] modules`
(551) and absence of `"Completed: "` (553).

The `-D:<dir>\Data` flag is **not per-call** — `ModDir_` is set once at line 489 when `-FO4:`
is passed, so both runs get it or neither does.

Full episode chain per run: write `%TEMP%\Plugins.txt` (`*<target>`, `*<source>`) → delete
`%TEMP%\UnattendedScript.log` → **wait 10s** → `START /B` (async, no `/wait`) → **wait 5s** →
PowerShell `AppActivate` + `SendKeys {ENTER}` for the Module Selection dialog → **poll every
5s** until the unattended log appears → **wait 10s** → `CloseMainWindow()` → **wait 15s** →
`TaskKill /IM` → **wait 10s** → append log to session log → scan.

## Archive — verbs, and where the tools diverge

| Verb | Archive2 | BSArch |
|---|---|---|
| pack one folder | `Archive2.exe <folder> -c="<archive>" [-compression=XBox] -f=General -q`, cwd = `Data` (396) | `BSArch.exe Pack "<fo4>\BSArchTemp" "<archive>" -mt -fo4 -z`, cwd = `Data` (388) — packs a *staging dir* |
| pack multiple folders | `meshes\precombined,vis` — comma-separated (433) | implicit; staging dir already holds both |
| extract | `Archive2.exe "<archive>" -e=. -q` (407) | **never invoked** — `:Extract` has no BSArch branch |
| append | **not supported** → extract-repack | **also not a verb** — see below |
| delete source folder | `RD /S /Q` (286, 434, 329) | never — folders were `MOVE`d into staging |

**BSArch has no native append.** It runs the *identical* `Pack` command in steps 3 and 8. The
difference is that step 3's BSArch path **moves** the precombined meshes into
`<fo4>\BSArchTemp\Meshes` rather than deleting them (contrast line 286, Archive2-only), so
step 8 adds `vis` to the same tree and repacks wholesale. Archive2 recovers prior content
*from the archive*; BSArch retains it *on disk* across steps.

Archive2's step-8 workaround (`docs/workarounds.md` §4, V2.98 lines 428–435): extract into
`Data` → **wait 5s** → delete the BA2 → re-scan `meshes\precombined\*.nif` → repack
`meshes\precombined,vis` → `RD` the precombined folder.

**Two divergences worth designing around:**

- **Xbox compression is Archive2-only.** `Arch2Quals_=-compression=XBox` (382) reaches the
  Archive2 command line (396) but only the *log header* on the BSArch path (383, 417) — it
  never reaches the BSArch command (388, 421). So `-xbox -bsarch` silently produces
  non-Xbox archives. Decide deliberately in Phase B whether to replicate or fix.
- **BSArch staging state is cross-step.** `<fo4>\BSArchTemp` is cleared only at line 256,
  inside the Step 1 preamble. Resuming at step 3 or 8 with `-bsarch` inherits whatever the
  previous run left there.

## Waits (MO2 VFS sync)

| Duration | After which action | Line | Notes |
|---|---|---|---|
| 5s | seed copy of `xPrevisPatch.esp` — **only if** not yet visible | 191 | conditional |
| **10s** | **every Creation Kit exit** (all 4 operations) | 459 | ×4 clean, ×2 filtered/xbox |
| 10s | **before** launching xEdit | 533 | pre-launch |
| 5s | after launching xEdit, before `SendKeys` | 536 | lets the dialog appear |
| 5s | each poll iteration awaiting the unattended log | 540 | unbounded loop |
| 10s | after the log appears, before `CloseMainWindow()` | 543 | |
| 15s | after `CloseMainWindow()`, before `TaskKill` | 546 | |
| 10s | after `TaskKill`, before reading the log | 549 | |
| 5s | after Archive2 extract, before deleting the BA2 | 430 | Archive2 only |

Fixed cost: ~50s per xEdit run (×2) plus polling; 10s per CK run (×4 clean, ×2 otherwise).

## Per-step notes not captured above

- **Intake**: resume menu lists steps 4–5 only in clean mode (213–216).
- **Step 1 preamble**: archive-exists → fatal; `Data\vis` non-empty → fatal; `RD` of
  `<fo4>\BSarchTemp` (256) — *only reached via step-1 entry*; delete `CombinedObjects.esp`,
  `- Geometry.psg`, and the session log.
- **Step 2 precondition**: no precombined meshes → `PauseAndExit`, **not** `failed` (275).
- **Step 3**: no precombined meshes → **silently skip to step 4**, not an error (283).
- **Step 5**: the only step that runs CK with **zero** pre-checks.
- **Step 6**: non-resume entry with a non-empty `Data\vis` is a hard stop, not a `failed`
  (304–306).
- **Step 8**: no `.uvd` files → **warning** and jump to `:Fin`, not a failure (326). Missing
  `- Main.ba2` → plain `:Archive vis` (415, 436–437).
- **Finish**: created-files manifest lists `.csg` / `.cdx` only in clean mode (337–340). The
  "Remove working files [Y]?" prompt is **skipped when non-interactive** — meaning
  non-interactive runs always clean (345). DLL restore runs on **every** exit path, including
  `:Failed` (366–368).

## Done when

- `docs/episodes.md` exists with the above, every citation verified against the file on disk.
- `docs/future-features.md` and `AGENTS.md` link to it.
