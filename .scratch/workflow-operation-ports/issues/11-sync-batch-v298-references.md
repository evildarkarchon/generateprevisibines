# 11 — Sync docs to the V2.98 batch reference

Status: resolved
Blocked by: none

Independent of Phase A. Deliberately kept out of the refactor so the Phase A diff stays
reviewable — this one touches all 8 steps across several files and changes no code behaviour.

## Why

The batch script on disk (`GeneratePrevisibines.bat`, gitignored, 556 lines) is **V2.98
Jun 2026**. Every line reference in the docs is against V2.96, and they no longer resolve.
Anyone following a citation lands in the wrong place.

## Known drift

`docs/workarounds.md` — all four references are stale:

| Section | Cited (V2.96) | Actual (V2.98) |
|---|---|---|
| §1 PowerShell keystroke automation | 499–511 | 534–547 |
| §2 MO2 timing delays | 169, 436, 497, 513 | 191, 430, 459, 533, 536, 540, 543, 546, 549 |
| §3 DLL renaming | 422–427, 330–335 | 445–450, 353–358 |
| §4 Archive2 extract-repack | 390–414 | 404–410, 428–435 |

Elsewhere:

- `docs/future-features.md` pins parity to V2.96 throughout, including the release note
  "print batch reference version (V2.96) in banner".
- `src/logging.rs:35` emits `Starting {mode} Build V2.95 of {plugin}` — a *third* version
  string. The batch's own header line has drifted too (it still says V2.95 inside a V2.98
  script), so decide deliberately whether to match the batch's literal output or the real
  version, and write the reasoning into the doc comment.
- `src/tools/creation_kit.rs:76` and `src/workflow/operations/generate_precombines.rs:76`
  carry inline batch line references that need re-checking.

## Work

Re-derive every batch line citation in `docs/` and in inline code comments against the V2.98
file, and update the version strings. Sweep for `V2.9` across the repo rather than fixing
only the ones listed above.

## Notes

- The user's assessment: the V2.98 changes are minor, mostly string-level, and should sync
  easily. Treat a *behavioural* difference found during the sweep as a finding to raise, not
  something to silently absorb into this ticket.
- The handle-array marker divergence is **not** part of this — it is a code fix, tracked in
  issue `07`.

## Done when

- Every batch line reference in `docs/` and in code comments resolves correctly against the
  V2.98 file.
- One version string is used consistently, with a documented reason where it differs from the
  batch's literal output.

## Comments

Resolved. Every citation below was re-derived by reading the 556-line V2.98 file on disk.

**Version-string policy.** V2.98 is now the single reference version everywhere a doc or
comment *names which batch we are porting*: `README.md` (×2), `src/cli.rs`, `src/config.rs`,
`src/main.rs` (module doc + the `print_banner` line), `src/validation.rs`,
`docs/future-features.md` (×3). The one deliberate exception is
`logging::build_session_header`, which still emits `V2.95`. That string is not a version claim
— it is a literal reproduction of batch line 260, which hardcodes `V2.95` inside a V2.98
script. Matching the batch's *output* is what session-log parity means, so the drift is the
batch's and copying it is correct. The reasoning now lives in the function's doc comment, and
`docs/future-features.md` § *Release and Packaging* points at it so the next reader who greps
`V2.9` does not "fix" it.

**Citations re-derived.** `docs/workarounds.md` §1 499–511 → 534–547; §2 169/436/497/513 →
191, 430, 459, 533, 536, 540, 543, 546, 549; §3 422–427 + 330–335 → 445–450 + 353–358;
§4 390–414 → 404–410 + 428–435. `docs/behaviors.md`: reserved names 147–154 → 168–176,
clean-mode spaces 134–158 → 155–157 + 177–180, resume menu 186–207 → 208–229.
`docs/future-features.md` tool discovery 24–41 + 57–78 → 24–40, 46–50, 67–88, plus 513/138 for
BSArch. Code comments: `src/discovery.rs:1` 24–41 → the four separate blocks it actually
covers; `src/validation.rs:51` 81–87 → 91–97; `src/logging.rs` `:Precomb` 247 → `:Precomb2` 260
and `:RunCK` 447–448 → 460–461; `src/tools/creation_kit.rs` 249–253 → 262–266 (×2).

`docs/adr/0002-…md` (270–271, 312–313, 463) was re-verified and needed no change.
`docs/technical.md` carries no line citations. `docs/episodes.md`'s citations were all correct,
but review caught one non-citation factual error in it: `:PauseAndExit` is entered by `goto`
from **fifteen** sites (44, 65, 80, 81, 82, 103, 114, 138, 150, 194, 275, 498, 502, 512, 515),
not eleven. Corrected. The claim that depends on it — all but one sit before any CK invocation —
survives unchanged; the four newly counted sites are in `:CheckParam` and `:CheckScripts`, both
of which run before the workflow.

The spec's Known drift also named `src/workflow/operations/generate_precombines.rs:76` as
carrying a batch citation to re-check. It carries none — Phase A moved that code and the
citation did not come with it. Nothing to do there.

Each of the four `workarounds.md` sections also gained the specific per-line detail the flat
range was hiding (which line is the `SendKeys`, which is the `TaskKill`, which delay is which),
since a bare range is what let the drift go unnoticed for two versions.

**Two behavioural differences found, both raised rather than absorbed.** Both are in
`discovery`, and both trace to items the V2.98 header advertises:

- V2.98 probes `xFOEdit.exe` first in `:xEditCheck` (line 26); `FO4EDIT_CANDIDATES` does not
  carry it. Filed as issue `12`.
- V2.98's "Support for Wine" is a `WHERE /Q reg.exe` probe (21–22) whose `RegErr_` guards
  (38, 49) skip *both* registry lookups on a host without `reg.exe`. `discovery` calls `winreg`
  unguarded. The xEdit half is already near-equivalent; the Fallout 4 half degrades an
  actionable error into a raw registry error. Filed as issue `13`. This one needs a Wine-support
  decision before any code changes, which is why it is not folded into `12`.

The remaining V2.98 header changes (`-D:<dir>\Data`, the MO2 error checks) land in FO4Edit
territory, which is unimplemented and already described correctly in `docs/episodes.md`.
`src/tools/dll.rs` was checked against the six-DLL list at lines 445–450 and already matches,
including `d3dcompiler_46e.dll`.

**Review corrections.** A verification pass over this work found five defects in it, all now
fixed: `workarounds.md` §4 cited 412–413 where only 413 supports the claim; §2 folded line 540
into a linear delay sequence when it is one iteration of the unbounded `:loop` poll (539–541);
issue `12` cited `src/discovery.rs:7` for `FO4EDIT_CANDIDATES`, which is line 8; the
`build_session_header` doc comment paired "banner and author header" with "(lines 15, 18)" in
the wrong order; and the `episodes.md` count above.
