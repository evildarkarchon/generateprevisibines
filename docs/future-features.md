# Future Features Backlog

## Purpose

This document is the implementation backlog for turning the Rust **scaffold** into a full replacement for the V2.96 GeneratePrevisibines batch workflow. [workarounds.md](workarounds.md) and the other docs in `docs/` remain the behavioral source of truth until parity is demonstrated on a real Fallout 4 / MO2 setup.

**Reader:** a maintainer choosing the next vertical slice.  
**After reading:** you can pick a slice, know which batch behavior it must preserve, and see how it relates to existing modules (`cli`, `discovery`, `validation`, `workflow`, `tools`).

---

## Parity Must-Haves

These are required before claiming the Rust binary replaces the batch script.

| Area | Batch reference | Rust target |
|------|-----------------|-------------|
| Tool discovery | Lines 24–41, 57–78 | `discovery` — registry + cwd FO4Edit, Fallout 4 path, CK, Archive2, BSArch |
| CLI / parameters | `:CheckParam`, header comments | `cli` — `-clean`/`-filtered`/`-xbox`, `-bsarch`, `-FO4:dir`, plugin arg |
| Plugin rules | `:CheckPluginName`, `:SpaceInName` | `validation` — reserved names, clean-mode spaces, seed copy prompts |
| CKPE validation | `:TestCKPEConfig`, `:CheckCKPEConfig` | `validation` — TOML/INI/legacy ini, log path, handle limit warning |
| xEdit script versions | `:CheckScripts` | `validation` — script presence + version markers |
| 8-step workflow | `:Precomb` … `:Fin`, `:GetStep` | `workflow` — correct step list per build mode, resume from step N |
| CK execution | `:RunCK` | `tools/creation_kit` — DLL guard, wait, log append, MO2 delay |
| FO4Edit scripts | `:RunScript` | `tools/fo4edit` — plugin list, `-D:Data` when `-FO4` set, keystroke automation |
| Archives | `:Archive`, `:Extract`, `:AddToArchive` | `tools/archive` — Archive2 extract/repack; BSArch pack/append; Xbox compression |
| DLL restore | `:Done` | `tools/dll` — restore `*-PJMdisabled` on success and failure |
| Logging | `%TEMP%` logs, unattended log | `logging` — session log + unattended log merge |
| UX messages | Throughout | Match familiar batch error strings where practical ([behaviors.md](behaviors.md)) |

---

## Workflow Implementation

Ordered slices recommended for parity work:

1. ~~**Interactive plugin flow**~~ — implemented in `interactive` (`dialoguer` prompts, seed copy + 5s MO2 delay, Y/N/C, `:GetStep` resume).
2. ~~**Step 1 — Generate precombines**~~ — implemented through the Workflow Operation seam with `tools/creation_kit` and `checks` (preamble/post, CK run + 10s delay). Steps 2–8 stop the runnable sequence at the first missing registered operation.
3. **Step 2 — Merge CombinedObjects** — FO4Edit `Batch_FO4MergeCombinedObjectsAndCheck.pas`, warning on `Error:` in unattended log.
4. **Step 3 — Archive precombines** — create `{plugin} - Main.ba2`, delete precombined folder when using Archive2.
5. **Steps 4–5 — PSG / CDX** (clean only) — `CompressPSG`, `BuildCDX`, delete intermediate `.psg`.
6. **Step 6 — Generate previs** — empty `Data\vis`, `GeneratePreVisData`, visibility task warning.
7. **Step 7 — Merge previs** — `Batch_FO4MergePrevisandCleanRefr.pas`, success string check.
8. **Step 8 — Add previs to archive** — `AddToArchive` with extract-repack path for Archive2.
9. **Finish / cleanup** — list output files, optional delete CombinedObjects.esp / Previs.esp, restore DLLs.

Each slice should wire through the Workflow Operation seam and real tool adapters.

---

## Reliability and Observability

- Structured tracing spans per workflow step with plugin name and build mode fields.
- On failure: ensure DLL guard restores even when CK/xEdit panic or are killed.
- Surface Creation Kit log path and tail relevant errors (handle array, visibility task).
- Distinguish **ERROR** vs **WARNING** consistent with batch (`failed` vs continued build).
- Single exit code policy documented for CI/non-interactive use.
- Persist last completed step to support resume without re-prompting (optional enhancement; batch uses interactive menu only).

