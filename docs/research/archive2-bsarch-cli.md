# Archive2 and BSArch command-line capabilities

Research for issue #26 (wayfinder map #23). This file records facts only. The design choice belongs to the
Archive design ticket (#29).

Researched 2026-10-02. Sources are tagged `[S1]`, `[S2]` and so on and listed at the end. A claim tagged
**(observed)** was run on this machine (Windows 11). Those runs were read-only: help text, a missing
archive, a missing source folder. No archive was created.

## Summary (most decision-relevant first)

1. **BSArch cannot make Xbox-compressed FO4 archives, in any version.** FO4 General BA2 supports only zlib
   in BSArch [S3 L177-187]. The compression enum is `None/ZLib/LZ4/LZ4F` [S5 L21]. `-z:xbox` is
   rejected with exit 1 (observed). Archive2 is the only one of the two that exposes `-compression=XBox`
   [S8].
2. **The reference batch V2.99 never asks for Xbox compression on either tool.** The line that would set it
   is commented out: `REM If /I "%BuildMode_%" EQU "xbox" SET Arch2Quals_=-compression=XBox`
   (`GeneratePrevisibines.bat` L391). `docs/episodes.md` § Archive describes V2.98, where the line was
   live and only Archive2 received it. In V2.99, `-xbox` changes nothing on either archive path.
3. **Neither tool can update an archive in place.** Archive2's help lists no append, add or update verb
   [S8]. BSArch only ever creates a new archive [S1 L129-273, S6].
4. **BSArch v1.0 (shipped in xEdit 4.1.5q) can pack several sources at once, including an existing
   archive.** The syntax is `pack <src1+src2+...> <archive>`, and sources can be folders, files or
   archives. "Files from later source win on matched file names" [S1 L198-202, L382-387]. So
   `pack "<old>.ba2"+"<Data>\vis" "<new>.ba2"` rebuilds the archive with the new files added, without
   extracting to disk. The output must be a new file. The source archive is opened `fmShareDenyWrite`
   and the output with `fmCreate` [S3 L1319, L1786], so writing over the input should fail. That is
   inferred from the source and has not been run.
5. **BSArch 0.9x (every xEdit release up to 4.1.5p) takes exactly one source folder.** That is why the
   batch needs its `BSArchTemp` staging dir [S6, S2].
6. **Paths inside the archive are worked out differently by each tool.** BSArch 0.9x uses the path
   relative to the source folder [S6 L1753-1776]. BSArch 1.0 looks for the first `\data\` or
   `\data files\` component, then the first known asset root (`meshes`, ..., `vis`), then the source
   folder [S3 L698-760]. Archive2 "looks for a data folder" unless `-root=` is given [S8]. The batch's
   `Meshes\vis\*.uvd` fallback bug happens in **both** BSArch versions (see the rooting section below).
7. **Exit codes.** BSArch exits 1 on unhandled exceptions in every version. v1.0 between 2026-01-12 and
   2026-01-25 exited 0 on per-file pack and unpack errors. Commit `c930de0003` fixed that before the
   4.1.5q release [S4]. Archive2 returns -1 on bad arguments and on a missing input archive (observed).
   But `Archive2 -c=x.ba2 -q` with no sources returns **0 and creates nothing** (observed), so checking
   that the archive exists afterwards is required.
8. **Archive2 can read a list of sources from a file** (`-sourceFile|s=`), and the NG build can write one
   from an archive (`-createSourceFile|csf=`). It also has regex include and exclude filters [S8]. The
   source-file format is not documented in any primary source.
9. **Wine/Proton: no authoritative record for either tool.** BSArch is a native Delphi console exe with no
   .NET dependency [S1 L21-37]. Archive2 is a .NET Framework 4 WPF exe that loads a C++/CLI mixed-mode
   interop DLL (observed), which is the hard case for Wine Mono [S11, S12].

## Versions in play

| Tool | Version | Where it comes from | Evidence |
|---|---|---|---|
| BSArch | 0.9c / 0.9d / 0.9e | xEdit 4.1.5f (2024-04) through 4.1.5p (2025-10). The CLI is identical across them | `csBSAVersion` at each tag. `BSArch.dpr` is byte-identical between 4.1.5f and 4.1.5p [S6] |
| BSArch | **1.0** | xEdit **4.1.5q** (tag 2026-06-15). This is the release named in the batch V2.99 header (L15) | `cBSArchVersion = '1.0'` [S3 L24]. CLI rewritten in "BSArch/Sniff - Many Updates" (2026-01-12) [S4] |
| BSArch | 1.1 | `dev-4.1.6` branch, not released. The CLI is the same as 1.0 (`BSArch.dpr` identical) | [S1] |
| Archive2 | 1.1.0.4 | Old-gen (pre-NG) Creation Kit | observed (`Tools.og`) |
| Archive2 | **1.1.0.5** | NG Creation Kit (`Fallout 4\Tools\Archive2`) | observed [S8] |

The batch finds BSArch next to the xEdit it runs from: `SET BSArchexe_=%~dp1BSArch.exe` (L522). So which
BSArch CLI applies depends on the user's xEdit version, and either generation is plausible.

## Archive2

### Full flag set (captured from `Archive2.exe -?`, v1.1.0.5) [S8]

```
Archive2 <archive, files/folders> [<arguments>]
  archive        Specifies the archive to open (for extracting/opening an archive).
  files/folders  Specifies the files and folders to open (for making an archive, comma-delimited).
   -sourceFile|s=<string>        Sets the source file to find contents to archive
   -excludeFile=<string>         Sets the file that lists files not to archive
   -create|c=<string>            Tells archiver to create an archive with the specified name
   -extract|e=<string>           Tells the archiver to extract an archive to the specified folder
   -createSourceFile|csf=<string> Tells the archiver to create a source file from an archive to the specified folder
   -root|r=<string>              Tells the archiver what path to use for the archive root (instead of
                                 looking for a data folder)
   -format|f=<General|DDS|XBoxDDS>              (default - General)
   -compression=<None|Default|XBox>             (default - Default)
   -count=<unsigned int>                        Sets the archive count to make (default - 0)
   -maxSizeMB|sMB=<unsigned int>                (default - 0)
   -maxChunkCount|mch=<unsigned int>            (default - 4)
   -singleMipChunkX|mipX=, -singleMipChunkY|mipY=  (default - 512)
   -nostrings                    Does not write a string table to the archive
   -quiet|q                      Does not report progress or success (only failures)
   -tempFiles                    Use temporary files instead of loading chunks into memory
   -cleanup                      Cleans up chunk temp folder on launch (do not use when multiple copies are running)
   -includeFilters=??            A list of regular expressions for file inclusion.
   -excludeFilters=??            A list of regular expressions for file exclusion.
   -?                            Prints usage information.
```

Differences between builds (observed, diffing the `-?` output):

- 1.1.0.4 (old-gen) has no `-createSourceFile`, and its `-format` also accepts `GNF`.
- Starfield's Archive2 1.2.0.1 adds `-outputOrderingFile`, `-chunkAlign` and `-compression=LZ4`. It is not
  relevant to FO4 and is listed only to show that the help text varies by build.

The flags the batch uses:

- `-c=` (L405), `-e=` (L416), `-f=General` (L405) and `-q` (L405, L416).
- `-compression=XBox` was used in V2.98 only (V2.99 L391 is REM'd).
- Multi-folder sources are comma-delimited, as in `meshes\precombined,vis` (L442 → L405). That matches
  the help text: "files/folders ... comma-delimited" [S8]. Not established: whether a path that
  contains a comma can be passed at all.

### Source-list file

`-sourceFile|s=` and `-excludeFile=` exist [S8]. Archive2.exe contains the strings
`Add the contents of a source file to the archive`, `Invalid SourceFile:` and
`Can only create the source file for one archive at a time.` (observed, string scan). The file format is
not documented in Archive2's help.

A community wiki says the CK's `.achlist` files are JSON arrays of `"Data\\..."` paths [S10]. Nothing
primary links `.achlist` to `-sourceFile`. See Open facts.

### Append / update

The help text has no append, add or update mode [S8]. The batch says the same: "Unfortunately Archive2 does
not support this so must extract and re-archive" (L422). The GUI's Archive menu has "Add Files/Add Folder"
followed by Save [S9], but that is a GUI workflow and has no CLI equivalent in the help.

### Exit codes (observed, v1.1.0.5)

| Invocation | Exit | Output |
|---|---|---|
| `-?` or `/?` | 0 | usage on stdout |
| `-help` (unknown arg) | **-1** | usage on stdout, `Unknown command line argument: help` on stderr |
| `"<missing>.ba2" -e=. -q` | **-1** | `System.IO.FileNotFoundException ... Could not open "..." for reading` plus a .NET stack trace on **stderr** (shown even with `-q`) |
| `-c="<tmp>\x.ba2" -q` with **no sources** | **0** | nothing. **No archive was created** |

Two consequences:

- Errors go to stderr, but the batch only redirects stdout to its log (`>> "%Logfile_%"`, L405/L416).
- A zero exit does not prove an archive was written. The batch's `If not Exist ... "No plugin archive
  Created"` check (L408) is what catches this.

Not tested, because it would have meant creating archives: a missing source folder with `-c`, an
unwritable output path, and a mid-pack I/O failure.

### Root / archive-internal paths

The `-root` help line implies Archive2 normally "look[s] for a data folder" in each source path and makes
the paths below it the internal paths [S8]. The batch relies on this. It runs with `/D"<fo4>\Data"` and
passes relative sources (`meshes\precombined`, `vis`) (L405), which resolve to `<fo4>\Data\...`.

Not established from any primary source:

- which `data` component wins (first or last) when an ancestor directory is also called `Data`
- what happens when no `data` component exists and `-root` is not given

### Xbox

`-compression=XBox` and `-format=XBoxDDS` exist [S8]. What `XBox` compression actually does to a General
archive is not documented. A community wiki says Xbox texture archives are best packed through the CK
"for proper Xbox compression" [S10]. That source is secondary and says nothing about General (mesh/vis)
archives.

### BA2 version written

BSArch writes FO4 BA2 header version 1 [S3 L549, L1598]. A community article says the NG Archive2 (1.1.0.5)
"may save archives as Version 7 or 8", which old-gen game builds cannot load [S13]. That is a secondary
source and was not checked here (see Open facts).

## BSArch

### Verb and flag set, v1.0 (xEdit 4.1.5q) [S1 L355-451, L129-352]

- **`pack <source1+source2+...> <archive> [params]`**
  - Each source is a folder, a single file or an existing archive. `+` separates sources, with no spaces
    allowed. Later sources override earlier ones on matching asset names [S1 L198-202,
    S3 L2837-2883].
  - Game type (one is required): `-tes3 -tes4 -fo3 -fnv -tes5 -sse -fo4 -fo4dds -sf1 -sf1dds`.
  - `-z[:type]` compresses. Allowed types are `zlib`, `lz4` and `lz4f`. With no type, the game's default
    is used. For FO4 General that is zlib and nothing else [S3 L177-187]. Strings and audio (except
    `.fuz`) are never compressed.
  - `-split:N` sets the size in GB, up to a maximum of 8. BA2 archives are not split by default.
  - `-f:mask,mask` filters by file-name mask.
  - `-share:yes|no` defaults to yes. `-mt:yes|no` defaults to yes. A bare `-mt` means yes.
  - `-af:` and `-ff:` set hex flags (TES4 through Skyrim only).
- **`unpack <archive> [folder] [-mt:yes|no]`**: the folder must already exist. It defaults to the
  archive's own folder.
- **`<archive> [-list|-dump]`**: shows archive info.
- Parsing details:
  - The verb is matched case-insensitively (`SameText`), so the batch's `Pack` works.
  - Switch values use `:` [S7 L38-65]. The switch characters are Delphi's `SwitchChars` (`-` and `/` on
    Windows).
  - Sources are split on `+` (`ParamStr(2).Split(['+'], '"')`, S1 L198), so a source path containing a
    literal `+` is split in two.

