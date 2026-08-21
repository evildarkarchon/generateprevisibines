# 16 — the session log loses `:RunCK`'s per-run framing

Status: resolved
Blocked by: none

Surfaced by code review while resolving issue `13`. Pre-existing, unrelated to that change.

## Why

The batch's `:RunCK` writes five things to the session log around each Creation Kit run
(lines 452–461):

```
452: echo Running CK option %1: >> "%Logfile_%"
453: echo ==================================== >> "%Logfile_%"
454: ECHO Start %time% >> "%Logfile_%"
455: START "CK" ... (the run itself)
457: ECHO Ended %time% >> "%Logfile_%"
460: If Not Exist "%CreationKitlog_%" ECHO Unable to find log  %CreationKitlog_% >> "%Logfile_%"
461: If Exist "%CreationKitlog_%" type "%CreationKitlog_%" >> "%Logfile_%"
```

`CreationKit::run` (`src/tools/creation_kit.rs:220`) ports only line 461. `logging::append_ck_log`
writes its own `----- Creation Kit log -----` banner, but the append is gated on the log
existing:

```rust
let log = self.read_ck_log()?;
if let (Some(session), Some(contents)) = (&self.session_log, &log) {
    logging::append_ck_log(session, contents, self.files)?;
}
```

So nothing at all is written when CK produced no log, which is exactly when the log matters
most. A Creation Kit that crashes before writing anything leaves a session log containing only
the `Starting <mode> Build V2.95 of <plugin>` header — no record that a run was even attempted,
which operation it was, or how long it ran.

Note the `CkRun` doc comment at `src/tools/creation_kit.rs:248` already cites the batch's "Unable
to find log" as the meaning of a `None` log. The distinction is modelled correctly in the type;
it just never reaches the file.

## What is missing

- **Which operation ran** (452). Four operations share one session log across a build, and
  without the banner an appended CK log cannot be attributed to any of them.
- **`Start` / `Ended` timestamps** (454, 457). The batch's only record of how long a step took.
- **`Unable to find log`** (460). The one line that distinguishes "CK ran and said nothing" from
  "CK never ran", currently indistinguishable in the file.

## Notes

- Timestamps mean `run` needs a clock. There is an existing port precedent for this shape — the
  `Wait` port (`src/tools/wait.rs`) exists so tests are not at the mercy of real time — so a
  clock port rather than a direct `SystemTime::now()` call is likely the consistent choice. The
  batch's `%time%` is local wall-clock time of day, not a duration.
- `logging::append_ck_log` currently owns the banner and trailing-newline normalisation. Decide
  whether the new framing joins it there (so one function owns the whole per-run block, and the
  no-log case becomes a variant of it) or wraps it at the `run` call site. The former keeps the
  "one block per append" property the function's comment claims.
- `append_ck_log_frames_the_creation_kit_contents` (`src/logging.rs:164`) asserts the exact
  session-log contents, so it will need updating alongside.

## Done when

- A Creation Kit run appends its operation banner and separator, `Start`/`Ended` timestamps, and
  either the log contents or the batch's `Unable to find log` line — including when CK wrote no
  log and when it exits non-zero.
- Timestamps come through a port that tests can control, not a direct system-clock call.

## Answer

`logging` now owns three functions, one per group of batch lines, and `CreationKitOps::run` calls
each where `:RunCK` calls it:

- `append_ck_run_header` — `Running CK option <op>:`, the 36-`=` rule, `Start <t>` (452–454),
  written **before** the spawn
- `append_ck_run_ended` — `Ended <t>` (457), written after the spawn returns and before the MO2
  delay
- `append_ck_log` — the log contents, or `Unable to find log  <path>` with the batch's two spaces
  (460–461), written after the delay

The notes floated a single function owning one contiguous block, and that was tried first. It
does not work: deferring every write until the run returns means a spawn that fails, or a log
that cannot be read, leaves the session log with *nothing* — which is the failure this issue was
filed about. The batch's placement is load-bearing, not incidental. Splitting on the batch's own
boundaries also gave `append_log_line` its first production caller, so its `dead_code` allow is
gone.

The invented `----- Creation Kit log -----` banner is gone. It existed only because nothing else
marked where a CK log began, and keeping it alongside the real framing would have been a
divergence from batch output with no remaining purpose. **This removal was not asked for by the
issue** — revert it if a session log is expected to carry that banner.

One state the batch has no line for: a log that exists but cannot be read. That stays an error to
the caller, and the session log keeps its header and both timestamps with no log line after them,
rather than being told the log was absent.

Timestamps come from a new `Clock` port (`src/tools/clock.rs`), an internal tools-layer seam
alongside `Wait` for the reason ADR-0002 gives — what time of day a run started is not build
meaning. `SystemClock` reads `chrono::Local`; `ScriptedClock` hands tests a scripted sequence,
holding its final reading so a fixture that does not care about time can supply one and stop
thinking about it. `chrono` is a new dependency: `std::time` has no calendar breakdown at all,
and a local time zone needs OS calls this crate cannot make under `unsafe_code = "forbid"`.

`the_creation_kit_episode_keeps_its_mandated_order` asserts the whole sequence, including all
three appends and both clock readings, through a `TracingClock` that records each reading onto
the shared timeline. Two tests pin the error paths directly: a failed spawn and an unreadable log
both still leave a session-log entry.

Adding a fourth port put `CreationKitOps::new` over `clippy::too_many_arguments`, so the four
seams travel as a `CkPorts` struct and `bind` takes it. That touched the three call sites in
`run.rs` and `operations.rs`.

`SystemClock`'s reading is zero-padded (`09:05:03.21`) where `cmd`'s `%time%` space-pads the hour
below ten. Reproducing the locale quirk would mean carrying the user's locale settings for no
reader's benefit; the divergence is noted in the adapter. Trailing whitespace is not reproduced
either — every `echo … >> "%Logfile_%"` in `:RunCK` leaves a space before the redirect, and
`init_session_log` already drops the batch header's two.
