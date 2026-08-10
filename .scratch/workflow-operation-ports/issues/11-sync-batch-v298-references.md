# 11 — Sync docs to the V2.98 batch reference

Status: ready-for-agent
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
