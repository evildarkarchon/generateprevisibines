# Workflow Operation Ports — Phase A (Creation Kit episode)

Status: ready-for-agent

Raise the Workflow Operation seam above the external tools, for the one external-tool
episode that has a real caller today: Creation Kit. Derived from an architecture review
and a grilling session; every decision below was made deliberately, and the rejected
alternatives are recorded so they are not re-litigated.

## Problem

The seam is drawn *below* Creation Kit rather than around it.

- `CkOperation` (CK's own verb enum) and `qualifiers: &str` (a raw batch-script fragment,
  `"clean all"` / `"filtered all"`) cross the seam, so a Workflow Operation has to know
  CK's command grammar.
- `OperationAdapters::run_creation_kit` takes `run: &WorkflowRun` back, and the production
  adapter immediately calls `run.tool_context()` to get out again. The call graph is a
  circle, and the test fake exploits it.
- The CK log path reaches the operation via `run.tool_context().ck_log_path`, bypassing the
  adapter that owns that file's lifecycle (deletes it before the run, appends it to the
  session log after).
- Nothing below `OperationAdapters` has a single test: not the CK argv, not the DLL guard,
  not the mandated 10s MO2 delay, not the log lifecycle, not the non-zero-exit policy.
  `ProductionOperationAdapters` itself has zero tests — every Step 1 assertion is about the
  fake.
- `OperationAdapters` is a trait with exactly one real implementation. By the seam rule used
  throughout this repo (one adapter = hypothetical seam, two = real), the bundle is not a
  seam at all — the *ports inside it* are.

## Target shape

Two **internal seams**, private to the tools layer. They never appear in what a Workflow
Operation is handed:

```rust
trait ProcessRunner {
    fn run(&self, exe: &Path, args: &[OsString], cwd: &Path) -> Result<ExitStatus>;
}

trait Wait {
    fn sync_delay(&self, seconds: u64);
}
```

One **external seam**, expressed as data rather than an interface:

```rust
struct OperationPorts<'a> {
    ck:      &'a CreationKitOps,   // concrete deep module — deliberately not a trait
    prompts: &'a dyn Prompts,
    files:   &'a dyn FileSpace,
}

type OperationExecution = fn(&WorkflowRun, &OperationPorts<'_>) -> Result<()>;
```

`CreationKitOps` becomes a concrete deep module holding its own narrow inputs (CK exe,
Fallout 4 dir, CK log path, session log path). Four domain methods over a shared private
`run`:

```rust
fn generate_precombined(&self, plugin_file: &str, mode: BuildMode) -> Result<CkRun>;
fn compress_psg(&self, plugin_file: &str)          -> Result<CkRun>;
fn build_cdx(&self, plugin_file: &str)             -> Result<CkRun>;
fn generate_previs_data(&self, plugin_file: &str)  -> Result<CkRun>;

struct CkRun { log: Option<String> }
```

The private `run` owns the whole episode, in this order: DLL guard → delete CK log → spawn →
10s MO2 wait → read CK log → append to session log → warn on non-zero exit. It returns the
log content; `None` means the log was absent, which the batch distinguishes explicitly
("Unable to find log").

The Workflow Operation keeps every success criterion: the expected-output-file check, the
handle-array scan, the mode-conditional PSG check.

`FileSpace` gains five operations: `rename` and `exists` for the DLL guard, and
`write` / `append` / `temp_dir` so `logging` stops writing into the machine's real `%TEMP%`
during tests. This revisits the module doc's deliberate omission of a write operation — that
omission was correct when nothing needed one, so issue `02` updates the reasoning rather than
deleting it.

`exists` and `is_file` stay distinct. `DllGuard`'s `Drop` uses `exists` to ask "has something
reappeared at this path", where a directory counts; narrowing it to `is_file` would turn a
deliberate skip-with-warning into an attempted rename that cannot succeed. On
`InMemoryFileSpace` the two necessarily coincide — it is a flat map with no directory concept,
by design — so any test that turns on the file/directory distinction stays on
`SystemFileSpace` with `tempfile`, which is where `src/files.rs` already says that
discrimination is verified.

## Decisions, and what was rejected

