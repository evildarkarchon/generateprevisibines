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

Line references are against the **V2.99** batch (`GeneratePrevisibines.bat`, committed at the
repo root, 564 lines, header `PJM V2.99 Aug 2026`). Every citation below was verified against
that file. The V2.98 numbering this document used before (556 lines, never committed) runs six
to nine lines earlier; the V2.98 → V2.99 re-sync and its behavioural diff are recorded on
issue #32. Where this document contradicts the older prose in [workarounds.md](workarounds.md)
or [future-features.md](future-features.md), the citations here are the current ones — see
`.scratch/workflow-operation-ports/issues/11-sync-batch-v298-references.md`.

---

## Creation Kit — four operations, one invocation shape

All four go through the single `:RunCK` subroutine (453–474):

```
START "CK" /D"<fo4dir>" /wait "CreationKit.exe" -<Operation>:"<PluginNameExt_>" <qualifiers>
```

The plugin argument is **always the patch plugin** — never `CombinedObjects.esp` or
`Previs.esp`; those are outputs.

The quotes around `<PluginNameExt_>` belong to `cmd`'s tokenizer, not to Creation Kit's
grammar: `CommandLineToArgvW` strips them, so CK sees `-<Operation>:<plugin>`. A port that
builds argv directly must **not** reproduce them — see
`.scratch/workflow-operation-ports/issues/15-ck-plugin-arg-double-quoting.md`.

| Operation | Step | Qualifier | Expected output | Post-run log scan |
|---|---|---|---|---|
| `GeneratePrecombined` | 1 | `clean all` in clean and xbox mode, `filtered all` in filtered mode — BuildMode-derived (268–272) | `CombinedObjects.esp` (fixed) | `OUT OF HANDLE ARRAY ENTRIES` (276) → **fatal** (277) |
| `CompressPSG` | 4 | *(empty)* (300) — clean mode only | `<plugin> - Geometry.csg` | none |
| `BuildCDX` | 5 | *(empty)* (307) — clean and xbox mode | `<plugin>.cdx` | none |
| `GeneratePreVisData` | 6 | `clean all` — **hardcoded, ignores BuildMode** (317) | `Previs.esp` (fixed) | `ERROR: visibility task did not complete.` (319) → **warning** (320) |

`:RunCK` itself enforces only two things, uniformly: the expected output file exists (471),
and a non-zero exit is downgraded to a warning (472). The log scans live at the call sites —
their predicate *and severity* differ per step. Both scans are also skipped entirely when the
CK log is absent (275, 318), which jumps straight to the next step rather than failing.

**The output check only stops an interactive run.** Line 471's `goto failed` runs inside the
`Call`ed `:RunCK` frame, and `:Failed` → `:Done` ends non-interactive runs with
`goto :eof` (369). Inside a `Call`, that *returns to the caller* rather than ending the script,
so a non-interactive run whose Creation Kit produced no output resumes after the `Call` and
carries on into the next step (Step 1 into 270/274, Step 6 into 318 and Step 7). Interactive
runs reach `PAUSE`/`Exit` (371–372) and really stop. The port deliberately does **not**
replicate this: a missing Creation Kit output stops every run — decided on #27. The same
`goto :eof`-inside-`Call` shape sits under the fatal checks in `:RunScript` (560–563), which
the port also stops on in every mode — decided on #28 — and under every `:Archive` failure path
(the non-zero exits at 398/406/417/431, the archive-exists check at 408, and the BSArch
move-backs at 400/433), which the port likewise stops on in every mode — decided on #29.

Step 1 has a second, mode-conditional output check (`<plugin> - Geometry.psg`, line 270, run in
every mode except filtered) that is not the `:RunCK` parameter. The same file is re-checked on
entry to step 4 (297), again in every mode except filtered.

