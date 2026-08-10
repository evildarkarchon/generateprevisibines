# 07 — Fix the handle-array marker divergence

Status: ready-for-agent
Blocked by: 06

## Why

A live behavioural divergence in shipped code, in the module Phase A is already rewriting.

`src/workflow/operations/precombine_workspace.rs:9` matches the CK log against
`DEFAULT: OUT OF HANDLE ARRAY ENTRIES`. The batch does:

```
Findstr /I /M /C:"OUT OF HANDLE ARRAY ENTRIES" "<CK log>"
```

Two differences, both making the Rust strictly narrower:

1. The batch does not require the `DEFAULT: ` prefix. A log line carrying the message under a
   different prefix passes our check and fails the batch's.
2. `/I` is case-insensitive; our comparison is not.

The consequence is a false success — the run continues past a Creation Kit failure that the
batch would have caught, and the user gets a broken build rather than "ran out of Reference
Handles".

## Work

Match the batch: case-insensitive substring search for `OUT OF HANDLE ARRAY ENTRIES`, no
prefix. Keep the constant named and doc-commented with the batch line reference.

Add tests covering: the `DEFAULT: ` prefixed form (must still match), an unprefixed form,
a lowercase form, and a log with no marker at all.

## Notes

- Do **not** generalise this into a shared "scan the CK log for a marker" helper. The other
  scan the workflow needs (`ERROR: visibility task did not complete.`, step 6) is a
  **warning**, not a failure — different severity, different step. Wait for step 6 to exist.
- This is the only behaviour change in Phase A. Everything else is structural.

## Done when

- The match is case-insensitive and prefix-free.
- Four tests as above.
- `cargo test` and `cargo clippy` are clean.
