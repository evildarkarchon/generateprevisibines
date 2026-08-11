# 05 — Reshape `CreationKitOps` into a concrete deep module

Status: resolved
Blocked by: 01, 02, 03, 04

The core of Phase A. `CreationKitOps` stops being a stateless unit struct that takes a
`ToolContext` per call, and becomes a deep module that owns the entire Creation Kit episode.

## Why

Today `run` (`src/tools/creation_kit.rs:38`) takes `&ToolContext` — a 5-field all-public bag
that `CONTEXT.md` lists under *Avoid* — plus `CkOperation` and a raw batch qualifier string.
The caller has to know CK's command grammar, and every line of the function is untested.

## Work

### Construction

```rust
#[derive(Debug)]
pub(crate) struct CreationKitOps<'a> {
    exe:          PathBuf,
    fallout4_dir: PathBuf,
    ck_log_path:  PathBuf,
    session_log:  Option<PathBuf>,
    process:      &'a dyn ProcessRunner,
    wait:         &'a dyn Wait,
    files:        &'a dyn FileSpace,
}
```

Narrow inputs, per decision Q8 — no `ToolContext`, no `&WorkflowRun`, no `&WorkflowToolchain`.

### Interface — four domain methods

```rust
pub(crate) fn generate_precombined(&self, plugin_file: &str, mode: BuildMode) -> Result<CkRun>;
pub(crate) fn compress_psg(&self, plugin_file: &str)         -> Result<CkRun>;
pub(crate) fn build_cdx(&self, plugin_file: &str)            -> Result<CkRun>;
pub(crate) fn generate_previs_data(&self, plugin_file: &str) -> Result<CkRun>;

pub(crate) struct CkRun {
    /// CK log contents after the run. `None` when CK wrote no log — a state the batch
    /// distinguishes explicitly ("Unable to find log").
    pub(crate) log: Option<String>,
}
```

Each method is three lines over a shared private `run(operation, plugin_file, qualifiers)`.
Qualifiers are derived inside, per the batch:

| Method | Qualifier |
|---|---|
| `generate_precombined` | `"clean all"` when `BuildMode::Clean`, else `"filtered all"` |
| `compress_psg` | empty |
| `build_cdx` | empty |
| `generate_previs_data` | **hardcoded `"clean all"`, regardless of build mode** — this is what the batch does; it is not a bug to fix here |

`CkOperation` stays as a private implementation detail of this module. It must not be `pub`
and must not appear anywhere in `src/workflow/`.

### The private `run`, in this exact order

1. `DllGuard::disable(self.files, &self.fallout4_dir)?`
2. Delete `ck_log_path` through `files` if present
3. `self.process.run(&self.exe, &args, &self.fallout4_dir)?`
4. `self.wait.sync_delay(MO2_DELAY_AFTER_CK_SECS)`
5. Read `ck_log_path` through `files` → `Option<String>`
6. Append it to `session_log` if both are present (existing `logging::append_ck_log` framing)
7. If the exit status is non-zero, `tracing::warn!` — **not** an error. Uniform across all
   four operations, exactly as the batch `:RunCK` does it
8. Guard drops → DLLs restored
9. Return `CkRun { log }`

Read the log **once** in step 5 and reuse the content for both the append and the return
value.

`CkRun` deliberately carries no `exit_status`: exit-code policy is settled inside `run`
(step 7), so a field would have no reader.

## Tests

This is where the previously untested mass gets covered. Using `RecordingProcessRunner`,
`RecordingWait` and `InMemoryFileSpace`:

- **Ordering** — DLLs are renamed *before* the recorded spawn and restored *after* it; the
  CK log is deleted before the spawn; the 10s delay is recorded after it and before the log
  append. Seed the in-memory space with `d3d11.dll` so the guard has something to move.
  *(The guard's file/directory edge cases stay on `SystemFileSpace` — see issue `04`. These
  tests only need the happy-path rename, which the in-memory space models fine.)*