### Verb and flag set, 0.9c–0.9e (xEdit ≤ 4.1.5p) [S6]

- **`pack <folder> <archive> [params]`** takes exactly one folder. Files directly in that folder are
  skipped. So are files with no extension, `.exe`, `.bsa`, `.ba2` and `.db`.
  - Switches: game type, plus `-z` (on/off only, which means zlib for FO4), `-share` (**off** unless
    given), `-mt` (**off** unless given), `-af:` and `-ff:`.
  - A relative `<archive>` path is resolved against `<folder>`.
- **`unpack <archive> [folder] [-q|-quiet] [-mt]`**
- **`<archive> [-list|-dump]`**

The batch's `Pack "<fo4>\BSArchTemp" "<Data>\<name> - Main.ba2" -mt -fo4 -z` (L397, L430) is valid in
both generations.

### Xbox compression

There is none.

- FO4 General archives accept only `ctZLib` [S3 L177-187]. The compression enum has no Xbox or XMem
  member [S5 L21].
- 0.9x has only an on/off `-z`, which also means zlib [S6].
- Observed: `BSArch pack ... -fo4 -z:xbox` exits 1 with `EInvalidArguments: Fallout 4 archives don't
  support xbox compression`. (`TypeByName` reads its loop variable after the loop finishes, so unknown
  names produce this message rather than "Unknown compression type". Either way the exit code is 1.)
