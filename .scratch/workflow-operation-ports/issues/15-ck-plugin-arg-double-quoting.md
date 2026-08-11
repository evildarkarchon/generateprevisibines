# 15 — `operation_arg` sends Creation Kit a plugin name with literal quotes

Status: ready-for-agent
Blocked by: none

Surfaced by code review while resolving issue `13`. Pre-existing, unrelated to that change.

## Why

`src/tools/creation_kit.rs:273` builds the operation argument with quotes baked into the string:

```rust
fn operation_arg(operation: CkOperation, plugin_file: &str) -> String {
    format!("-{}:\"{plugin_file}\"", operation.flag())
}
```

That reads as a direct transcription of batch line 455:

```
455: START "CK" /D"%locCreationKit_%" /wait "%CK_%" -%1:"%PluginNameExt_%" %~3
```

but the two do not deliver the same thing, because each side quotes at a different layer.

**Batch.** `cmd` expands the variable and writes `-GeneratePrecombined:"MyMod.esp"` onto the raw
command line. Creation Kit parses that with `CommandLineToArgvW`, which strips the quote pair,
so CK sees `-GeneratePrecombined:MyMod.esp`.

**Rust.** `Command::arg` escapes on Windows: a `"` inside an argument is emitted as `\"`, so the
raw command line becomes `-GeneratePrecombined:\"MyMod.esp\"`. `CommandLineToArgvW` reads `\"`
as a *literal* quote character, so CK sees `-GeneratePrecombined:"MyMod.esp"` — quotes included,
as part of the plugin name.

The quoting in the batch exists to survive `cmd`'s own tokenizer, which is a layer Rust does not
have. `Command::arg` already guarantees one argv entry regardless of spaces, so the quotes are
not merely redundant here, they are wrong.

## Blast radius

Every Creation Kit operation, since all four route through `run`: `GeneratePrecombined`,
`CompressPSG`, `BuildCDX`, `GeneratePreVisData`. Whether CK tolerates the stray quotes is
unverified — it may strip them itself, or it may look for a file literally named `"MyMod.esp"`.
Confirm against a real CK before assuming either.

Names with spaces (legal in filtered and Xbox modes) are affected the same way: the argument
becomes `-GeneratePrecombined:"My Mod.esp"` with the quotes as characters, not delimiters.

## The test pins the current behaviour

`the_plugin_argument_stays_one_argv_entry_beside_separate_qualifiers`
(`src/tools/creation_kit.rs:512`) asserts:

```rust
OsString::from("-GeneratePrecombined:\"My Mod.esp\""),
```

Its docstring is about argument *count* — one argv entry for the plugin, separate ones for the
qualifiers — which is a real property worth pinning. But the literal it compares against also
cements the quoting, so a fix has to update this test and the three sibling assertions at lines
543, 560, 562 and 578. Keep the count property; change the expected literal.

## Done when

- Creation Kit receives the plugin name without literal quote characters, i.e.
  `format!("-{}:{plugin_file}", operation.flag())`, letting `Command::arg` handle any spaces.
- The argv-count property stays asserted; only the expected strings change.
- If CK turns out to *require* the quotes, record that in `docs/workarounds.md` instead and close
  this — but verify against the real tool first, do not infer it from the batch.
