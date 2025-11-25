# Key Behaviors to Preserve

## From Original Script

- **Reserved Plugin Names**: "previs", "combinedobjects", "xprevispatch" are forbidden (lines 147-154)
- **Clean Mode Space Check**: Plugin names cannot contain spaces in clean mode (lines 134-158)
- **8-Step Resume**: Users can restart from any step 1-8 after failures (lines 186-207)
- **CKPE Config Checking**: Must validate `bBSPointerHandleExtremly=true` setting exists
- **Multiple Config Locations**: CKPE may use .toml, .ini, or fallout4_test.ini with different setting names
- **Version Display**: Show version info for FO4Edit, Fallout4.exe, CreationKit, CKPE
- **Error Messages**: Match original error messages for user familiarity

## UX Expectations

- Interactive prompts using CHOICE-style Y/N confirmations
- Ability to run non-interactively with plugin name parameter
- Clear step-by-step progress messages
- Comprehensive logging to temp file
- Command-line parameters: `-clean`/`-filtered`/`-xbox`, `-bsarch`, `-FO4:<dir>`, `<plugin.esp>`
