# FO4Edit window probe (throwaway)

Probe for map ticket [Probe FO4Edit's windows and dismissal on Windows](https://github.com/evildarkarchon/generateprevisibines/issues/40).
Not part of the main crate. It has its own `[workspace]` and may use `unsafe`.

## What a scenario does

1. Writes a stub `GPProbe.esp` (a `TES4` header only) into `Fallout 4\Data`, plus
   `%TEMP%\GPProbePlugins.txt` and `%TEMP%\GPProbe_WindowProbe.pas`.
2. Launches FO4Edit with the batch's flags:
   `-fo4 -autoexit -P:… -Script:… -Mod:GPProbe.esp -log:%TEMP%\GPProbe.log`.
3. Waits for a visible `Module Selection` window owned by that PID, then counts down so you
   can set the focus.
4. Tries **one** dismissal method. If it hasn't dismissed the dialog after 15s, it asks you to
   press OK.
5. Waits for the log (the probe script adds a `GPProbeMarker` GLOB to `GPProbe.esp` in memory
   and writes the log the way the PJM scripts do).
6. Waits 10s, then runs the close sequence from the FO4Edit design: `WM_CLOSE`, wait 15s,
   `WM_CLOSE` again, wait 10s. If FO4Edit is still running after that, it asks you to close it
   by hand. **It never kills FO4Edit.**
7. Checks that the marker reached `GPProbe.esp` on disk, and lists any `GPProbe.esp.save.*`
   files and `FO4Edit Backups\GPProbe.esp*` files. It then deletes all of them.
8. Writes `reports/<unix>-<method>-<focus>-<close>.json`.

## Run

From a normal terminal (Windows Terminal or conhost), **not** elevated:

```powershell
cd probes\fo4edit-window-probe
cargo run --release -- matrix
```

`matrix` runs seven scenarios and prompts before each one. Follow the FOCUS line it prints.
Run a single scenario with
`cargo run --release -- run --method <bm-click|wm-command|post-enter|send-input|manual> --focus <console|away> [--close all|visible|main]`.
`dump <pid>` prints a window snapshot of any process.
