# 06 — Replace the `OperationAdapters` trait with an `OperationPorts` struct

Status: ready-for-agent
Blocked by: 05

## Why

`OperationAdapters` (`src/workflow/operations.rs:203`) is a trait with exactly one real
implementation. By the rule used throughout this repo — one adapter means a hypothetical
seam, two means a real one — the bundle is not a seam. The ports *inside* it are: `FileSpace`
has two adapters, `Prompts` will have two, and `CreationKitOps` sits above two more.

Two further tells. `ProductionOperationAdapters` (`:224`) holds no state whatsoever — both
its fields are unit structs, so it is a zero-sized type that exists to own a vtable. And
`files()` (`:219`) returns a port *through* a port, which its own doc comment has to
apologise for.

Make the bundle data instead of an interface.

## Work

### The struct

```rust
pub(crate) struct OperationPorts<'a> {
    pub(crate) ck:      &'a CreationKitOps<'a>,
    pub(crate) prompts: &'a dyn Prompts,
    pub(crate) files:   &'a dyn FileSpace,
}

type OperationExecution = fn(&WorkflowRun, &OperationPorts<'_>) -> Result<()>;
```

`ck` is concrete, not `&dyn CreationKit`. Per decision Q10, tests substitute at
`ProcessRunner`/`Wait` and run the real `CreationKitOps`, so a `CreationKit` trait would have
exactly one adapter — the hypothetical seam this issue is removing.

### The prompt port

Extract the one prompt method into its own small trait, keeping it concrete:

```rust
pub(crate) trait Prompts {
    fn confirm_clear_precombined(&self, precombined_dir: &Path) -> Result<bool>;
}
```

Do **not** generalise to a `confirm(Question)` enum. There is one caller. The full workflow
has exactly three confirmations (clear precombined, clear vis at step 6, remove working files
at finish) and they are structurally identical, so generalising later is mechanical — do it
when step 6 lands.

### Wiring

- Delete the `OperationAdapters` trait and `ProductionOperationAdapters`.
- Build `OperationPorts` where `ProductionOperationAdapters::new()` was called, constructing
  `CreationKitOps` from the toolchain's resolved paths.
- **Delete `WorkflowRun::tool_context()`** (`src/run.rs:203`) and `ToolContext`
  (`src/tools/mod.rs:18`) along with its `Default` impl, which builds an unusable value out of
  empty `PathBuf`s.
- Update `generate_precombines::run` to call `ports.ck.generate_precombined(&config.plugin.file_name, config.build_mode)`
  and pass `CkRun.log` into the postcondition check. `precombine_workspace::validate_generated`
  takes the log **content** (`Option<&str>`) instead of a path — the operation keeps the
  handle-array scan, per ADR-0001.
- Delete `precombine_qualifiers` from `generate_precombines.rs`; that mapping now lives in
  `CreationKitOps`.

### Minimal blast radius

Issue `06` changes the `OperationExecution` signature and nothing else in
`src/workflow/operations.rs`. Leave the registration macro, the five single-caller
forwarders, and `ProductionWorkflowPreparation` exactly as they are — that is a separate
candidate, and nothing here makes it harder to do afterwards.

Also leave the dead `resume_step1` branch in `generate_precombines.rs:47-54` alone. It is
unreachable (Step 1 is the minimum of an `Ord`-derived enum, so a plan containing it always
has `resume_from` of `None` or `Some(GeneratePrecombines)`), but removing it is a Workflow
Plan concern, not a seam concern.

## Test migration — replace, don't layer

**Delete these four.** Each pays for a full `WorkflowRun::prepare` fixture — tempdir, a
written `CreationKit.exe`, a written `fallout4_test.ini`, a probe — only to re-assert an
error variant a 20-line `precombine_workspace` test already covers, adding no interaction
assertion:

- `step_one_reports_missing_combined_objects_from_operation` (`operations.rs:542`)
- `step_one_reports_missing_geometry_psg_from_operation` (`:551`)
- `step_one_reports_no_precombined_meshes_from_operation` (`:562`)
- `step_one_reports_handle_array_log_error_from_operation` (`:572`)

**Keep and strengthen** the four that assert ordering and interaction (`:516`, `:528`, `:583`,
`:596`). They can now additionally assert the DLL guard, the 10s wait, and the log lifecycle —
none of which was previously observable from a Step 1 test.

**Replace `recording_adapters.rs`.** `RecordingOperationAdapters` disappears — there is no
trait left to implement. The file becomes a recording `Prompts` alongside the
`RecordingProcessRunner` and `RecordingWait` from issue `01` and the existing
`InMemoryFileSpace`.

## Done when

- No `OperationAdapters` trait, no `ProductionOperationAdapters`, no `ToolContext`, no
  `WorkflowRun::tool_context()`.
- No `CkOperation` or qualifier string anywhere under `src/workflow/`.
- Step 1 tests run with no real spawn, no real sleep, and no writes outside their fixture.
- `cargo test` and `cargo clippy` are clean.