---

## Testing Strategy

| Layer | Scope |
|-------|--------|
| Unit tests | CLI parsing, plugin validation, CKPE parsing, workflow step lists (existing) |
| Fixture tests | Sample CKPE `.ini`/`.toml` files, mock `Edit Scripts` with version strings |
| Mock operation adapters | Assert step order and arguments without real CK/FO4Edit |
| Integration | Manual checklist on Windows + MO2 + real plugin (document in README) |

Manual integration checklist (minimum):

**Slices 1–2 (interactive + Step 1)** — verify on Windows with Fallout 4 + MO2:

- [ ] Interactive: empty plugin name exits 0
- [ ] Interactive: seed copy from `Data\xPrevisPatch.esp` (5s MO2 delay if file not visible immediately)
- [ ] Interactive: existing plugin Y/N/C; **C** shows resume menu; **0** re-prompts plugin name
- [ ] Interactive: resume step 1 with existing `meshes\precombined\*.nif` prompts to delete folder
- [ ] Non-interactive: `generateprevisibines.exe MyMod.esp` runs Step 1 only; errors if `--resume-from` ≥ 2
- [ ] Clean mode: CK qualifiers `clean all`; PSG `{plugin} - Geometry.psg` required after CK
- [ ] Filtered/Xbox: CK qualifiers `filtered all`; no PSG check
- [ ] CK log scanned for `OUT OF HANDLE ARRAY ENTRIES` (case-insensitive, any prefix)
- [ ] DLLs renamed to `*-PJMdisabled` during CK and restored after exit

**Full parity (later slices):**

- [ ] Clean mode full run produces `.esp`, `.csg`, `.cdx`, `- Main.ba2`
- [ ] Filtered mode skips steps 4–5
- [ ] Xbox mode uses Xbox compression flag on archives
- [ ] Resume from step 6 after forced failure at step 5
- [ ] BSArch path vs Archive2 path
- [ ] `-FO4:` override sets xEdit data directory correctly

---

## Usability Improvements

Optional once parity exists; must not change batch-compatible defaults:

- Full interactive mode with `dialoguer` (Y/N/C prompts matching CHOICE behavior).
- `--dry-run` expanded to show external commands without executing them.
- Progress ETA / elapsed time per step.
- Validate Fallout 4 / CK / FO4Edit executables up front with actionable paths.
- Safer path canonicalization and space handling in `--FO4`.

---

## Release and Packaging

- GitHub Actions: `cargo build --release`, `cargo test`, `cargo clippy` on Windows runner.
- Ship `generateprevisibines.exe` with README install instructions.
- Embed version from `Cargo.toml`; print batch reference version (V2.96) in banner.
- Version alignment with the local V2.96 batch reference (not tracked in git).

---

## Deferred Ideas

Do **not** implement these in parity work unless explicitly requested:

- Removing MO2 timing delays ([workarounds.md](workarounds.md) §2).
- Replacing FO4Edit keystroke automation with unsupported “headless” assumptions (§1).
- Skipping DLL renaming around CK (§3).
- “Optimizing” Archive2 by skipping extract-repack (§4).
- MO2 mode via `mo2-mode` submodule integration (README previously described `--mo2` flags — evaluate against batch “run from MO2” requirement before exposing).

---

## Module Map (current scaffold)

```text
src/
  main.rs           # entry, interactive + production run
  lib.rs
  cli.rs            # native legacy normalization + private Clap grammar
  config.rs         # BuildMode, ArchiveTool, WorkflowStep, ProjectConfig
  interactive.rs    # plugin prompts, seed copy, resume menu
  checks.rs         # Step 1 preamble/post path checks
  discovery.rs      # tool paths (partial)
  validation.rs     # plugin + CKPE + xEdit scripts
  workflow.rs       # step planning + engine
  logging.rs        # session log + CK log append
  workflow/         # Workflow Operation registration, dispatch, and operation tests
  tools/            # CK adapter, DLL guard, FO4Edit/archive stubs,
                    #   process/wait internal seams (MO2 sync delays live here)
  error.rs
```

Pick the next slice from **Workflow Implementation** and close parity items in **Parity Must-Haves** for that step before moving on.
