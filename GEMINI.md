# GeneratePrevisibines - Rust Edition

## Project Overview

This project is a Rust port of the `GeneratePrevisibines.bat` script, designed to automate the complex, 8-step workflow for generating precombined meshes and previs data for Fallout 4 mods. It orchestrates interactions between three external tools: **Creation Kit**, **FO4Edit**, and an archiver (**Archive2** or **BSArch**).

The primary goal is to provide a robust, crash-resilient, and resumable automation tool that handles the specific quirks and instability of the Bethesda modding toolchain.

### Key Features
*   **8-Step Workflow:** Automates the entire process from generating precombines to archiving previs data.
*   **Resumability:** Users can restart the workflow from any of the 8 steps if a tool crashes.
*   **Modes:** Supports 'Clean' (production), 'Filtered' (testing), and 'Xbox' build modes.
*   **Tool Management:** Automatically discovers tools via Windows Registry and handles necessary workarounds (e.g., disabling ENB DLLs).
*   **MO2 Support:** Dedicated support for running within Mod Organizer 2's virtual file system.

## Architecture

### Core Modules (`src/`)
*   **`main.rs`**: CLI entry point. Handles argument parsing, tool discovery, validation, and initialization.
*   **`workflow.rs`**: The heart of the automation. Defines the `WorkflowStep` enum and `WorkflowExecutor` struct which runs the 8-step process.
*   **`config.rs`**: Manages configuration state (paths, build modes, plugin names).
*   **`mo2_helper.rs`**: specialized logic for handling Mod Organizer 2 paths and environment variables.
*   **`registry.rs`**: Windows Registry access for finding Fallout 4, Creation Kit, and other tools.
*   **`validation.rs`**: Logic for validating plugin names and file existence.

### Tool Wrappers (`src/tools/`)
*   **`creation_kit.rs`**: Manages the Creation Kit process.
*   **`fo4edit.rs`**: Manages FO4Edit. Includes **critical automation logic** (using `SendInput` to simulate keystrokes) because FO4Edit lacks a true headless mode for some operations.
*   **`archive.rs`**: Abstracts the difference between `Archive2.exe` and `BSArch.exe`. Handles the "extract-add-repack" dance required for `Archive2`.
*   **`dll_manager.rs`**: handles the temporary renaming of ENB/ReShade DLLs (`d3d11.dll`, etc.) which are known to crash the Creation Kit.

## Development Guidelines

### ⚠️ Critical Constraints & Workarounds
**Do not refactor these "inefficiencies" without understanding why they exist.** They are strictly required by the external tools.

1.  **Keystroke Automation:** FO4Edit requires simulated `ENTER` keys to progress through dialogs. This is implemented using Windows APIs in `fo4edit.rs`.
2.  **Delays:** `std::thread::sleep` is used intentionally to allow Mod Organizer 2's virtual file system to synchronize.
3.  **DLL Renaming:** The tool **must** rename `d3d11.dll` and friends before launching Creation Kit and restore them afterwards.
4.  **Archive Repacking:** `Archive2.exe` cannot append to archives. The tool must extract the entire archive, add the new file, and repack it.

### Building and Running

**Prerequisites:**
*   Rust (latest stable)
*   Windows OS (strictly required due to `windows` crate and Registry dependencies)

**Commands:**
```bash
# Build for development
cargo build

# Build for release (minimized binary)
cargo build --release

# Run the tool (interactive mode)
cargo run

# Run the tool (non-interactive, specific plugin)
cargo run -- MyMod.esp

# Run tests (unit tests only)
cargo test
```

### Testing
*   **Unit Tests:** Cover path manipulation, validation logic, and configuration parsing.
*   **Integration Tests:** Difficult to automate because they require a valid Fallout 4 installation and the actual external tools. Manual testing is often required for workflow changes.

## Directory Structure
```
src/
├── tools/              # Wrappers for external binaries
│   ├── archive.rs      # Archive2/BSArch abstraction
│   ├── creation_kit.rs # CK runner
│   ├── dll_manager.rs  # ENB DLL handling
│   └── fo4edit.rs      # FO4Edit runner + input automation
├── config.rs           # Configuration structs
├── main.rs             # Entry point & CLI args
├── registry.rs         # Windows Registry lookups
└── workflow.rs         # The 8-step state machine
```
