# 01 — Add the `ProcessRunner` and `Wait` internal seams

Status: resolved

Two new ports below the tools layer. Nothing consumes them yet — issue `05` wires them into
`CreationKitOps`. They are **internal seams**: they must not appear in `OperationPorts` or in
any Workflow Operation signature.

## Why

`CreationKitOps::run` (`src/tools/creation_kit.rs:38`) sequences six mandated things and none
of them is tested, because the only way to avoid launching Creation Kit today is to replace
the whole adapter. Behind these two ports, the sequence becomes assertable.

The wait is not incidental. The batch has **nine** distinct wait sites at three durations
(5s / 10s / 15s): 10s after every CK exit, and ~50s of fixed delay per xEdit run. A test
suite that really sleeps is not viable, and `#[cfg(test)]`-ing the constants to zero would
make the tests assert a production behaviour that isn't there — the delays are required
(`docs/workarounds.md` §2) and deserve a test proving they still happen.

## Work

New module `src/tools/process.rs`:

```rust
pub(crate) trait ProcessRunner: std::fmt::Debug {
    /// Spawn `exe` with `args`, working directory `cwd`, and wait for it to exit.
    ///
    /// Equivalent to the batch `START "..." /D"<cwd>" /wait <exe> <args>`.
    /// Returns the exit status; a non-zero status is **not** an error here — exit-code
    /// policy belongs to the caller.
    fn run(&self, exe: &Path, args: &[OsString], cwd: &Path) -> Result<ExitStatus>;
}

#[derive(Debug, Default)]
pub(crate) struct SystemProcessRunner;
```

`SystemProcessRunner::run` is the `std::process::Command` implementation lifted from
`creation_kit.rs:52-57` — `Command::new(exe).current_dir(cwd).args(args).status()`.

New module `src/tools/wait.rs`, holding the port and absorbing today's `src/timing.rs`:

```rust
pub(crate) trait Wait: std::fmt::Debug {
    /// Sleep for `seconds` to let the MO2 virtual filesystem settle.
    ///
    /// Required, not incidental — see docs/workarounds.md §2. Callers must not skip it.
    fn sync_delay(&self, seconds: u64);
}

#[derive(Debug, Default)]
pub(crate) struct SystemWait;
```

`MO2_DELAY_AFTER_CK_SECS` and `MO2_DELAY_AFTER_SEED_COPY_SECS` move here from `timing.rs`,
keeping their doc comments and the `docs/workarounds.md` traceability. Delete `src/timing.rs`
and re-point `src/interactive.rs:69` at the new constant — leave that call site on a direct
`SystemWait` for now; routing intake through the port is candidate 7, not Phase A.

Test doubles live in a `#[cfg(test)] pub(crate)` module so `src/workflow/` can reach them
(same pattern as `recording_adapters.rs` today):

- `RecordingProcessRunner` — records every `(exe, args, cwd)` in order, returns a
  configurable `ExitStatus`, and can be given a callback that simulates the tool's filesystem
  effects **through a `FileSpace`** (see issue `05` for why: CK returns nothing, it writes
  files).
- `RecordingWait` — records every `seconds` value in order, sleeps for nothing.

## Notes

- `ProcessRunner::run` is deliberately minimal. FO4Edit will need async spawn plus a process
  handle (`AppActivate` / `CloseMainWindow` / `TaskKill` / poll) — that widening happens in
  Phase B against a real caller, not now.
- `cwd` is not optional: every CK invocation sets `/D"<fo4dir>"` and every Archive2 call runs
  with cwd = `Data`.

## Done when

- Both ports exist with a system adapter and a recording adapter.
- `src/timing.rs` is gone and its constants live in `src/tools/wait.rs`.
- `cargo test` and `cargo clippy` are clean. No behaviour change yet.
