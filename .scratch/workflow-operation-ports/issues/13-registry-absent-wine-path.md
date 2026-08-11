# 13 — `discovery` has no equivalent of V2.98's `reg.exe`-absent (Wine) path

Status: ready-for-agent
Blocked by: none

Second behavioural difference raised by the V2.98 doc sweep (issue `11`), which was
documentation-only by charter. Sibling of issue `12`; same module, different concern, so kept
separate — `12` is a one-line list fix with an obvious answer, this one needs a decision first.

## Why

V2.98's author header advertises **"Support for Wine"**. That support is a single guarded
probe. Before anything else the batch asks whether `reg.exe` exists at all:

```
21: WHERE /Q reg.exe
22: Set RegErr_=%ERRORLEVEL%
```

and then *both* registry lookups are skipped when it does not:

```
38: IF %RegErr_% EQU 1 goto xEditCheckFail    (before the HKCR\FO4Script\DefaultIcon query at 39)
49: IF %RegErr_% EQU 1 goto DoCheckParam      (before the HKLM Fallout4 "installed path" query at 50)
```

`src/discovery.rs` has no counterpart. It calls `winreg` directly in `fo4edit_from_registry`
and `discover_fallout4_dir`.

## What actually diverges

The two lookups fail differently, and only one of them matters much:

- **xEdit (line 38).** Near-equivalent already. `discover_fo4edit` swallows any registry error
  with `.ok()` and falls through to the same "FO4Edit/xEdit directory not found…" error the
  batch's `:xEditCheckFail` produces. No user-visible difference.
- **Fallout 4 (line 49).** Divergent. The batch leaves `locCreationKit_` empty and continues to
  the line-63 check, which reports the *actionable* message: `ERROR - <path>Fallout4.exe cannot
  be found. To Fix Run Fallout4Launcher.exe once, Or use "-FO4:<dir>" command parameter…`.
  `discover_tools` instead propagates the raw `winreg` error through `?`, so the user sees a
  registry error rather than the two concrete remedies. Note this is only reachable without
  `--FO4`: `discover_tools` consults `fallout4_override` first, so the override path never
  touches the registry in either implementation.

So the practical gap is **error quality on a registry-less host**, not a failure to find tools
that the batch would have found.

## Decide first

Is Wine/Proton a supported host for the Rust port at all? The answer changes the work:

- **If yes** — mirror the batch: treat "no registry" as a non-fatal empty result for the
  Fallout 4 path, and let the existing `Fallout4.exe`-not-found check produce the actionable
  message. That is the batch's own structure and needs no new error type.
- **If no** — do nothing to the code, but say so in `docs/behaviors.md`, because a reader
  comparing against the V2.98 header will otherwise keep re-finding this.

## Notes

- `WHERE /Q reg.exe` is a weaker test than it looks: it proves the binary is on `PATH`, not
  that a registry exists behind it. A Wine prefix with a stub `reg.exe` passes the probe and
  then returns nothing useful. Whatever is implemented should treat "query returned nothing"
  the same as "no registry", not just "binary missing".
- Do not conflate this with issue `12` (`xFOEdit.exe` candidate). Both live in `discovery`, but
  `12` is settled and this is not.

## Done when

- A decision on Wine support is recorded (here or in `docs/behaviors.md`).
- If in scope: a registry-less host reaches the `Fallout4.exe`-not-found message with its two
  remedies, not a raw `winreg` error, and a test covers that path.