Full episode chain per CK run: rename six ENB/ReShade DLLs to `*-PJMdisabled` (454–459) →
delete the CK log (460) → write header and `Start <time>` to the session log (461–463) →
`START /wait` (464) → capture `ERRORLEVEL` into `Err_` (465) → write `Ended <time>` (466) →
**wait 10s** (468) → append the CK log to the session log, or note it was missing (469–470) →
output-file check (471) → exit-code warning (472).

Note that the DLLs are *not* restored inside `:RunCK` — restoration happens once, at `:Done`
(362–367). That covers every exit path that *reaches* `:Done`: success via `:Fin`, `:Failed`
(375–377), and the three `Goto Done` hard stops (255, 260, 313). It does **not** cover
`:PauseAndExit` (370), which is entered by `goto` from fifteen sites and only falls through
from `:Done` on the interactive path (369). All but one of those `goto PauseAndExit` sites sit
before any CK invocation, so nothing is renamed yet; the exception is step 2's precondition
(281), reachable only when step 1's identical check at 274 already passed — i.e. on a
resume-at-step-2 where this run never renamed anything. Leftover `*-PJMdisabled` files from an
earlier crashed run are not restored on that path.

The port restores per CK run instead (`DllGuard`, dropped when each CK episode ends, on every
path including panics). Leftovers from a crashed earlier run — decided on #30 — are **adopted**
by the next guard: a `*-PJMdisabled` with no original beside it is recorded as one of the
guard's own renames, so that run's restore recovers it. Recovery therefore happens on the next
run that launches CK (Steps 1, 4, 5, 6), not on every run; Finish has no DLL duty. When both the
leftover and the original exist, the guard keeps its existing behaviour (the leftover is
replaced).

## FO4Edit — two script runs, identical shape

Both through `:RunScript` (530–564).

| Step | Script | Source plugin | Required version | Call-site criterion |
|---|---|---|---|---|
| 2 | `Batch_FO4MergeCombinedObjectsAndCheck.pas` | `CombinedObjects.esp` | V1.5 (141) | `"Error: "` **present** → warning (284–285) |
| 7 | `Batch_FO4MergePrevisandCleanRefr.pas` | `Previs.esp` | V2.3 (140) | `"Completed: No Errors."` **absent** → warning (327–328) |

Note the polarity inversion between the two — one fails on a positive match, the other on a
negative one.

Shared fatal checks inside `:RunScript`: `Error: Missing [<target>] or [<source>] modules`
(560–561) and absence of `"Completed: "` (562–563).

The launch flag set (543) is:

```
START "xEdit" /B <xEdit.exe> -fo4 -autoexit -P:"%TEMP%\Plugins.txt" <ModDir_> ^
    -Script:<script.pas> -Mod:<target plugin> -log:"%TEMP%\UnattendedScript.log"
```

The `-D:<dir>\Data` flag is **not per-call** — `ModDir_` is set once at line 498 when `-FO4:`
is passed, so both runs get it or neither does.

Full episode chain per run: write `%TEMP%\Plugins.txt` (`*<target>`, `*<source>` — 534–535) →
delete `%TEMP%\UnattendedScript.log` (537) → **wait 10s** (542) → `START /B` (543: async, no
`/wait`) → **wait 5s** (545) → PowerShell `AppActivate(<xEdit process>)`, `Start-Sleep 1`,
`AppActivate('Module Selection')`, `SendKeys {ENTER}` (546) → **poll every 5s** until the
unattended log appears (548–550) → **wait 10s** (552) → `CloseMainWindow()` (553) →
**wait 15s** (555) → `TaskKill /IM` (556) → **wait 10s** (558) → append log to session log
(559) → scan (560–563).

Neither close step is a kill: `TaskKill` has no `/F`, so it is a second close request, and it
reaches every `FO4Edit.exe` on the machine. Both matter because xEdit writes the merged plugin
from its own close path, so the close sequence is what saves the merge. Under Wine both
PowerShell lines (546, 553) are silent no-ops, because Wine's `powershell.exe` is a stub. A Wine
user therefore presses OK in Module Selection by hand, and Wine's own `taskkill` does the
closing. Sources: `docs/research/fo4edit-window-automation.md` on branch
`research/fo4edit-window-automation`.

