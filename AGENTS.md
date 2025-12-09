## Project Purpose

Translating `GeneratePrevisibines.bat` (500+ lines) into idiomatic Rust. Automates Fallout 4 mod development using Creation Kit and FO4Edit through an 8-step workflow for generating precombined meshes and previs data.

## Build Commands

```bash
cargo build              # Debug build
cargo build --release    # Release build (optimized, uses LTO)
cargo test               # Run all tests
cargo clippy             # Lint with pedantic warnings enabled
```

## Critical Constraints

**The external programs (CreationKit, FO4Edit, Mod Organizer 2) are NOT designed for automation.** The workarounds in the batch script are **necessary**, not code smell.

**DO NOT "FIX" THESE** - See [docs/workarounds.md](docs/workarounds.md) for details:
- PowerShell keystroke automation for FO4Edit (no headless mode)
- MO2 timing delays (5-10s VFS sync required)
- DLL renaming (CK crashes with ENB/ReShade DLLs)
- Archive2 extract-repack (no append functionality)

## Architecture

### Main Components

- **main.rs** - CLI argument parsing (clap), tool discovery, configuration setup, workflow entry point
- **workflow.rs** - `WorkflowExecutor` orchestrates the 8-step process via `WorkflowStep` enum
- **config.rs** - `Config` struct holding paths, build mode (`Clean`/`Filtered`/`Xbox`), archive tool selection

### Tool Wrappers (`src/tools/`)

- **creation_kit.rs** - `CreationKitRunner`: generates precombines, compresses PSG, builds CDX, generates previs
- **fo4edit.rs** - `FO4EditRunner`: merges generated ESPs using Windows SendInput for keystroke automation
- **archive.rs** - `ArchiveManager`: handles BA2 creation via Archive2 or BSArch
- **dll_manager.rs** - Disables/restores ENB/ReShade DLLs that crash Creation Kit

### Support Modules

- **registry.rs** - Windows Registry lookups for tool paths (HKCR, HKLM)
- **ckpe_config.rs** - Parses CKPE `.toml`/`.ini` configs, validates `bBSPointerHandleExtremly=true`
- **validation.rs** - Plugin name validation (reserved names, space restrictions)
- **prompts.rs** - Interactive Y/N prompts via dialoguer
- **filesystem.rs** - Directory creation, file counting, cleanup operations
- **mo2_helper.rs** + **mo2-mode/** subcrate - Mod Organizer 2 VFS integration

### Key Types

- `BuildMode`: `Clean` (full workflow), `Filtered` (skip PSG/CDX), `Xbox` (filtered + Xbox compression)
- `ArchiveTool`: `Archive2` (Bethesda's tool) or `BSArch` (community tool with append support)
- `WorkflowStep`: enum for steps 1-8, supports resume from any step

## Reference Documentation

- [docs/workarounds.md](docs/workarounds.md) - Required workarounds with batch line references
- [docs/behaviors.md](docs/behaviors.md) - Key behaviors and UX expectations to preserve
- [docs/technical.md](docs/technical.md) - Windows APIs, recommended crates, code style, testing
