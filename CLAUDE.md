# GeneratePrevisibines - Project Guidelines

## Project Purpose

Translating the `GeneratePrevisibines.bat` Windows batch script (500+ lines) into idiomatic Rust. This script automates Fallout 4 mod development workflows using Creation Kit and FO4Edit.

## Critical Constraints

### Non-Automatable Tools

**IMPORTANT**: The external programs (CreationKit, FO4Edit, Mod Organizer 2) are NOT designed for automation. The workarounds in the batch script are **necessary**, not code smell to be refactored away.

### Required Workarounds - DO NOT "FIX" THESE

1. **PowerShell Keystroke Automation** (batch lines 499-511)
   - FO4Edit has no true headless mode
   - Must send ENTER keystroke to Module Selection dialog
   - Must force-close window after completion (despite `-autoexit` flag)
   - **Rust must replicate this using Windows SendInput API**

2. **MO2 Timing Delays** (batch lines 169, 436, 497, 513)
   - Mod Organizer 2 virtual file system introduces real timing issues
   - 5-10 second delays are required for VFS synchronization
   - **Keep these delays; they're not arbitrary**

3. **DLL Renaming** (batch lines 422-427, 330-335)
   - CreationKit crashes with ENB/ReShade DLLs loaded
   - Must rename d3d*.dll, dxgi.dll, enbimgui.dll to .dll-PJMdisabled
   - Must restore after CK exits
   - **Preserve this exactly**

4. **Archive2 Extract-Repack** (batch lines 390-414)
   - Archive2.exe has no append functionality
   - Must extract, add files, re-archive
   - **This is an Archive2.exe limitation, not inefficient code**

## Key Behaviors to Preserve

### From Original Script

- **Reserved Plugin Names**: "previs", "combinedobjects", "xprevispatch" are forbidden (lines 147-154)
- **Clean Mode Space Check**: Plugin names cannot contain spaces in clean mode (lines 134-158)
- **8-Step Resume**: Users can restart from any step 1-8 after failures (lines 186-207)
- **CKPE Config Checking**: Must validate `bBSPointerHandleExtremly=true` setting exists
- **Multiple Config Locations**: CKPE may use .toml, .ini, or fallout4_test.ini with different setting names
- **Version Display**: Show version info for FO4Edit, Fallout4.exe, CreationKit, CKPE
- **Error Messages**: Match original error messages for user familiarity

### UX Expectations

- Interactive prompts using CHOICE-style Y/N confirmations
- Ability to run non-interactively with plugin name parameter
- Clear step-by-step progress messages
- Comprehensive logging to temp file
- Command-line parameters: `-clean`/`-filtered`/`-xbox`, `-bsarch`, `-FO4:<dir>`, `<plugin.esp>`

## Technical Requirements

### Windows-Only APIs Needed

- **Registry**: Read HKCR and HKLM for tool paths
- **SendInput**: Keyboard automation for FO4Edit
- **File Versioning**: Get .exe ProductVersion info
- **Process Management**: Launch, wait, force-close external tools

### Recommended Crates

- `clap` - CLI argument parsing
- `dialoguer` - Interactive prompts (Y/N confirmations)
- `winreg` - Windows Registry access
- `windows` or `winapi` - Win32 APIs (SendInput, version info)
- `walkdir` - Directory traversal for validation
- `anyhow` or `thiserror` - Error handling
- `log` + `env_logger` or `tracing` - Logging

### Code Style

- Document WHY workarounds exist (prevent future "optimization" attempts)
- Use descriptive variable names matching batch script concepts
- Prefer explicit error handling
- Keep tool-specific logic isolated in wrapper functions

## Testing Limitations

- Cannot fully test without actual FO4/CreationKit installation
- Mock external tools for unit tests
- Real-world validation required for integration tests
- Test directory validation logic with temporary directories

## Success Criteria

1. Completes all 8 workflow steps successfully
2. Produces identical output to batch script
3. Maintains all necessary workarounds
4. Better error messages than batch script
5. Users familiar with batch version feel at home
