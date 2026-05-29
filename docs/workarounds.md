# Required Workarounds

These workarounds are **necessary** due to external tool limitations - DO NOT "FIX" THESE.

## 1. PowerShell Keystroke Automation (batch lines 499-511)

- FO4Edit has no true headless mode
- Must send ENTER keystroke to Module Selection dialog
- Must force-close window after completion (despite `-autoexit` flag)
- **Rust must replicate this using Windows SendInput API**

## 2. MO2 Timing Delays (batch lines 169, 436, 497, 513)

- Mod Organizer 2 virtual file system introduces real timing issues
- 5-10 second delays are required for VFS synchronization
- **Keep these delays; they're not arbitrary**

## 3. DLL Renaming (batch lines 422-427, 330-335)

- CreationKit crashes with ENB/ReShade DLLs loaded
- Must rename d3d*.dll, dxgi.dll, enbimgui.dll to .dll-PJMdisabled
- Must restore after CK exits
- **Preserve this exactly**

## 4. Archive2 Extract-Repack (batch lines 390-414)

- Archive2.exe has no append functionality
- Must extract, add files, re-archive
- **This is an Archive2.exe limitation, not inefficient code**
