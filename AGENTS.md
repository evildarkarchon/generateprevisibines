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

## Agent skills

### Issue tracker

Issues and PRDs are tracked as local markdown files under `.scratch/<feature-slug>/`. See `docs/agents/issue-tracker.md`.

### Triage labels

The canonical triage roles use their default same-name GitHub labels. See `docs/agents/triage-labels.md`.

### Domain docs

This repository uses a single-context domain documentation layout. See `docs/agents/domain.md`.

## graphify

This project has a knowledge graph at graphify-out/ with god nodes, community structure, and cross-file relationships.

When the user types `/graphify`, use the installed graphify skill or instructions before doing anything else.

Rules:
- For codebase questions, first run `graphify query "<question>"` when graphify-out/graph.json exists. Use `graphify path "<A>" "<B>"` for relationships and `graphify explain "<concept>"` for focused concepts. These return a scoped subgraph, usually much smaller than GRAPH_REPORT.md or raw grep output.
- Dirty graphify-out/ files are expected after hooks or incremental updates; dirty graph files are not a reason to skip graphify. Only skip graphify if the task is about stale or incorrect graph output, or the user explicitly says not to use it.
- If graphify-out/wiki/index.md exists, use it for broad navigation instead of raw source browsing.
- Read graphify-out/GRAPH_REPORT.md only for broad architecture review or when query/path/explain do not surface enough context.
- After modifying code, run `graphify update .` to keep the graph current (AST-only, no API cost).
