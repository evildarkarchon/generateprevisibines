# GeneratePrevisibines

Rust port of the GeneratePrevisibines V2.96 workflow for automating Fallout 4 precombine and previs generation.

## Project status (reboot)

The repository is in a **scaffold** phase:

- **Behavioral reference:** PJM batch V2.96 (kept locally, not in this repo) until parity is proven on a real install.
- **Implemented today:** CLI/types, validation rules, workflow step planning, logging layout, tool wrapper stubs, unit tests.
- **Not implemented yet:** Creation Kit / FO4Edit / archive execution, interactive prompts, full 8-step automation.

Track Rust work in **[docs/future-features.md](docs/future-features.md)**.

Related docs: [behaviors](docs/behaviors.md) · [workarounds](docs/workarounds.md) · [technical](docs/technical.md)

## Overview

The goal is an 8-step workflow for precombined meshes and previs data using Creation Kit and FO4Edit, preserving required workarounds (MO2 delays, DLL renaming, FO4Edit keystrokes, Archive2 extract-repack).

## Features (scaffold vs planned)

| Area | Scaffold | Planned (see backlog) |
|------|----------|------------------------|
| Build modes (clean / filtered / xbox) | CLI + types | Full CK/archive behavior per mode |
| Archive tool (Archive2 / BSArch) | CLI + types | Real pack/extract/append |
| Tool discovery | Partial (Windows registry + cwd) | Version display, BSArch path rules |
| CKPE / plugin validation | Yes | Same checks before CK runs |
| 8-step workflow | Step planning + `--dry-run` | Execute all steps |
| Interactive / resume | Types + menu labels | dialoguer prompts, resume execution |
| External tool automation | Stubs in `src/tools/` | Parity with batch |

## Requirements

- **Windows** (uses Windows-specific APIs)
- **Fallout 4** installation
- **Creation Kit** (in Fallout 4 directory)
- **Creation Kit Platform Extended (CKPE)** - properly configured
- **FO4Edit** - in current directory or installed
- **Archive2.exe** or **BSArch.exe** - for BA2 archive creation

## Installation

### From Release
Download the latest `generateprevisibines.exe` from the [Releases](../../releases) page.

### Building from Source
```bash
# Clone the repository
git clone https://github.com/evildarkarchon/generateprevisibines.git
cd generateprevisibines

# Build release version
cargo build --release

# Binary will be at: target/release/generateprevisibines.exe
```

## Usage

### Dry run (scaffold)

List planned steps without calling external tools:

```bash
cargo run -- --dry-run --filtered MyMod
```

### Non-interactive (scaffold)

Validates config and executes the currently implemented Workflow Operations:

```bash
cargo run --release -- MyMod.esp
cargo run --release -- -f --bsarch --FO4 "D:\Games\Fallout4" MyMod.esp
```

### Interactive mode

Not implemented in the scaffold. See [docs/future-features.md](docs/future-features.md) § Workflow Implementation.

### Command-Line Options

```
Usage: generateprevisibines.exe [OPTIONS] [PLUGIN]

Arguments:
  [PLUGIN]  Plugin name (e.g., MyMod.esp)

Options:
  -c, --clean       Build mode: clean (default)
  -f, --filtered    Build mode: filtered
  -x, --xbox        Build mode: xbox
      --bsarch      Use BSArch instead of Archive2
      --FO4 <PATH>  Override Fallout 4 directory
      --dry-run       List planned workflow steps and exit
      --resume-from N Resume from workflow step 1-8 (non-interactive)
  -h, --help        Print help
```

### Examples

**Clean mode (default):**
```bash
generateprevisibines.exe MyMod.esp
```

**Filtered mode:**
```bash
generateprevisibines.exe -f MyMod.esp
```

**Xbox mode with BSArch:**
```bash
generateprevisibines.exe -x --bsarch MyMod.esp
```

**Custom Fallout 4 directory:**
```bash
generateprevisibines.exe --FO4 "D:\Games\Fallout4" MyMod.esp
```

## The 8-Step Workflow