**Port divergences, decided on #28:**

- Dismissal targets this run's `Module Selection` window only. The port tries a targeted
  message, then foreground plus `SendInput`, then prints an instruction to press OK. It never
  sends input to any other window.
- The poll stops the run if FO4Edit exits before the log appears.
- Close requests go to this run's PID only, and the port never kills FO4Edit. If FO4Edit is
  still running after the close sequence, the run stops.
- Step 2 checks for `CombinedObjects.esp` before launching FO4Edit, mirroring Step 7's
  `Previs.esp` check at 324. The batch does not check it at all.
- The two shared fatal checks (560–563) stop every run.

The port keeps every delay, keeps `-autoexit`, and writes `Plugins.txt` without the trailing
space that `echo` adds.

## Archive — verbs, and where the tools diverge

| Verb | Archive2 | BSArch |
|---|---|---|
| pack one folder | `Archive2.exe <folder> -c="<archive>" %Arch2Quals_% -f=General -q`, cwd = `Data` (405) — `%Arch2Quals_%` is always empty at V2.99, see below | `BSArch.exe Pack "<fo4>\BSArchTemp" "<archive>" -mt -fo4 -z`, cwd = `Data` (397) — packs a *staging dir* |
| pack multiple folders | `meshes\precombined,vis` — comma-separated (442) | implicit; staging dir already holds both |
| extract | `Archive2.exe "<archive>" -e=. -q` (416) | **never invoked** — `:Extract` (413–419) has no BSArch branch |
| append | **not supported** → extract-repack | **also not a verb** — see below |
| delete source folder | `RD /S /Q` (292, 443, 336) | never — folders were `MOVE`d into staging (396, 429) |

**BSArch has no native append.** It runs the *identical* `Pack` command in steps 3 and 8. The
difference is that step 3's BSArch path **moves** the precombined meshes into
`<fo4>\BSArchTemp\Meshes` (395–396) rather than deleting them (contrast line 292,
Archive2-only), so step 8 adds `vis` to the same tree (428–429) and repacks wholesale.
Archive2 recovers prior content *from the archive*; BSArch retains it *on disk* across steps.

On failure, both BSArch paths move the staged folder back before `goto failed` (400, 433) —
note the two targets differ (`Data\Meshes` vs `Data`), mirroring the differing `MOVE`
destinations above.

Archive2's step-8 workaround ([workarounds.md](workarounds.md) §4, V2.99 lines 437–444):
extract into `Data` (438) → **wait 5s** (439) → delete the BA2 (440) → re-scan
`meshes\precombined\*.nif` (441) → repack `meshes\precombined,vis` (442) → `RD` the
precombined folder (443).

That re-scan is a branch, not an assertion: if the extract produced no precombined `.nif`
files, control falls through to `:ArchiveOnly` (441 → 445) and the archive is rebuilt from
`vis` alone, silently dropping the precombines.

**Two divergences worth designing around:**

- **No archive gets Xbox compression.** At V2.99 the only line that would set
  `Arch2Quals_=-compression=XBox` is commented out (`REM`, 391), so `Arch2Quals_` is always
  the empty string `:Archive` assigns at 390. The `%Arch2Quals_%` slot on the Archive2 command
  line (405) expands to nothing, and both archive log headers (392, 426) print a doubled space
  where the qualifier would go. `-xbox` therefore changes nothing on either archive path. The
  BSArch command never carried the qualifier in any version (397, 430). (V2.98 still set it, so
  there Xbox compression reached the Archive2 command line only, and `-xbox -bsarch` silently
  produced non-Xbox archives.) The port replicates this: no Build Mode requests Xbox
  compression and `-xbox -bsarch` gets no warning, because nothing is being dropped — decided
  on #29.
