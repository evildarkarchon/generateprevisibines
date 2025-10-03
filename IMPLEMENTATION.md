# GeneratePrevisibines - Implementation Plan

## Architecture Overview

### Module Structure

```
src/
├── main.rs                 # CLI entry point, argument parsing
├── config.rs              # Configuration and paths
├── registry.rs            # Windows Registry operations
├── workflow.rs            # 8-step workflow state machine
├── validation.rs          # Directory/file state validation
├── tools/
│   ├── mod.rs
│   ├── creation_kit.rs    # CreationKit wrapper
│   ├── fo4edit.rs         # FO4Edit wrapper with keystroke automation
│   ├── archive.rs         # Archive2/BSArch abstraction
│   └── dll_manager.rs     # ENB/ReShade DLL disable/restore
├── prompts.rs             # Interactive user prompts
└── utils.rs               # File operations, version info, logging helpers
```

### Data Flow

```
CLI Args → Config → Workflow State Machine → Tool Wrappers → External Processes
                         ↓
                    Validation Checks
                         ↓
                    User Prompts (if interactive)
                         ↓
                    Logging
```

## Implementation Phases

### Phase 1: Foundation (Core Infrastructure)

**Goal**: Basic CLI that can find tools and parse arguments

#### Tasks
- [ ] Set up `clap` for CLI argument parsing
  - Build modes: `-clean`, `-filtered`, `-xbox`
  - Archive mode: `-bsarch`
  - FO4 directory: `-FO4:<path>`
  - Plugin name parameter
- [ ] Registry module for finding tool paths
  - FO4Edit location (check current dir first, then registry HKCR\FO4Script\DefaultIcon)
  - Fallout4 location (registry HKLM\SOFTWARE\Wow6432Node\Bethesda Softworks\Fallout4)
- [ ] Configuration struct to hold all paths and settings
- [ ] Logging setup (file logging to %TEMP%)
- [ ] Version info extraction using Windows API

#### Deliverable
CLI that can locate all required tools and display their versions, matching batch script output lines 63-75.

---

### Phase 2: File System Operations

**Goal**: Validate directory states and handle file operations

#### Tasks
- [ ] Directory validation functions
  - Check if `meshes\precombined\*.nif` exists
  - Check if `vis\*.uvd` exists
  - Generic "is directory empty" checker
- [ ] Plugin name validation
  - No spaces (clean mode only)
  - No reserved names (previs, combinedobjects, xprevispatch)
  - Extension handling (.esp/.esm/.esl)
- [ ] File operations
  - Safe copy with MO2 delay
  - Safe delete
  - DLL renaming (d3d11.dll ↔ d3d11.dll-PJMdisabled)
- [ ] Archive handling detection (which archive exists)

#### Deliverable
Validation module that can check pre-conditions for each workflow step.

---

### Phase 3: Interactive Prompts

**Goal**: Replicate batch script's interactive UX

#### Tasks
- [ ] Plugin name prompt with validation loop
- [ ] "Use existing plugin?" Y/N/Continue prompt (lines 182-184)
- [ ] "Restart at step" menu (1-8 or 0 to exit) (lines 186-207)
- [ ] "Clean directory?" confirmations (lines 211-213, 217-220)
- [ ] "Rename xPrevisPatch?" confirmation (line 165-166)
- [ ] "Remove working files?" confirmation (line 323)
- [ ] Non-interactive mode (skip all prompts when plugin name passed as arg)

#### Deliverable
Interactive mode matching original UX exactly.

---

### Phase 4: Tool Wrappers

**Goal**: Abstractions for external tool execution

#### Tasks

##### DLL Manager
- [ ] Scan for ENB/ReShade DLLs (d3d11, d3d10, d3d9, dxgi, enbimgui, d3dcompiler_46e)
- [ ] Disable (rename to .dll-PJMdisabled)
- [ ] Restore (rename back)
- [ ] RAII guard pattern to ensure restoration on drop

##### CreationKit Wrapper
- [ ] Execute with `-GeneratePrecombined`, `-CompressPSG`, `-BuildCDX`, `-GeneratePreVisData`
- [ ] Log file deletion before run
- [ ] Process timeout handling
- [ ] Log file parsing for errors
  - "OUT OF HANDLE ARRAY ENTRIES"
  - "visibility task did not complete"
- [ ] Exit code handling (may exit non-zero but still succeed)

##### FO4Edit Wrapper
- [ ] Create Plugins.txt file in %TEMP%
- [ ] Launch with `-fo4 -autoexit -P:<plugins> -Script:<script> -Mod:<plugin> -log:<logfile>`
- [ ] **Keystroke automation**:
  - Wait for Module Selection window
  - Activate window
  - Send ENTER key via SendInput
- [ ] Wait for log file creation
- [ ] Force close main window (CloseMainWindow)
- [ ] Taskkill as fallback
- [ ] Parse log for "Completed:" and "Error:"
- [ ] Script version validation (check for version string in .pas file)

##### Archive Wrapper
- [ ] Archive2 mode:
  - Create archive: `archive2 <path> -c=<archive> [-compression=XBox] -f=General -q`
  - Extract: `archive2 <archive> -e=. -q`
  - No append support (extract-modify-repack pattern)
- [ ] BSArch mode:
  - Pack: `BSArch Pack <source> <archive> -mt -fo4 -z`
  - Can append to existing archives
  - Uses BSArchTemp directory for staging