| Decision | Chosen | Rejected, and why |
|---|---|---|
| Scope | CK episode only; FO4Edit/archive designed in Phase B against real callers | Designing all five adapter kinds now — one caller means a hypothetical method, same rule as one adapter meaning a hypothetical seam |
| Spawn | Behind a `ProcessRunner` port | Leaving spawn concrete and hoisting the ordering out of the adapter — that scatters the four obligations `docs/workarounds.md` keeps in one place |
| Adapter inputs | Each adapter constructed with its own narrow inputs | Keeping `ToolContext` (a 5-field public bag whose `Default` builds an unusable value, and which `CONTEXT.md` lists under *Avoid*); passing `&WorkflowToolchain` (recreates the circularity one level down) |
| Seam vocabulary | Domain verbs — `generate_precombined(plugin, mode)` | `CkOperation` + qualifier string crossing the seam; `CONTEXT.md` says a Workflow Operation is the step's build meaning *before* an adapter is chosen |
| CK log | Adapter owns lifecycle and returns content; operation owns criteria | Adapter returning a verdict — contradicts ADR-0001, which rejected putting success criteria in tool adapters. The scan predicate *and severity* differ per step (handle-array is fatal, visibility-task is a warning, two operations scan nothing), so it is build meaning |
| Non-zero CK exit | Adapter emits the warning; `CkRun` carries no `exit_status` | Operation deciding — the batch treats it uniformly across all four operations inside `:RunCK`, and it is explicitly not a criterion. A field nothing reads is dead interface |
| Test substitution | At `ProcessRunner`/`Wait`; the real `CreationKitOps` runs | Faking a `CreationKit` port — that leaves the composition untested (which is the thing that has never been tested) and mints a one-adapter trait |
| Bundle | Plain struct of port references | Trait with per-episode methods (reaches ~8–10 by Phase B); trait as accessor bag (a shallow interface for something with one shape) |
| DLL guard | Onto `FileSpace` so its ordering relative to the spawn is assertable | Leaving it on `std::fs` — its current tests cannot see *when* it runs, which is what workaround §3 cares about |
| `exists` vs `is_file` | Add `exists` to `FileSpace`; keep both predicates exactly as the guard uses them | Collapsing `exists` into `is_file` — equivalent at `dll.rs:44`, but at `:68` it silently converts a deliberate skip into a failed restore |
| Directory-sensitive tests | Stay on `SystemFileSpace` + `tempfile` | Porting the rollback test to `InMemoryFileSpace` — it forces failure with a *directory*, which that adapter cannot model without contradicting its own design note |

## Out of scope

- **Phase B** — FO4Edit and archive methods. The episode enumeration (issue `10`) confirms
  why they cannot be designed now: FO4Edit needs async spawn plus process-handle control
  (`AppActivate`, `CloseMainWindow`, `TaskKill`) and an unbounded poll loop, and "append" is
  a fiction for *both* archive tools — BSArch runs the identical `Pack` command in steps 3
  and 8 and differs only by retaining staged content on disk between them.
- **Registration ceremony** (the macro, the five single-caller forwarders,
  `ProductionWorkflowPreparation`). Issue `06` makes the minimal `OperationExecution`
  signature change and leaves the rest alone. Nothing here makes that cleanup harder later.
- **General `FileSpace` adoption.** Issue `03` routes `logging` only — that is Phase A paying
  for its own test isolation, not a sweep of the ~30 other direct `std::fs` sites.
- **Splitting `toolchain.rs`, dissolving `validation.rs`, narrowing the prompt seam.**
  Separate candidates.

## Issue order

Ordering is forced: ports before the CK reshape, CK reshape before the trait goes away.

`01` ports · `02` FileSpace widening · `03` logging through FileSpace · `04` DllGuard onto
FileSpace · `05` CreationKitOps reshape · `06` OperationPorts replaces the trait ·
`07` handle-array marker fix · `08` delete the stubs · `09` ADR-0002 and CONTEXT.md.

`10` (episode enumeration doc) and `11` (V2.98 doc sync) are independent and can run at any
time. `12` (`xFOEdit.exe` discovery candidate) and `13` (the registry-absent Wine path) were
raised out of `11`'s sweep and are likewise independent — both are `discovery` parity questions,
not part of this refactor.

## Done when

- Step 1 tests run entirely in-memory: no real process spawn, no real sleep, no writes to the
  machine's `%TEMP%`.
- The CK episode's mandated ordering — DLL guard, log delete, spawn, 10s wait, log append,
  non-zero-exit warning — is asserted, in order, by a test.
- `WorkflowRun::tool_context()` no longer exists.
- No `CkOperation` or qualifier string appears in `src/workflow/`.
- `cargo test` and `cargo clippy` are clean.
