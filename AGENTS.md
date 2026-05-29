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

### Key Types

- `BuildMode`: `Clean` (full workflow), `Filtered` (skip PSG/CDX), `Xbox` (filtered + Xbox compression)
- `ArchiveTool`: `Archive2` (Bethesda's tool) or `BSArch` (community tool with append support)
- `WorkflowStep`: enum for steps 1-8, supports resume from any step

## Reference Documentation

- [docs/workarounds.md](docs/workarounds.md) - Required workarounds with batch line references
- [docs/behaviors.md](docs/behaviors.md) - Key behaviors and UX expectations to preserve
- [docs/technical.md](docs/technical.md) - Windows APIs, recommended crates, code style, testing
- [docs/future-features.md](docs/future-features.md) - Post-scaffold implementation backlog