- **BSArch staging state is cross-step.** `<fo4>\BSArchTemp` is cleared in exactly two
  places: line 262, inside the Step 1 preamble, and line 435, after a *successful* BSArch
  step-8 pack. So resuming at step 3 or 8 with `-bsarch` inherits whatever a run that stopped
  between step 3 and a successful step 8 left there — including the partially-moved tree from
  a failed pack (431–434, which returns only the one folder it moved).

**Port divergences, decided on #29** (rationale in
[ADR-0004](adr/0004-plugin-archive-is-the-only-cross-step-archive-state.md)):

- **The Plugin Archive is the only state carried across the archive steps.** Step 3 packs the
  precombines and removes the loose files with either tool. Step 8 rebuilds the archive from its
  own contents plus `vis`. Archive2 keeps the batch's extract → 5s → repack exactly. BSArch
  `unpack`s the archive into staging, moves `vis` in and packs. The batch's `BSArchTemp` has no
  port counterpart: one run-owned work folder, `<fo4>\ArchiveWork`, holds the BSArch staging tree
  and both tools' output. The step that created it removes it when it ends, except when a cleanup fails or
  when a move-back or swap fails (see below).
- **Build elsewhere, check, then swap.** Both tools write the new archive into the work folder
  under its final name. Archive2 does this with `-c=` pointing there; its sources stay relative
  under cwd `Data`, so rooting is unchanged. The archive-exists check runs on that file, and
  only then is the archive in `Data` replaced. The batch deletes the archive *before*
  repacking (440 → 442), so its failed repack leaves no archive.
- **BSArch waits 5s before every pack**, in Step 3 and Step 8, after everything is staged. The
  batch's BSArch path has no wait. Under MO2, files moved out of the virtual `Data` may not have
  settled when BSArch reads staging, which yields an incomplete archive.
- **Leftover work folders are cleared at the start of every run.** After preparation and
  before the first step, whatever the resume point, the run removes `<fo4>\ArchiveWork` and
  every `<fo4>\ArchiveWork.<n>`, whatever they hold. This replaces the batch's Step 1-only `RD`
  of its staging folder (262; see *Per-step notes*). A folder that cannot be removed (for
  example, because of a usvfs or antivirus lock) gets a Build Warning naming it, and the run
  continues. Each archive step builds in the first work-folder name that does not exist
  (`ArchiveWork`, then `ArchiveWork.1`, `ArchiveWork.2`, …). The archive is therefore always
  built in a clean folder, so nothing stale is packed (BSArch packs the whole staging folder),
  and a stuck folder never stops the run. *Amended on #45, which reversed #29's "a leftover
  stops the run".*
