# 12 — `discovery` misses the `xFOEdit.exe` candidate V2.98 probes first

Status: resolved
Blocked by: none

Raised by the V2.98 doc sweep (issue `11`), which was documentation-only by charter. This is a
code change, so it was deliberately not absorbed there.

## Why

V2.98's `:xEditCheck` block probes five executable names in the script's own directory, in
order (lines 26–35):

```
xFOEdit.exe        <- line 26, tried FIRST
FO4Edit64.exe      <- 28
xEdit64.exe        <- 30
FO4Edit.exe        <- 32
xEdit.exe          <- 34
```

`FO4EDIT_CANDIDATES` in `src/discovery.rs:8` carries only the last four:

```rust
const FO4EDIT_CANDIDATES: &[&str] = &["FO4Edit64.exe", "xEdit64.exe", "FO4Edit.exe", "xEdit.exe"];
```

`xFOEdit.exe` appears to be new in V2.98 — the batch header for that version reads
`(xEdit 4.1.5q, Support for Wine, MO2 error Checks, ...)`, and the Rust list matches V2.96's
four exactly.

## Impact

Two distinct failures, both on an install that has `xFOEdit.exe`:

1. If `xFOEdit.exe` is the *only* xEdit executable present, `discover_fo4edit` falls through to
   the registry, and then to `Error::Other("FO4Edit/xEdit directory not found…")` — the batch
   would have found it.
2. If `xFOEdit.exe` sits alongside e.g. `FO4Edit64.exe`, the two disagree on *which* binary to
   run. The batch takes `xFOEdit.exe`; the Rust port takes `FO4Edit64.exe`.

Case 2 also moves `BSArch.exe` discovery, which `discover_tools` derives from the resolved
FO4Edit's parent directory — though in practice both live in the same folder.

## Work

Add `"xFOEdit.exe"` at the **front** of `FO4EDIT_CANDIDATES`; order is the batch's contract, not
incidental. Add a unit test alongside `finds_fo4edit_in_exe_dir` covering the precedence: a
directory holding both `xFOEdit.exe` and `FO4Edit64.exe` resolves to `xFOEdit.exe`.

## Done when

- `discover_fo4edit` probes the same five names in the same order as batch lines 26–35.
- A test pins `xFOEdit.exe` ahead of `FO4Edit64.exe`.

## Comments

Resolved. `FO4EDIT_CANDIDATES` now lists all five names in batch order with a doc comment
recording that the order is a parity contract, and `prefers_xfoedit_over_fo4edit64` pins the
precedence (red before the constant changed, green after). No other candidate list existed in
the tree to keep in sync. `cargo test` (141 lib + 1 integration) and `cargo clippy
--all-targets` are clean.