- BSArch's only Xbox handling is for DDS textures (`TwbBSArchiveTarget = (btPC, btXBox)`, the
  `_xbox.` DDS headers) [S3 L30].

### Append / multiple roots

- **0.9x:** no append and one root only. The `BSArchTemp` staging tree is the only way to combine
  `Meshes\Precombined` and `vis` [S6].
- **1.0:** no in-place append. The multi-source `+` syntax covers both of the batch's needs:
  - Packing several roots without a staging dir: `"<Data>\meshes\precombined"+"<Data>\vis"`.
  - Rebuilding from an existing BA2 plus new files: `"<Data>\X - Main.ba2"+"<Data>\vis"`. This one is
    documented in BSArch's own example "Merge archives and overwrite with files from folder" [S1 L439-440].
  - The output must not be one of the input archives. That follows from the share modes [S3 L1319,
    L1786] but has not been run.

### Exit codes

- **All versions:** the top-level `except` writes `<Class>: <message>` to **stdout** and sets
  `ExitCode := 1` [S1 L619-633, S6]. Errors are not written to stderr.
- **1.0:** per-file errors in the parallel pack and unpack loops are collected into `ProcessedError`
  rather than raised. Commit `c930de0003` (2026-01-25, "non-zero Exit code on error") added
  `ExitCode := 1` on that path [S4]. So BSArch 1.0 builds made from the 2026-01-12 rewrite before that
  commit exit **0** on per-file failures. The 4.1.5q release includes the fix.
