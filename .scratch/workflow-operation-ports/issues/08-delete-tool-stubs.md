# 08 — Delete the FO4Edit and archive stubs, keep their reference detail

Status: ready-for-agent
Blocked by: none

## Why

`Fo4EditOps` (`src/tools/fo4edit.rs:15`) and `ArchiveOps` (`src/tools/archive.rs:15`) have
**zero construction sites** anywhere in the repo — only the definitions, the `impl` blocks,
and the re-exports at `src/tools/mod.rs:11-14`. Both methods return
`Error::Other("... not yet implemented")`.

They are `pub` in the crate's API: published interface with no implementation, which is the
worst possible depth ratio. Phase B designs these against a real caller (step 2 for FO4Edit,
step 3 for archive), so the code is pure noise until then.

What is **not** noise is their doc comments, which carry operational detail
`docs/workarounds.md` does not: the exact FO4Edit flag set
(`-fo4 -autoexit -P: -Script: -Mod: -log:`), `%TEMP%\Plugins.txt`, BSArch's `-mt -fo4 -z`,
and Archive2's `-compression=XBox`.

## Work

- Delete `src/tools/fo4edit.rs` and `src/tools/archive.rs` and their `mod` / `pub use` lines
  in `src/tools/mod.rs`.
- Before deleting, move the flag detail into `docs/future-features.md`, attached to the
  slices that will implement it (slice 3 for FO4Edit, slices 4 and 8 for archive) — that is
  where a maintainer picking the next slice will actually look.
- Issue `10` supersedes most of this detail with a fuller per-step enumeration. If `10` has
  already landed, point `future-features.md` at that document instead of duplicating.

## Notes

- `src/tools/mod.rs` still holds `ToolContext`, which issue `06` deletes. If `06` has landed
  first, `mod.rs` may be left holding only re-exports — collapse it if so.

## Done when

- Neither file exists; no dangling `pub use`.
- No flag detail was lost — every line of the two doc comments is findable in `docs/`.
- `cargo test` and `cargo clippy` are clean.
