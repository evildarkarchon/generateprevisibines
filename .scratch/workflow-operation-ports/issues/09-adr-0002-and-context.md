# 09 — Record the decision: ADR-0002 and CONTEXT.md

Status: ready-for-agent
Blocked by: 06

## Why

ADR-0001 says Creation Kit, FO4Edit, archive, prompt **and wait** behaviour "sit behind named
adapters". Only CK and one prompt ever did. Phase A delivers the rest of that intent for CK
and adds decisions ADR-0001 does not cover — so this refines ADR-0001 rather than reversing
it.

Write a **new** ADR. Do not amend 0001 in place: its actual decision (the Workflow Operation
owns step domain flow, and why that beat `ToolRunner`) survives untouched, and editing it
would erase the reasoning trail.

## Work

### `docs/adr/0002-operation-ports-and-internal-tool-seams.md`

Status: accepted. Follow 0001's format — short, prose, no headings-per-section. It must
record, with reasons:

1. **The adapter bundle is data, not an interface.** `OperationAdapters` had exactly one real
   implementation; the ports inside it are what vary. `OperationPorts` is a plain struct.
2. **`ProcessRunner` and `Wait` are internal seams**, private to the tools layer. They are
   deliberately not exposed through `OperationPorts` — a deep module may have internal seams
   its own tests use without those becoming part of its interface.
3. **Tests substitute at `ProcessRunner`/`Wait`, not at a `CreationKit` port.** This is the
   decision most likely to be re-litigated, so give it the most space: faking a CK port would
   leave the composition untested — and the composition is precisely what had never been
   tested — while also minting a trait with one adapter.
4. **The CK adapter owns the log lifecycle and the exit-code policy; the Workflow Operation
   owns the success criteria.** Both halves follow from ADR-0001. Cite the evidence: the log
   scan predicate *and its severity* differ per step (handle-array is fatal, visibility-task
   is a warning, `CompressPSG` and `BuildCDX` scan nothing), whereas the non-zero-exit
   downgrade is uniform across all four operations inside the batch `:RunCK`.
5. **FO4Edit and archive adapters are deferred to Phase B**, designed against real callers.
   Note why they cannot be designed now: FO4Edit needs async spawn plus process-handle
   control and an unbounded poll, and "append" is a fiction for both archive tools.

### `CONTEXT.md`

Add one term, in the existing format:

> **Operation Ports**:
> The adapters a Workflow Operation is handed for one dispatch: the Creation Kit episode,
> confirmations, and the file space. It is the set of substitutable things a Workflow
> Operation may reach, not a seam in its own right — process spawn and MO2 wait sit behind
> the Creation Kit episode, not beside it.
> _Avoid_: adapter bundle, operation context, tool registry

Extend the existing **Workflow Operation** entry to say adapters arrive as ports rather than
through a bundle trait.

Do **not** add terms for `ProcessRunner` or the MO2 wait. They are plumbing behind an
internal seam; adding them would dilute the glossary.

## Done when

- `docs/adr/0002-*.md` exists covering all five points, and ADR-0001 is unmodified.
- `CONTEXT.md` has **Operation Ports** and an updated **Workflow Operation**.
