# Technical Requirements

## Windows-Only APIs Needed

- **Registry**: Read HKCR and HKLM for tool paths
- **SendInput**: Keyboard automation for FO4Edit
- **File Versioning**: Get .exe ProductVersion info
- **Process Management**: Launch, wait, force-close external tools

## Recommended Crates

- `clap` - CLI argument parsing
- `dialoguer` - Interactive prompts (Y/N confirmations)
- `winreg` - Windows Registry access
- `windows` or `winapi` - Win32 APIs (SendInput, version info)
- `walkdir` - Directory traversal for validation
- `anyhow` or `thiserror` - Error handling
- `log` + `env_logger` or `tracing` - Logging

## Code Style

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
