# 03 — Route `logging` through `FileSpace`

Status: ready-for-agent
Blocked by: 02

## Why

There is a live test-isolation defect, and Phase A makes it worse.

`logging::session_log_path` (`src/logging.rs:11`) reads the process-global
`std::env::temp_dir()`, and `init_session_log` (`:39`) does a real `std::fs::write`. Every
test that builds a `WorkflowRun` therefore writes `<plugin>.log` into the machine's real
`%TEMP%`, outside any `TempDir` — and three separate test modules use the plugin name
`MyMod` (`src/run.rs:249`, `src/run.rs:355`, `src/workflow/operations.rs:483`). They collide
on one path. Issue `06` adds more Step 1 tests through the same fixture.

This is scoped narrowly on purpose: `logging` only. It is Phase A paying for its own test
isolation, not general `FileSpace` adoption.

## Work

Give every function in `src/logging.rs` a `&dyn FileSpace` parameter and replace the direct
`std::fs` calls:

| Function | Was | Becomes |
|---|---|---|
| `session_log_path` | `std::env::temp_dir()` | `files.temp_dir()` |
| `unattended_log_path` | `std::env::temp_dir()` | `files.temp_dir()` |
| `append_log_line` | `OpenOptions … .append(true)` | `files.append` |
| `init_session_log` | `std::fs::write` | `files.write` |
| `append_ck_log` | `is_file` + `read_lossy` + `OpenOptions` | `files.is_file` + `files.read_lossy` + `files.append` |

Thread the `FileSpace` from the existing call sites (`src/run.rs:126-131` and
`src/tools/creation_kit.rs:62`). `WorkflowRun::prepare` will need one — pass
`SystemFileSpace` from production and the in-memory space from tests.

Keep `append_ck_log`'s exact output framing (the blank line, the
`----- Creation Kit log -----` banner, and the trailing-newline normalisation at
`src/logging.rs:58-60`) — it is asserted by an existing test.

## Notes

- `build_session_header` still emits `V2.95` while the batch on disk is V2.98. **Leave it.**
  It is covered by issue `11`, which syncs all the version drift in one reviewable change.
- Do not change what gets logged, only where the bytes go.

## Done when

- No `std::env::temp_dir()` or `std::fs` call remains in `src/logging.rs`.
- Tests that construct a `WorkflowRun` write nothing outside their own fixture.
- The three `MyMod` test modules can run concurrently without touching a shared path.
- `cargo test` and `cargo clippy` are clean.