#### Deliverable
Tested wrappers for each external tool that handle all their quirks.

---

### Phase 5: Workflow Engine

**Goal**: 8-step state machine with resume capability

#### Workflow Steps

1. **Generate Precombines Via CK**
   - Pre-check: meshes\precombined and vis must be empty
   - Run: `CreationKit -GeneratePrecombined:<plugin> "clean/filtered all"`
   - Post-check: .nif files created, .psg file created (clean mode)
   - Error detection: Handle limit check

2. **Merge PrecombineObjects.esp Via xEdit**
   - Pre-check: Precombined meshes exist
   - Run: FO4Edit script `Batch_FO4MergeCombinedObjectsAndCheck.pas`
   - Post-check: Look for "Error:" in log

3. **Create BA2 Archive from Precombines**
   - Archive meshes\precombined
   - Delete source files (Archive2 mode only)

4. **Compress PSG Via CK** (clean mode only)
   - Pre-check: .psg file exists
   - Run: `CreationKit -CompressPSG:<plugin> - Geometry.csg ""`
   - Delete .psg file

5. **Build CDX Via CK** (clean mode only)
   - Run: `CreationKit -BuildCDX:<plugin>.cdx ""`

6. **Generate Previs Via CK**
   - Pre-check: vis directory empty
   - Run: `CreationKit -GeneratePreVisData:<plugin> "clean all"`
   - Error detection: "visibility task did not complete"

7. **Merge Previs.esp Via xEdit**
   - Pre-check: .uvd files exist, Previs.esp exists
   - Run: FO4Edit script `Batch_FO4MergePrevisandCleanRefr.pas`
   - Post-check: "Completed: No Errors."

8. **Add Previs files to BA2 Archive**
   - Extract existing archive (if Archive2)
   - Add vis directory
   - Re-archive
   - Delete source files (Archive2 mode only)

#### Tasks
- [ ] Workflow state enum (8 steps + completion)
- [ ] Step executor functions
- [ ] Resume logic (start at step N)
- [ ] Error handling and rollback (cleanup on failure)
- [ ] Progress logging with timestamps
- [ ] Final summary output (lines 309-321)

#### Deliverable
Complete workflow engine that can run all 8 steps or resume from any step.

---

### Phase 6: CKPE Configuration Validation

**Goal**: Check CKPE settings before running workflow

#### Tasks
- [ ] Find CKPE config file
  - Try CreationKitPlatformExtended.toml
  - Try CreationKitPlatformExtended.ini
  - Try fallout4_test.ini (old CKPE)
- [ ] Parse config (TOML or INI format)
- [ ] Check logging setting
  - `sOutputFile` (TOML/new INI) or `OutputFile` (old)
  - Must not be empty or "none"
- [ ] Check handle setting
  - `bBSPointerHandleExtremly=true` (TOML/new INI)
  - `BSHandleRefObjectPatch` (old)
  - Warn if not enabled
- [ ] Extract CK log file path

#### Deliverable
CKPE configuration validator matching batch lines 78-109.

---

### Phase 7: Testing & Polish

**Goal**: Comprehensive testing and error handling

#### Tasks
- [ ] Unit tests
  - Registry parsing
  - Path validation
  - Plugin name validation
  - Version extraction
- [ ] Integration tests
  - Mock external tools
  - Test workflow state transitions
- [ ] Error messages
  - Match original messages for familiarity
  - Add helpful context where appropriate
- [ ] Documentation
  - README with usage examples
  - Doc comments on public APIs
- [ ] Logging improvements
  - Structured logging with levels
  - Separate summary log and detailed log

#### Deliverable
Production-ready binary with tests.

---

## Dependencies (Cargo.toml)

```toml
[dependencies]
clap = { version = "4.5", features = ["derive"] }
anyhow = "1.0"
dialoguer = "0.11"
log = "0.4"
env_logger = "0.11"
walkdir = "2.5"
winreg = "0.52"
tempfile = "3.10"

[dependencies.windows]
version = "0.58"
features = [
    "Win32_Foundation",
    "Win32_System_Registry",
    "Win32_UI_Input_KeyboardAndMouse",
    "Win32_UI_WindowsAndMessaging",
    "Win32_Storage_FileSystem",
]

[dev-dependencies]
assert_cmd = "2.0"
predicates = "3.1"
```

## Milestones

### M1: Basic CLI (Phase 1)
- Can parse arguments
- Can find all tools via registry
- Displays version information

### M2: Validation & Prompts (Phases 2-3)
- All validation checks working
- Interactive mode fully functional
- Non-interactive mode supported

### M3: Tool Integration (Phase 4)
- All tool wrappers complete
- Keystroke automation working
- DLL management working

### M4: Complete Workflow (Phases 5-6)
- All 8 steps execute successfully
- Resume capability works
- CKPE validation complete

### M5: Production Ready (Phase 7)
- Tests passing
- Documentation complete
- Ready for real-world usage

## Risk Areas

1. **Keystroke Automation**: Most complex part, may need iteration
2. **MO2 Timing**: May need tunable delays per-system
3. **CKPE Config Parsing**: Three different formats to handle
4. **CK Exit Codes**: Non-zero exit doesn't always mean failure
5. **Log Parsing**: Fragile if tool output changes

## Future Enhancements (Post-V1)

- Configuration file (TOML) for default settings
- Parallel execution where safe
- Progress bars
- Better log parsing with structured output
- Auto-detection of MO2 mode
- Dry-run mode to preview steps
