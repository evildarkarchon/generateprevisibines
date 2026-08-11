# 14 — no counterpart to V2.98's `Fallout4.exe` existence check (batch line 63)

Status: ready-for-agent
Blocked by: none

Surfaced by code review while resolving issue `13`. Split out rather than absorbed, because it
is a design decision about where a check belongs, not the error-wording fix `13` was scoped to.

## Why

The batch validates the resolved install directory before doing anything with it:

```
63: If Not Exist "!locCreationKit_!Fallout4.exe" (echo ERROR - !locCreationKit_!Fallout4.exe cannot be found. To Fix Run Fallout4Launcher.exe once,
64: echo Or use "-FO4:<dir>" command parameter to specify the location of Fallout4.exe
65: goto PauseAndExit)
```

Note this guards *both* sources of the directory — the registry answer and `-FO4:<dir>`.

`WorkflowToolchainProbe::from_tool_paths` only tests whether a directory was resolved at all. A
directory that exists in name only passes:

- a typo'd `--FO4 <DIR>`
- a stale registry entry left behind by a moved or uninstalled game

Both then proceed against a nonexistent `Data` directory. Nothing is silently wrong — the
`CreationKit.exe` check (batch line 80, `toolchain.rs`) stops the run — but it stops later and
with "CreationKit.exe not found in `<dir>`", which reads as "Creation Kit is not installed" when
the real problem is that `<dir>` is not a Fallout 4 install. The batch's two remedies are the
actionable ones and the user never sees them.

## Decide first

Where does the check go? `from_tool_paths` is the obvious site but not a free one:

- Six probe fixtures across `intake.rs`, `run.rs` and `toolchain.rs` construct `ToolPaths` with
  synthetic directories (`C:\Fallout4`, `D:\Games\Fallout4`) specifically to keep plugin-readiness
  and data-root tests off the filesystem. An existence check in the constructor forces every one
  of them onto a tempdir.
- The alternative is checking in `discover_tools` (so `ToolPaths.fallout4_dir` means "a directory
  holding `Fallout4.exe`") and leaving `from_tool_paths` a pure view over already-validated
  paths. That keeps the fixtures honest but makes the override path silently fall back to `None`
  unless the error is threaded out separately, which loses which of the two sources failed.

Neither is obviously right. Pick one deliberately.

## Done when

- A directory that exists but holds no `Fallout4.exe` is rejected with batch line 63's wording
  and both remedies, from the registry path and the `--FO4` path alike.
- `docs/behaviors.md`'s "Host Support" section drops its note that line 63's `Exist` test has no
  counterpart.
- Issue `13`'s message can then adopt line 63's own "Fallout4.exe cannot be found" wording, which
  it currently avoids precisely because the check behind it does not exist.