- **Argv** — `-GeneratePrecombined:"My Mod.esp"` is one argument, and `clean all` arrives as
  two. Assert against the recorded `args`, not against a helper's return value. *(The current
  test at `creation_kit.rs:110` asserts the Rust `String` and says nothing about what reaches
  `CreateProcess`.)*
- **Qualifiers** — `Clean` → `clean all`, `Filtered` → `filtered all`, `Xbox` →
  `filtered all`; `compress_psg` and `build_cdx` pass none; `generate_previs_data` passes
  `clean all` for every build mode.
- **cwd** — every spawn uses `fallout4_dir`.
- **Log absent** — `CkRun.log` is `None` and nothing is appended to the session log.
- **Non-zero exit** — returns `Ok`, does not error.
- **Restore on failure** — a spawn error still restores the DLLs.

The recording process runner needs a hook to simulate CK's filesystem effects (writing the
CK log, `CombinedObjects.esp`, the precombined meshes) through the same `FileSpace` the
caller reads. That coupling is the point: it is what makes "CK wrote a log, the operation
read *that* log" verifiable for the first time.

## Done when

- Four domain methods; `CkOperation` and qualifier strings are private to this module.
- No `std::process`, `std::fs` or `std::thread::sleep` call remains in `creation_kit.rs`.
- The nine-step ordering above is asserted in order by a test.
- `cargo test` and `cargo clippy` are clean.

## Resolution notes

Done as specified. Five things worth flagging, four of them consequences the issue text did
not spell out.

**`logging::append_ck_log` changed signature.** Step 5's "read the log once and reuse the
content" is incompatible with a helper that takes a path and reads it itself, so it now takes
`contents: &str`. "No log at all" became the adapter's state to recognise rather than the
helper's. Two of its tests were reshaped accordingly and
`append_ck_log_writes_nothing_when_the_creation_kit_log_is_absent` was deleted — that
behaviour now lives in `CreationKitOps::read_ck_log`, covered by
`an_absent_creation_kit_log_yields_no_content_and_appends_nothing`. Its non-UTF-8 test no
longer applies either (the helper reads nothing); tolerant reading stays pinned on
`SystemFileSpace::read_lossy` in `files::tests`, and what remains asserts the framing through
the system space.

**The `OperationAdapters` trait had to move too**, because `CkOperation` must not appear in
`src/workflow/`. `run_creation_kit(run, operation, plugin_file, qualifiers)` became
`generate_precombined(run, plugin_file, build_mode) -> Result<()>`. The trait itself, the
`&WorkflowRun` parameter and `ToolContext` all still die in issue `06`; this is the smallest
change that gets CK's command grammar out of the workflow layer.

**Issue `06` has one less bullet.** `precombine_qualifiers` is already gone from
`generate_precombines.rs` — the qualifier mapping moved into `CreationKitOps` here, which is
what made the domain-verb signature possible.

**Three domain methods and `CkRun.log` carry `cfg_attr(not(test), allow(dead_code))`**, the
same pattern `logging::append_log_line` uses. `compress_psg`, `build_cdx` and
`generate_previs_data` wait on the Step 3/4/6 Workflow Operations; `CkRun.log` waits on issue
`06` threading it into the postcondition check, which is when `CkRun` also rejoins the
`tools::mod` re-export. Delete each allow with the arrival of its caller.

**Step 5's `Option<String>` is narrower than it first looks.** `None` means Creation Kit wrote
no log. A log that exists but cannot be read stays an error, as it was before the reshape —
collapsing the two would have been a live regression, because the caller's handle-array scan
clears a run it finds no marker in, so an unreadable log would have become a silent pass for a
failed Creation Kit run. `read_ck_log` returns `Result<Option<String>>` and
`a_present_but_unreadable_creation_kit_log_is_an_error` pins it.

**`std::process` survives in `creation_kit.rs` as one type import** — `ExitStatus`, in the
test module, for the local `FailingProcessRunner` that covers restore-on-spawn-failure. No
spawn, no `std::fs`, no `sleep`.
