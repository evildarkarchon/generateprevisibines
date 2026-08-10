# 02 — Widen `FileSpace` with rename, write, append and temp root

Status: ready-for-agent
Blocked by: none

## Why

`FileSpace` (`src/files.rs:24`) is the narrowest, best-specified interface in the crate —
documented invariants per method, one test per invariant, two adapters. Its problem is reach:
five call sites in `precombine_workspace`, against ~30 production sites that touch
`std::fs` directly.

Phase A needs five more operations to bring two of those bypassers inside:

- `rename` and `exists` — so `DllGuard` (issue `04`) runs in-memory and its *ordering
  relative to the spawn* becomes assertable. Its current tempfile tests can see that it
  renames; they cannot see when.
- `write` / `append` / `temp_dir` — so `logging` (issue `03`) stops writing into the
  machine's real `%TEMP%` during tests.

The module doc at `src/files.rs:1-10` deliberately omits a write operation. That was correct
when nothing needed one; it is being revisited because two callers now do. Update the doc to
say so rather than deleting the reasoning.

## Work

Add to the trait, each with the same invariant-level doc comments the existing methods have
(error modes, idempotency, case-sensitivity):

```rust
/// Rename `from` to `to`. Fails if `from` does not exist.
/// Does **not** create parent directories.
fn rename(&self, from: &Path, to: &Path) -> Result<()>;

/// Whether anything exists at `path` — file **or** directory.
///
/// Deliberately distinct from `is_file`. `DllGuard` uses it to detect that something has
/// reappeared at a path it is about to write to, and a directory counts; narrowing it to
/// `is_file` would turn a deliberate skip into an attempted rename that cannot succeed.
fn exists(&self, path: &Path) -> bool;

/// Write `contents` to `path`, replacing any existing file. Creates parent directories.
fn write(&self, path: &Path, contents: &str) -> Result<()>;

/// Append `contents` to `path`, creating the file if absent.
fn append(&self, path: &Path, contents: &str) -> Result<()>;

/// Root for per-run temporary files. `SystemFileSpace` returns `std::env::temp_dir()`;
/// `InMemoryFileSpace` returns a synthetic root so tests never touch the real one.
fn temp_dir(&self) -> PathBuf;
```

Implement all five on `SystemFileSpace` and `InMemoryFileSpace`. Add one test per documented
invariant on each adapter, matching the existing convention at `src/files.rs:261-376` — in
particular: rename of a missing source errors; append to a missing file creates it; write
replaces rather than appends.

**`exists` on `InMemoryFileSpace` coincides with `is_file`, and that is correct.** That
adapter is a flat `BTreeMap<PathBuf, Option<String>>` with no directory concept, by explicit
design — its doc at `src/files.rs:122-127` records that directory-versus-file discrimination
"[is] verified against `SystemFileSpace`, where they actually live." Do not add directory
modelling to make the two differ. Document the coincidence on the in-memory impl and point at
that reasoning, so the next reader does not "fix" it.

The consequence for issue `04`: any test that turns on the file/directory distinction stays
on `SystemFileSpace` with `tempfile`. Only `SystemFileSpace` needs a test where `exists` and
`is_file` disagree — a directory at the queried path.

## Notes

- Do **not** route the other ~30 direct `std::fs` sites through `FileSpace` here. That is a
  separate candidate. Only `logging` (issue `03`) and `DllGuard` (issue `04`) move in
  Phase A.
- `SystemFileSpace::read_lossy` already delegates to `crate::text::read_lossy`; leave that
  alone.

## Done when

- Five methods on the trait, both adapters, one test per documented invariant.
- `SystemFileSpace` has a test where `exists` is true and `is_file` is false.
- The module doc explains why a write operation now exists.
- `cargo test` and `cargo clippy` are clean.