1. **Generate Precombines Via CK** - Creates precombined meshes
2. **Merge PrecombineObjects.esp Via xEdit** - Merges generated data into your plugin
3. **Create BA2 Archive from Precombines** - Archives the precombined meshes
4. **Compress PSG Via CK** *(clean mode only)* - Compresses geometry data
5. **Build CDX Via CK** *(clean mode only)* - Builds CDX file
6. **Generate Previs Via CK** - Creates previs data
7. **Merge Previs.esp Via xEdit** - Merges previs data into your plugin
8. **Add Previs to BA2 Archive** - Adds previs files to the archive

## Build Modes

### Clean Mode (`-c` or default)
- Generates full precombine and previs data
- Includes PSG compression and CDX building
- Recommended for final releases

### Filtered Mode (`-f`)
- Generates precombines and previs without extra processing
- Skips PSG compression and CDX building
- Faster workflow for testing

### Xbox Mode (`-x`)
- Same as filtered mode but uses Xbox compression for archives
- Required for Xbox mods

## Archive Tools

### Archive2 (default)
- Bethesda's official tool
- Found in `Fallout 4\Tools\Archive2\Archive2.exe`
- **No append support** - must extract, modify, re-archive

### BSArch (`--bsarch`)
- Community tool with better performance
- Can append to existing archives
- Searched in order:
  1. Current directory
  2. Executable directory
  3. Fallout 4 directory
  4. `Fallout 4\Tools` directory

## CKPE Configuration

The tool validates your CKPE configuration before running. Required settings:

**CreationKitPlatformExtended.ini:**
```ini
[CreationKit]
bBSPointerHandleExtremly = true

[Log]
sOutputFile = CreationKit.log
```

**Legacy fallout4_test.ini:**
```ini
[CreationKit]
BSHandleRefObjectPatch = true

[CreationKit_Log]
OutputFile = CreationKit.log
```

## Important Notes

### Necessary Workarounds
This tool includes several workarounds that are **REQUIRED** and should not be "optimized away":

1. **DLL Renaming** - ENB/ReShade DLLs crash Creation Kit and must be temporarily disabled
2. **FO4Edit Keystroke Automation** - Module Selection dialog requires ENTER keystroke even with `-autoexit`
3. **Archive2 Extract-Repack** - Archive2 has no append functionality
4. **MO2 Timing Delays** - Mod Organizer 2's virtual file system requires sync delays

These are documented in code with explanations.

### Reserved Plugin Names
Do **not** use these in your plugin name:
- `previs`
- `combinedobjects`
- `xprevispatch`

### Clean Mode Restrictions
In clean mode, plugin names **cannot contain spaces**. Use filtered mode if your plugin has spaces.

## Logging

Logs are saved to `%TEMP%\generateprevisibines_YYYYMMDD_HHMMSS.log`

The log file path is displayed at the end of execution.

## Troubleshooting

### "CKPE configuration error: bBSPointerHandleExtremly is not set to true"
Edit your CKPE config file and add:
```ini
bBSPointerHandleExtremly = true
```

### "No precombined meshes were generated"
- Check Creation Kit log for errors
- Ensure your plugin is in the Data directory
- Verify CKPE is working correctly

### "FO4Edit window not found"
- FO4Edit automation requires the window to appear
- Check that FO4Edit is not already running
- Verify FO4Edit.exe path is correct

### "Directory is not empty"
In interactive mode, you'll be prompted to clean directories. In non-interactive mode, clean them manually or run interactively.

## Development

See [docs/future-features.md](docs/future-features.md) for the implementation backlog and recommended vertical slices.

### Running Tests
```bash
cargo test
```

Unit tests cover CLI parsing, plugin/CKPE validation, workflow step planning, and discovery helpers.

### Building
```bash
# Debug build
cargo build

# Release build (optimized, smaller binary)
cargo build --release
```

## Credits

- Original GeneratePrevisibines batch workflow by [PJMail](https://www.nexusmods.com/fallout4/users/28439055)
- Rust reboot scaffold; parity work tracked in [docs/future-features.md](docs/future-features.md)

## License

This project is licensed under the [GNU General Public License v3.0](https://www.gnu.org/licenses/gpl-3.0.en.html)