- **Cleanup failures are Build Warnings, never stops** (decided on #45). Cleanup covers the
  work folder after a step, the loose `meshes\precombined` after the Step 3 Archive2 swap, and
  the loose `meshes\precombined` and `vis` after the Step 8 Archive2 swap. On a failure path,
  the step's own error is still the one that stops the run. Two removals are not cleanup, and
  their failure stops the run: the swap's delete of the old archive, and the BSArch delete of
  the old `vis` it unpacked, because both decide what the archive contains.
- **A failed move-back or swap** stops the run, with both paths named, and leaves the work
  folder in place. It may hold the user's only copy of their precombines or of the new archive.
  The next run will clear it, so the error tells the user to recover their files before
  rerunning.
- **No precombined meshes after the extract or unpack stops the run** before the old archive
  is touched, instead of rebuilding from `vis` alone (441 → 445).
- **`:ArchiveOnly` is dropped.** A missing archive at Step 8 stops the run (see *Per-step
  notes*), which also removes the `Meshes\vis\*.uvd` bug.
- **Tool output** is captured — stdout *and* stderr — and appended to the session log under the
  batch's `Creating … Archive …` / `====` header. The batch keeps stdout only, which loses
  Archive2's `-1` stack traces.

**Known limitation, kept for parity:** BSArch 1.0 roots internal paths after the *first*
`\data\` in the absolute path, so an install under a `Data`-named ancestor (for example
`D:\Data\Games\Fallout 4`) mis-roots every file. That applies to `Data\…` sources and to
staged ones alike, and Archive2 may behave the same way (unverified). The batch has the same
exposure, and the port does not guard against it.

## Waits (MO2 VFS sync)

| Duration | After which action | Line | Notes |
|---|---|---|---|
| 5s | seed copy of `xPrevisPatch.esp` — **only if** not yet visible | 197 | conditional |
| **10s** | **every Creation Kit exit** (all 4 operations) | 468 | ×4 clean, ×3 xbox, ×2 filtered |
| 10s | **before** launching xEdit | 542 | pre-launch |
| 5s | after launching xEdit, before `SendKeys` | 545 | lets the dialog appear |
| 1s | between the two `AppActivate` calls | 546 | inside the PowerShell one-liner |
| 5s | each poll iteration awaiting the unattended log | 549 | unbounded loop |
| 10s | after the log appears, before `CloseMainWindow()` | 552 | |
| 15s | after `CloseMainWindow()`, before `TaskKill` | 555 | |
| 10s | after `TaskKill`, before reading the log | 558 | |
| 5s | after Archive2 extract, before deleting the BA2 | 439 | Archive2 only |

Fixed cost: ~50s per xEdit run (×2) plus polling; 10s per CK run (×4 clean, ×3 xbox, ×2
filtered).

The port adds one wait the batch lacks: **5s before every BSArch pack** (Steps 3 and 8), so that
files moved out of MO2's virtual `Data` settle before BSArch reads its staging folder — decided
on #29.

## User prompts

Every prompt is reached only on the interactive path, so a non-interactive run never blocks on
input — but by two different mechanisms. Four prompts sit behind an explicit `NoPrompt_` test
that bypasses them; the other three are unreachable because the only route to them is through
a prompt of the first kind.

| Prompt | Line | Why non-interactive skips it | Non-interactive behaviour |
|---|---|---|---|
| `Enter Patch Plugin name (return to exit)` | 158 | `NoPrompt_` test at 156 | `PauseAndExit` |
| `Plugin does not exist, Rename xPrevisPatch.esp to this? [Y/N]` | 193 | `NoPrompt_` test at 191 | error, back to `:GetPlugin` → exit |
| `Plugin already exists, Use It? [Y], Exit [N], Rerun from failed step [C]` | 210 | `NoPrompt_` test at 209 | assumes **Y** — straight to step 1 |
| `Restart at step (1 - 8 or 0 to exit)` | 226 | `:GetStep` is entered only from 212 (**C**), or re-entered from 240 / 247 | unreachable |
| `Precombine directory … needs to be empty. Clean it? [Y/N]` | 239 | `:RePrecomb` is a `:GetStep` target only | unreachable |
| `Previs directory (Data\vis) needs to be empty. Clean it? [Y/N]` | 246 | `:RePreVis` is a `:GetStep` target only | unreachable |
| `Remove working files [Y]?` | 355 | `NoPrompt_` test at 354 | assumes **Y** — always cleans |

The two "clean it?" prompts are the only places the workflow deletes `Data\meshes\Precombined`
(241) or `Data\vis` (248) on the user's behalf at intake, and they exist only on the resume
path (`:RePrecomb`, `:RePreVis`).

## Per-step notes not captured above

- **Intake**: resume menu lists steps 4–5 in every mode except filtered (219–222); an
  unrecognised choice loops back to `:CheckPluginExists` (235).
- **Step 1 preamble**: precombined dir non-empty → `:Done` (253–255, non-resume entry only);
  archive already exists → back to `:GetPlugin` (257), i.e. re-prompt interactively and
  `PauseAndExit` otherwise; `Data\vis` non-empty → `:Done` (258–260). None of these three is
  a `:failed`. Then `RD` of `<fo4>\BSarchTemp` (262) — *only reached via step-1 entry* —
  delete `CombinedObjects.esp` (263), `- Geometry.psg` (264), and the session log (265). The
  port replaces the `RD` at 262 with clearing its own work folders (`<fo4>\ArchiveWork` and any
  `ArchiveWork.<n>`) at the start of **every** run, not only on a Step 1 entry. A folder that
  cannot be removed is a Build Warning — decided on #45 (see *Archive*). The port never touches
  the batch's `BSArchTemp`.
- **Step 2 precondition**: no precombined meshes → `PauseAndExit`, **not** `failed` (281).
- **Step 3**: no precombined meshes → **silently skip to step 4**, not an error (289); in
  filtered mode step 4 then immediately forwards to step 6 (296), and in clean and xbox mode
  the skip lands on step 4's `- Geometry.psg` check (297). The skip is reachable only on a
  resume at 3, because the port's Step 2 stops on zero meshes. The port completes with nothing
  to do when the archive already exists (Step 3 ran on an earlier attempt), and **stops the run**
  when there is no archive either — decided on #29.
- **Step 4**: three-way build-mode gate. Filtered → step 6 (296). Clean and xbox: no
  `<plugin> - Geometry.psg` → `failed` (297). Xbox then skips `CompressPSG` and goes straight
  to step 5 (298), so its `.psg` is never compressed or deleted — it is the geometry file the
  Finish manifest lists for xbox. Clean runs `CompressPSG` (299–300) and deletes the `.psg`
  (301).
- **Step 5**: the only step that runs CK with **zero** pre-checks of its own (304–307 — only
  the build-mode gate, which skips it in filtered mode alone, 305). Reached by fallthrough it
  inherits step 4's `.psg` check; a resume at step 5 enters at `:BldCDX` (231) and skips that.
- **Step 6**: non-resume entry with a non-empty `Data\vis` is a hard stop via `:Done`, not a
  `failed` (311–313).
- **Step 7 preconditions**: no `.uvd` files → `failed` (323); no `Previs.esp` → `failed` (324).
  Contrast step 8's warning for the same missing `.uvd` files.
- **Step 8**: no `.uvd` files → **warning** and jump to `:Fin`, not a failure (333). Missing
  `- Main.ba2` → plain `:Archive vis` (424, 445–446). On the BSArch path that fallback stages
  `vis` under `BSArchTemp\Meshes` (395–396, which hardcodes `\Meshes` for its step-3 caller),
  so the resulting BA2 holds `Meshes\vis\*.uvd` rather than `vis\*.uvd` — a latent bug in the
  batch, not a behaviour to replicate. The port keeps the no-`.uvd` warning as a Build Warning
  and completes. It is reachable only on a resume at 8, and it is the normal state after a Step 8
  that succeeded. A missing `- Main.ba2` **stops the run** instead of falling back to
  `:ArchiveOnly`: a `vis`-only archive is a broken build — decided on #29.
- **Finish**: created-files manifest (343–350) lists the plugin and `- Main.ba2` in every mode;
  outside filtered mode it adds `<plugin>.cdx` plus one geometry file — `- Geometry.csg` in
  clean mode, `- Geometry.psg` in xbox mode (345–349). The
  "Remove working files [Y]?" prompt is **skipped when non-interactive** — meaning
  non-interactive runs always clean (354). DLL restore runs on every exit path that reaches
  `:Done`, including `:Failed` (375–377 → `:Done`, 362–367) — but not on the `:PauseAndExit`
  paths; see the `:RunCK` note above for why that is nearly always harmless.
  **Port, decided on #30:** Finish is a run epilogue, not a Workflow Operation, and runs only
  when every planned step completed. It keeps the batch's lines, manifest and prompt verbatim
  (manifest by Build Mode, unchecked; only "Build of Patch X Complete." reaches the session
  log; no MO2 wait before the deletes; a missing Working File is skipped). One divergence: a
  Working File that cannot be deleted is a Build Warning and the run still exits 0.
