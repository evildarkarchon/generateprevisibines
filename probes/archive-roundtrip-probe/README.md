# Archive round-trip probe (throwaway)

Probe for the map ticket [Probe archive rebuild round trips and the work-folder swap on Windows](https://github.com/evildarkarchon/generateprevisibines/issues/44).
It isn't part of the main crate: it has its own `[workspace]` and may use `unsafe`.

It plays the Archive design ([#29](https://github.com/evildarkarchon/generateprevisibines/issues/29),
ADR-0004) against the real tools. It runs every pairing of a Step 3 tool with a Step 8 tool,
drawn from Archive2, BSArch 1.0 and BSArch 0.9x, so nine pairs.

| Step | Archive2 | BSArch |
|---|---|---|
| 3 | cwd `Data`, `Archive2 meshes\precombined -c=<fo4>\ArchiveWork\GPProbe - Main.ba2 -f=General -q`; check; RD loose; swap | move `Data\meshes\precombined` → `ArchiveWork\meshes\precombined`; 5s; `BSArch pack <ArchiveWork> <ArchiveWork>\GPProbe - Main.ba2 -mt -fo4 -z`; check; drop staged loose; swap |
| 8 | extract into `Data` (`-e=.`); 5s; `Archive2 meshes\precombined,vis -c=<ArchiveWork>\…`; check; swap; RD loose | `BSArch unpack <Data>\GPProbe - Main.ba2 <ArchiveWork>`; move `Data\vis` in; 5s; pack as above; check; swap |

Here **swap** means: delete `Data\GPProbe - Main.ba2` if present, then `std::fs::rename` the archive
from `ArchiveWork` into `Data`. That is the same call the port's `FileSpace::rename` makes.

The fixture is 40 synthetic precombined `.nif` files and 20 `.uvd` files, named the way the CK names them.
The probe reads every archive back with its own BA2 reader, so no tool gets to judge itself.
For each archive it checks:

- the internal paths, ignoring case and `/` vs `\`
- the bytes of every file
- that each record's lookup key (name hash, extension, directory hash) matches the first archive that held
  the same path

BSArch writes `/` in the name table where Archive2 writes `\`, but its keys are identical. The game finds
files by those keys.

Ticket items in the summary:

- **item1**: BSArch pack → unpack → repack fidelity (0.9x and 1.0) with staging at `<fo4>\ArchiveWork`.
- **item2**: Archive2 writing outside `Data` with relative sources keeps `meshes\precombined\…` / `vis\…`.
- **item3**: delete-then-move into `Data` lands where this process (and, inside MO2, the VFS) sees it.
  Inside MO2 the report also records where the files physically ended up: in `overwrite`, or in a mod
  folder.

## Safety

The probe refuses to start if any of these is true, and it deletes only what it created:

- `Data\meshes\precombined` or `Data\vis` holds any file
- `<fo4>\ArchiveWork` exists
- `Data\GPProbe - Main.ba2` exists

Inside MO2 those checks see the VFS. If a mod in the profile ships loose precombines or previs, switch to
a profile without that mod (an empty profile is fine).

## Run

Build once, from a normal terminal (**not** elevated):

```powershell
cd probes\archive-roundtrip-probe
cargo build --release
```

**Outside MO2:**

```powershell
.\target\release\archive-roundtrip-probe.exe
```

**Inside MO2:** in *Modify Executables*, add `target\release\archive-roundtrip-probe.exe`, with no
arguments and *Start in* set to this folder. Then run it from MO2. The probe detects MO2's `usvfs` DLL
by itself and waits for ENTER before closing its console.

Defaults:

- the Fallout 4 folder comes from the registry
- Archive2 is `<fo4>\Tools\Archive2\Archive2.exe`
- BSArch 1.0 is `D:\programs\xEdit 4.1\BSArch.exe`
- BSArch 0.9x is `D:\programs\xEdit 4.1.5f\BSArch.exe`
- the MO2 instance is `E:\Mod Organizer\Fallout 4`

Override them with `--fo4`, `--archive2`, `--bsarch10`, `--bsarch09` and `--mo2-instance`. Pairs that
need a missing tool are skipped. To run fewer pairs, use `--step3 archive2,bsarch10 --step8 bsarch09`.

Each run writes `reports/<unix>-<inside|outside>-mo2.json`.