- **Info mode** (`BSArch <archive>`) catches load errors, prints `Error: ...` and exits 0 [S1 L95-126].
  For a *missing* file it raises an argument error instead, and exits 1 (observed).
- Observed (v1.0 and v0.9c), each exiting 1:
  - a missing source folder: v1.0 prints `No valid source file(s) found.`, v0.9c prints
    `EDirectoryNotFoundException`
  - a missing archive on `unpack`
  - `-z:xbox`

## How each tool roots archive-internal paths

**BSArch 0.9x**: the internal path is the file path relative to the single `<folder>` argument
(`Copy(aFileName, Length(root)+1, ...)`) [S6 L1753-1776, DoPack `sl.Add(Copy(s, Succ(Length(root)), ...))`].

**BSArch 1.0**: `TwbAsset.GetAssetName(file, sourceFolder)` [S3 L698-760] tries these rules in order:

1. If the lower-cased full path contains `\data\` or `\data files\`, it takes everything after the **first**
   occurrence.
2. Otherwise it checks the known asset roots in table order: `meshes`, `textures`, `materials`, ...,
   `vis`, .... For the first root found anywhere in the path, it takes everything from that root onwards.
3. Otherwise it takes the path relative to the source folder.
4. With no source folder (a single-file source), it falls back to a root guessed from the extension.

Consequences:

- Rule 1 scans the **absolute** path. If an ancestor of the FO4 install is called `Data`, files under
  `<fo4>\BSArchTemp\...` would be rooted after *that* `Data` and get the wrong internal paths.
  Unusual ancestor names (`meshes`, `sound`, `video`, ...) can trigger the same problem through rule 2.
- `<fo4>\Data\meshes\precombined\x.nif` → `meshes\precombined\x.nif`, and `<fo4>\Data\vis\x.uvd` →
  `vis\x.uvd`. Both come from rule 1, so passing the `Data` subfolders directly as sources roots them
  correctly.
- `.cdx` and `.csg` (along with `.esp`, `.ba2` and others) are always skipped as sources
  (`cSkippedExtensions`) [S3 L498].

**The `Meshes\vis\*.uvd` fallback bug** (`docs/episodes.md` Step 8): when step 8 finds no `- Main.ba2`, it
calls `:Archive vis` (L445-446). On the BSArch path that moves `Data\vis` into `<fo4>\BSArchTemp\Meshes`
(L395-396), giving `<fo4>\BSArchTemp\Meshes\vis\*.uvd`. Each version roots that as follows:

- **0.9x:** relative to `BSArchTemp`, which gives `Meshes\vis\*.uvd`. The bug occurs.
- **1.0:** rule 2 finds `meshes` before `vis` in table order, which gives `Meshes\vis\*.uvd`. The bug
  occurs. (Rule 1 does not apply, because `BSArchTemp` sits beside `Data`, not inside it.)

So the bug comes from where the batch stages the files, not from either BSArch version.

**Archive2**: by default it roots paths at a `data` folder found in the source path, or at `-root=<path>`
if given [S8]. The batch's relative sources, run with cwd `Data`, give `meshes\precombined\...` and
`vis\...`.

## Wine / Proton

No WineHQ AppDB or Bugzilla entry was found for Archive2 or BSArch. The AppDB pages for Fallout 4 and the
Bethesda.net Launcher sit behind a bot check and could not be read [S14]. Proton issue #308 covers the
game, not the tools [S15]. What is established:

- **BSArch** is a native Delphi console program (`{$APPTYPE CONSOLE}`) built for Win32 and Win64. It uses
  only the Delphi RTL plus xEdit's own units, with no .NET [S1 L19-37]. Under Wine:
  - Paths must be passed Windows-style (`Z:\...`). A Unix absolute path begins with `/`, which is a
    `SwitchChar`, so DoPack would reject it as a missing source [S1 L137-139, S7].
  - The multithreaded paths use `TParallel.For` [S1 L215-239]. No Wine-specific report about them was
    found.
- **Archive2** is a .NET Framework 4.0 WPF application. The assembly references `PresentationFramework`,
  `PresentationCore`, `System.Xaml` and `.NETFramework,Version=v4.0`, and the CLI runs inside
  `App.AppStartup` → `CommandLineArchiver.Run` (observed: string scan and the stack trace above).
  - Its archive engine `Archive2Interop.dll` is a **C++/CLI mixed-mode** assembly. It contains the CRT
    mixed-mode strings "The C++ module failed to load during appdomain initialization" (observed).
  - Mono documents mixed-mode assemblies as supported only on Windows, or under Wine through the
    Windows build of Mono [S12]. Wine Mono is that Windows build, positioned as a replacement for .NET
    Framework ≤ 4.8.1 [S11].
  - Whether Archive2 works under the default Wine Mono, or needs `winetricks dotnet48`, is not
    established.
- The batch V2.99 header says "Support for Wine" (L15). In the batch body, the only Wine-specific
  handling found is `WHERE /Q reg.exe` (L21-22), which skips the registry lookups when `reg.exe` is
  missing (L38, L49). Nothing in the archive sections is Wine-specific.
- A community thread reports the Creation Kit running under both Wine and Proton, with crashes in some
  editor operations [S16]. It says nothing about Archive2 or BSArch.

## Open facts (each needs a real-machine experiment)

1. **Does Archive2 `-compression=XBox` on a `-f=General` archive change the output bytes?** Pack the same
   `meshes\precombined` + `vis` with `Default` and with `XBox`, then compare the BA2 headers and the
   per-file packed sizes. (This decides whether the V2.99 REM loses anything.)
2. **Which BA2 header version does Archive2 1.1.0.5 write for a General archive?** Pack a small folder and
   read bytes 4–7 (1 = OG, 7/8 = NG). Check whether an OG `Fallout4.exe` loads it.
3. **BSArch 1.0 `pack old.ba2+Data\vis old.ba2`:** confirm it fails cleanly with exit 1 and leaves
   `old.ba2` intact, as the share modes predict. Then confirm `old.ba2+vis → new.ba2` keeps every
   precombined `.nif` byte-identical.
4. **Archive2 with a missing or empty source folder and `-c=`:** does it exit 0 and create nothing, as it
   did with no sources? Also the exit codes on an unwritable output path and on a locked existing
   archive.
5. **Archive2 `-sourceFile` format:** try a plain one-path-per-line file and a CK `.achlist` JSON. Use
   `-createSourceFile` on an existing BA2 to see what format Archive2 writes itself.
6. **Archive2 root detection:**
   - with an ancestor folder named `Data` (for example `D:\Data\Games\Fallout 4\Data\...`)
   - with no `data` component at all
   - whether `-root=` fixes both
7. **BSArch 1.0 rooting with a `Data`-named ancestor of the install:** confirm the predicted wrong internal
   paths for `<fo4>\BSArchTemp\...`, and that passing `<fo4>\Data\...` sources directly avoids them.
8. **Archive2 under Wine and Proton:**
   - with default Wine Mono, against `winetricks dotnet48`
   - the exit-code propagation of `-1` through Wine's `start /wait` and `cmd`
9. **BSArch under Wine and Proton:**
   - a pack and unpack round trip with `-mt` on and off
   - that the exit code reaches the caller
10. **Archive2 paths containing a comma** in the multi-folder argument, for example a mod or install path
    with `,`.

## Sources

- **[S1]** TES5Edit `BSArch.dpr`, tag `xedit-4.1.5q` (identical on `dev-4.1.6`):
  https://github.com/TES5Edit/TES5Edit/blob/xedit-4.1.5q/BSArch.dpr
- **[S2]** `GeneratePrevisibines.bat` V2.99 in this repo (line numbers above refer to it; `docs/episodes.md`
  cites V2.98 numbering, offset by roughly 9 lines in the archive section).
- **[S3]** TES5Edit `Core/wbBSArchive.pas`, tag `xedit-4.1.5q`:
  https://github.com/TES5Edit/TES5Edit/blob/xedit-4.1.5q/Core/wbBSArchive.pas
- **[S4]** TES5Edit commit history for `BSArch.dpr`, including `2d48403c3c` "BSArch/Sniff - Many Updates"
  (2026-01-12) and `c930de0003` "BSArch - non-zero Exit code on error." (2026-01-25):
  https://github.com/TES5Edit/TES5Edit/commits/dev-4.1.6/BSArch.dpr ·
  https://github.com/TES5Edit/TES5Edit/commit/c930de0003
- **[S5]** TES5Edit `Core/wbCompression.pas`, `dev-4.1.6`:
  https://github.com/TES5Edit/TES5Edit/blob/dev-4.1.6/Core/wbCompression.pas
- **[S6]** TES5Edit `BSArch.dpr` and `Core/wbBSArchive.pas` at tags `xedit-4.1.5f` and `xedit-4.1.5p` (BSArch
  0.9c/0.9e): https://github.com/TES5Edit/TES5Edit/blob/xedit-4.1.5p/BSArch.dpr ·
  https://github.com/TES5Edit/TES5Edit/blob/xedit-4.1.5p/Core/wbBSArchive.pas
- **[S7]** TES5Edit `Core/wbCommandLine.pas` (`wbFindCmdLineParam`):
  https://github.com/TES5Edit/TES5Edit/blob/dev-4.1.6/Core/wbCommandLine.pas
- **[S8]** `Archive2.exe -?` output, Archive2 1.1.0.5 (`Fallout 4\Tools\Archive2`), captured 2026-10-02;
  compared against 1.1.0.4 and Starfield 1.2.0.1. Primary: Bethesda's own built-in help.
- **[S9]** Step Modifications, Guide:Archive2 (GUI walkthrough): https://stepmodifications.org/wiki/Guide:Archive2
- **[S10]** The Fallout Wiki (CK resource mirror): Archive File / `.achlist`
  https://fallout.wiki/wiki/Resource:Creation_Kit/Archive_File · Creative Family Wiki/Archives
  https://fallout.wiki/wiki/Resource:Creative_Family_Wiki/Archives (secondary; the UESP CK wiki
  https://falloutck.uesp.net/wiki/Archive2 was unreachable behind Cloudflare)
- **[S11]** Wine Mono README: https://github.com/wine-mono/wine-mono/blob/main/README.md
- **[S12]** Mono project, C++ / mixed-mode assemblies:
  https://www.mono-project.com/docs/about-mono/languages/cplusplus/
- **[S13]** Nexus Mods article 5844 "BA2 and Archive2" (secondary, NG BA2 v7/v8):
  https://www.nexusmods.com/fallout4/articles/5844
- **[S14]** WineHQ AppDB, Fallout 4: https://appdb.winehq.org/objectManager.php?iId=17171&sClass=application
  (bot-gated; not read)
- **[S15]** ValveSoftware/Proton issue #308 (Fallout 4): https://github.com/ValveSoftware/Proton/issues/308
- **[S16]** Steam Community, "Creation Kit under Linux?":
  https://steamcommunity.com/app/377160/discussions/0/1639787494953794744/
