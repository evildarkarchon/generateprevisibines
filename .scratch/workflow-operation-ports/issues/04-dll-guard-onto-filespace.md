# 04 — Move `DllGuard` onto `FileSpace`

Status: ready-for-agent
Blocked by: 02

## Why

`DllGuard` (`src/tools/dll.rs:26`) is the deepest module in `src/tools/` — RAII restore on
drop, restore-skip when the original path reappeared, log-don't-panic on restore failure,
partial-failure rollback. It has two tempfile tests, and they are good ones.

What they cannot see is **when** it runs. Workaround §3 is not "the DLLs get renamed" — it is
"the DLLs are renamed *before* Creation Kit launches and restored *after* it exits, on every
exit path including failure". That ordering is only observable from a test that also observes
the spawn, which issue `05` makes possible — but only if the guard and the recording process
runner share one filesystem.

## Work

`DllGuard::disable` takes a `&dyn FileSpace`; the guard holds it for `Drop`:

```rust
pub(crate) struct DllGuard<'a> {
    files: &'a dyn FileSpace,
    pairs: Vec<RenamePair>,
}

impl<'a> DllGuard<'a> {
    pub(crate) fn disable(files: &'a dyn FileSpace, fallout4_dir: &Path) -> Result<Self>;
}
```

Replace the seven direct calls, preserving each predicate exactly:

| Site | Was | Becomes |
|---|---|---|
| `:37` | `original.is_file()` | `files.is_file` |
| `:44` | `disabled.exists()` | `files.exists` |
| `:45` | `std::fs::remove_file` | `files.remove_file` |
| `:48` | `std::fs::rename` | `files.rename` |
| `:65` (Drop) | `disabled.is_file()` | `files.is_file` |
| `:68` (Drop) | `original.exists()` | `files.exists` |
| `:75` (Drop) | `std::fs::rename` | `files.rename` |

### `exists` is not interchangeable with `is_file` here

Issue `02` adds `exists` for this reason. Do **not** substitute `is_file` at `:44` or `:68`.

At `:68` the two are genuinely different. The guard is asking "has something reappeared at
the original path?" — and a directory counts. Under `is_file`, a directory there goes
undetected, so the guard attempts a rename that cannot succeed: a deliberate
**skip-with-warning** (`"DLL restore skipped: original path already exists"`) becomes a
reported **failure** (`"failed to restore DLL after CK run"`).

At `:44` the outcome happens to survive the swap — the failure just moves from `remove_file`
to `rename` — but there is no reason to accept a different mechanism when `exists` is
available.

### Test placement — do not port both tests

- `renames_and_restores_on_drop` (`:94-109`) → port to `InMemoryFileSpace`.
- `restores_previous_renames_when_disable_fails` (`:112-131`) → **keep on `SystemFileSpace`
  with `tempfile`.** It forces the rename to fail by creating a *directory* at
  `d3d9.dll-PJMdisabled`, and `InMemoryFileSpace` is a flat `BTreeMap<PathBuf, Option<String>>`
  with no directory concept — deliberately, per `src/files.rs:122-127`. Porting it would
  require teaching that adapter about directories, contradicting the design note that says
  file-versus-directory discrimination is verified against `SystemFileSpace` "where [it]
  actually live[s]".

`DllGuard::disable` takes `&dyn FileSpace`, so passing `SystemFileSpace` in that one test
costs nothing.

## Notes

- `Drop` cannot return an error, and the existing code correctly logs rather than panics
  (`:75-82`). Preserve that. `FileSpace::rename` returning `Result` does not change it.
- The lifetime on `DllGuard<'a>` propagates into `CreationKitOps`'s private `run` in issue
  `05`. That is expected — the guard must not outlive the file space.

## Done when

- No `std::fs` call remains in `src/tools/dll.rs`.
- Every predicate is unchanged: `exists` at `:44` and `:68`, `is_file` at `:37` and `:65`.
- `renames_and_restores_on_drop` passes against `InMemoryFileSpace`;
  `restores_previous_renames_when_disable_fails` still passes against `SystemFileSpace`.
- `cargo test` and `cargo clippy` are clean.
