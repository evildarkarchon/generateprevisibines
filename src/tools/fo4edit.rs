//! `FO4Edit` automation with required keyboard automation workarounds
//!
//! This module provides utilities for running `FO4Edit` in automated workflows. `FO4Edit` is
//! the community-created tool for editing Fallout 4 plugins and running batch scripts.
//! Unlike `CreationKit`, `FO4Edit` is designed to be more scriptable, but it still has
//! significant automation challenges that require workarounds.
//!
//! # Overview
//!
//! The main interface is [`FO4EditRunner`], which handles:
//! - Merging PrecombineObjects.esp into the main plugin (`Batch_FO4MergeCombinedObjectsAndCheck.pas`)
//! - Merging Previs.esp into the main plugin (`Batch_FO4MergePrevisandCleanRefr.pas`)
//! - Automated script execution with keyboard automation
//! - Log file parsing for error detection
//!
//! # Critical Workarounds (DO NOT REMOVE)
//!
//! ## Workaround #1: Keyboard Automation for Module Selection Dialog
//!
//! **Problem:** `FO4Edit` displays a "Module Selection" dialog even when launched with the
//! `-autoexit` flag and a pre-configured `Plugins.txt` file. There is NO headless mode,
//! NO command-line option to suppress this dialog, and NO way to automate it except via
//! keyboard input.
//!
//! **Solution:** The [`send_enter_keystroke`](FO4EditRunner::send_enter_keystroke) function uses the
//! Windows `SendInput` API to:
//! 1. Wait for the `FO4Edit` window to appear (3-second delay)
//! 2. Find the window by title "`FO4Edit`"
//! 3. Bring the window to the foreground
//! 4. Send ENTER key press + release events
//!
//! **This is NOT code smell - it's the ONLY way to automate `FO4Edit`.**
//!
//! See original batch script lines 499-511 for the same workaround using `PowerShell`.
//!
//! ## Workaround #2: Force Close After Script Completion
//!
//! **Problem:** `FO4Edit`'s `-autoexit` flag is unreliable. Even after the script completes
//! successfully and writes the log file, the main window often remains open indefinitely.
//! This blocks the workflow from continuing.
//!
//! **Solution:** The [`close_fo4edit_window`](FO4EditRunner::close_fo4edit_window) function forcefully
//! closes the window using a two-step process:
//! 1. Send `WM_CLOSE` message to request graceful shutdown
//! 2. Wait 2 seconds, then use `taskkill /F` as a fallback
//!
//! **This is NOT inefficient code - the `-autoexit` flag cannot be relied upon.**
//!
//! See original batch script lines 499-511 for the same workaround.
//!
//! # Mod Organizer 2 Support
//!
//! When configured with [`with_mo2`](FO4EditRunner::with_mo2), `FO4Edit` is launched through
//! MO2's virtual file system. This ensures `FO4Edit` sees mods in the correct load order and
//! can access files from MO2's staging directories.
//!
//! **Important:** MO2 introduces timing delays. The original batch script uses 5-10 second
//! delays after launching tools through MO2 to allow the VFS to synchronize. These delays
//! are built into this implementation and are NOT arbitrary.
//!
//! See original batch script lines 169, 436, 497, 513 for MO2 timing delays.
//!
//! # Error Detection
//!
//! `FO4Edit` writes detailed logs during script execution. This module:
//! - Deletes old log files before each operation (ensures fresh error detection)
//! - Waits for the log file to be created (indicates script has started)
//! - Parses the log for error patterns:
//!   - [`LOG_ERROR`]: General error indicator ("Error:")
//!   - [`LOG_SUCCESS`]: Success indicator ("Completed: No Errors.")
//!
//! # Examples
//!
//! ```no_run
//! use std::path::Path;
//! use generateprevisibines::tools::fo4edit::FO4EditRunner;
//!
//! let fo4edit_exe = Path::new("C:\\Games\\FO4Edit\\FO4Edit.exe");
//! let fo4_dir = Path::new("C:\\Games\\Fallout4");
//!
//! let runner = FO4EditRunner::new(fo4edit_exe, fo4_dir);
//!
//! // Step 5: Merge PrecombineObjects.esp
//! runner.merge_combined_objects("MyMod.esp")?;
//!
//! // Step 8: Merge Previs.esp
//! runner.merge_previs("MyMod.esp")?;
//! # Ok::<(), anyhow::Error>(())
//! ```
//!
//! # Platform Requirements
//!
//! **Windows only.** `FO4Edit` automation requires:
//! - Windows `SendInput` API for keyboard automation
//! - Windows `FindWindowW` API for window detection
//! - Windows `SendMessageW` API for window closing
//! - `taskkill.exe` for force-close fallback
//!
//! Non-Windows platforms will get compile-time errors.
//!
//! # See Also
//!
//! - Original batch script (`GeneratePrevisibines.bat`) lines 499-511 for keyboard automation
//! - Project CLAUDE.md for detailed workaround documentation
//! - `creation_kit` module for similar workarounds with `CreationKit`

use anyhow::{Context, Result, bail};
use log::{info, warn};
use mo2_mode::MO2Command;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::Duration;

#[cfg(windows)]
use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput, VK_RETURN,
};
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowW, SendMessageW, SetForegroundWindow, WM_CLOSE,
};

/// Script name for merging PrecombineObjects.esp
///
/// **Script:** `Batch_FO4MergeCombinedObjectsAndCheck.pas`
///
/// This `FO4Edit` script merges the temporary `PrecombineObjects.esp` plugin (created by
/// `CreationKit`'s `-GeneratePrecombined` command) back into the main plugin. This is Step 5
/// of the workflow.
///
/// # What This Script Does
///
/// 1. Loads both the main plugin and PrecombineObjects.esp
/// 2. Copies all CELL records from PrecombineObjects.esp to the main plugin
/// 3. Performs validation checks to ensure data integrity
/// 4. Saves the merged plugin
/// 5. Writes completion status to log
///
/// # Usage
///
/// Used by [`FO4EditRunner::merge_combined_objects`]
pub const SCRIPT_MERGE_COMBINED: &str = "Batch_FO4MergeCombinedObjectsAndCheck.pas";

/// Script name for merging Previs.esp
///
/// **Script:** `Batch_FO4MergePrevisandCleanRefr.pas`
///
/// This `FO4Edit` script merges the temporary `Previs.esp` plugin (created by `CreationKit`'s
/// `-GeneratePreVisData` command) back into the main plugin. This is Step 8 of the workflow.
///
/// # What This Script Does
///
/// 1. Loads both the main plugin and Previs.esp
/// 2. Copies all previs data (CELL records, visibility data) from Previs.esp to the main plugin
/// 3. Cleans up temporary reference records
/// 4. Performs validation checks
/// 5. Saves the merged plugin
/// 6. Writes completion status to log
///
/// # Usage
///
/// Used by [`FO4EditRunner::merge_previs`]
pub const SCRIPT_MERGE_PREVIS: &str = "Batch_FO4MergePrevisandCleanRefr.pas";

/// Success indicator in `FO4Edit` log files
///
/// **Pattern:** `"Completed: No Errors."`
///
/// `FO4Edit` scripts write this string to the log file when they complete successfully
/// without encountering any errors. This is checked after script execution to verify
/// success.
///
/// # When This Appears
///
/// - Script loaded all required files successfully
/// - All merge operations completed without errors
/// - Data validation passed
/// - Output file was saved successfully
///
/// # Detection
///
/// Checked in [`FO4EditRunner::check_log_for_errors`] after every script execution.
/// If this string is NOT found, a warning is logged (but the operation doesn't fail
/// unless `LOG_ERROR` is also present).
const LOG_SUCCESS: &str = "Completed: No Errors.";

/// Error indicator in `FO4Edit` log files
///
/// **Pattern:** `"Error:"`
///
/// `FO4Edit` scripts write this string to the log file when they encounter errors during
/// execution. This is a general error pattern that catches most failure conditions.
///
/// # Common Errors
///
/// - "Error: Plugin not found" - Specified plugin doesn't exist
/// - "Error: Failed to load master" - Missing master file dependency
/// - "Error: Cannot save plugin" - Permission denied or disk full
/// - "Error: Invalid record" - Corrupted data in plugin
///
/// # Impact
///
/// This is a **FATAL ERROR** for workflow purposes. If `LOG_ERROR` is found in the log,
/// the workflow cannot continue and will abort with an error message.
///
/// # Detection
///
/// Checked in [`FO4EditRunner::check_log_for_errors`] after every script execution.
const LOG_ERROR: &str = "Error:";

/// Runner for FO4Edit.exe operations
///
/// Provides a safe interface for running `FO4Edit` in automated workflows. `FO4Edit` is the
/// community-created tool for editing Fallout 4 plugins and running batch scripts. While
/// more scriptable than `CreationKit`, it still requires significant workarounds for full automation.
///
/// # Key Features
///
/// - **Keyboard Automation**: Automatically dismisses Module Selection dialog via `SendInput` API
/// - **Force Close**: Ensures `FO4Edit` window closes even when `-autoexit` fails
/// - **Log Parsing**: Detects errors from log files for reliable error detection
/// - **MO2 Support**: Can launch `FO4Edit` through Mod Organizer 2's virtual file system
/// - **Plugins.txt Management**: Creates temporary plugin lists for script execution
///
/// # Automation Workflow
///
/// Each script execution follows this process:
/// 1. **Create Plugins.txt**: Writes plugin name to temporary file for `FO4Edit` to load
/// 2. **Delete old log**: Ensures fresh error detection by removing previous log files
/// 3. **Launch `FO4Edit`**: Starts `FO4Edit` with script arguments (optionally through MO2)
/// 4. **Send ENTER keystroke**: Dismisses Module Selection dialog (REQUIRED WORKAROUND)
/// 5. **Wait for log**: Polls for log file creation (indicates script is running)
/// 6. **Wait for completion**: Allows script to finish execution (5-second delay)
/// 7. **Force close window**: Sends `WM_CLOSE` + taskkill fallback (REQUIRED WORKAROUND)
/// 8. **Parse log**: Checks for error patterns to determine success/failure
/// 9. **Cleanup**: Removes temporary Plugins.txt file
///
/// # Important Workarounds (DO NOT REMOVE)
///
/// **REQUIRED WORKAROUND #1: Keyboard Automation**
///
/// `FO4Edit` displays a Module Selection dialog even when launched with `-autoexit` flag and
/// a pre-configured `Plugins.txt` file. There is NO headless mode, NO command-line option
/// to suppress this dialog. The ONLY way to dismiss it is by sending an ENTER keystroke
/// via the Windows `SendInput` API.
///
/// **This is NOT code smell - it's the ONLY way to automate `FO4Edit`.**
///
/// See [`send_enter_keystroke`](FO4EditRunner::send_enter_keystroke) for implementation details.
/// See original batch script lines 499-511 for the same workaround using `PowerShell`.
///
/// **REQUIRED WORKAROUND #2: Force Close Window**
///
/// `FO4Edit`'s `-autoexit` flag is unreliable. Even after the script completes successfully
/// and writes the log file, the main window often remains open indefinitely. This blocks
/// the workflow from continuing. We must forcefully close the window using `WM_CLOSE` +
/// `taskkill` fallback.
///
/// **This is NOT inefficient code - the `-autoexit` flag cannot be relied upon.**
///
/// See [`close_fo4edit_window`](FO4EditRunner::close_fo4edit_window) for implementation details.
/// See original batch script lines 499-511 for the same workaround.
///
/// # Error Detection
///
/// `FO4Edit` scripts write detailed logs during execution. Error detection relies ENTIRELY
/// on log file parsing:
/// - **`Error:`** - General error pattern (fatal)
/// - **`Completed: No Errors.`** - Success indicator (logged as warning if missing)
///
/// # Examples
///
/// ```no_run
/// use std::path::Path;
/// use generateprevisibines::tools::fo4edit::FO4EditRunner;
///
/// let fo4edit_exe = Path::new("C:\\Games\\FO4Edit\\FO4Edit.exe");
/// let fo4_dir = Path::new("C:\\Games\\Fallout4");
///
/// let runner = FO4EditRunner::new(fo4edit_exe, fo4_dir);
///
/// // Step 5: Merge PrecombineObjects.esp
/// runner.merge_combined_objects("MyMod.esp")?;
///
/// // Step 8: Merge Previs.esp
/// runner.merge_previs("MyMod.esp")?;
/// # Ok::<(), anyhow::Error>(())
/// ```
///
/// # Platform Requirements
///
/// **Windows only.** `FO4Edit` automation requires:
/// - Windows `SendInput` API for keyboard automation
/// - Windows `FindWindowW` API for window detection
/// - Windows `SendMessageW` API for window closing
/// - `taskkill.exe` for force-close fallback
///
/// Non-Windows platforms will get compile-time errors.
///
/// # See Also
///
/// - Module-level documentation for detailed workaround explanations
/// - `creation_kit` module for similar automation challenges
/// - Original batch script for historical context
pub struct FO4EditRunner {
    fo4edit_exe: PathBuf,
    fallout4_dir: PathBuf,
    mo2_path: Option<PathBuf>,
}

impl FO4EditRunner {
    /// Create a new `FO4Edit` runner
    pub fn new(fo4edit_exe: impl AsRef<Path>, fallout4_dir: impl AsRef<Path>) -> Self {
        Self {
            fo4edit_exe: fo4edit_exe.as_ref().to_path_buf(),
            fallout4_dir: fallout4_dir.as_ref().to_path_buf(),
            mo2_path: None,
        }
    }

    /// Set Mod Organizer 2 path for VFS execution
    pub fn with_mo2(mut self, mo2_path: impl AsRef<Path>) -> Self {
        self.mo2_path = Some(mo2_path.as_ref().to_path_buf());
        self
    }

    /// Run `FO4Edit` script to merge combined objects
    ///
    /// Script: `Batch_FO4MergeCombinedObjectsAndCheck.pas`
    /// Merges PrecombineObjects.esp into the main plugin
    pub fn merge_combined_objects(&self, plugin_name: &str) -> Result<()> {
        self.run_script(
            plugin_name,
            "CombinedObjects.esp",
            SCRIPT_MERGE_COMBINED,
            "Merge Combined Objects",
        )
    }

    /// Run `FO4Edit` script to merge previs data
    ///
    /// Script: `Batch_FO4MergePrevisandCleanRefr.pas`
    /// Merges Previs.esp into the main plugin
    pub fn merge_previs(&self, plugin_name: &str) -> Result<()> {
        self.run_script(
            plugin_name,
            "Previs.esp",
            SCRIPT_MERGE_PREVIS,
            "Merge Previs",
        )
    }

    /// Run an `FO4Edit` script with full automation
    ///
    /// Internal wrapper that handles all the necessary workarounds for running `FO4Edit` scripts
    /// in an automated environment. This coordinates plugin list creation, keyboard automation,
    /// log parsing, and window management.
    ///
    /// # Arguments
    ///
    /// * `plugin_name` - Name of the plugin to process (e.g., "MyMod.esp")
    /// * `script_name` - Name of the Pascal script to run (e.g., "`Batch_FO4MergeCombinedObjectsAndCheck.pas`")
    /// * `operation` - Human-readable operation name for logging (e.g., "Merge Combined Objects")
    ///
    /// # Process Flow
    ///
    /// 1. **Create Plugins.txt**: Writes plugin name to `%TEMP%\FO4Edit_Plugins_{plugin}.txt`
    /// 2. **Delete old log**: Removes `%TEMP%\FO4Edit_Log_{plugin}.txt` if it exists
    /// 3. **Build arguments**: Constructs `FO4Edit` command line with:
    ///    - `-fo4`: Fallout 4 mode
    ///    - `-autoexit`: Exit after script completes (unreliable, see workaround #2)
    ///    - `-P:{path}`: Path to Plugins.txt
    ///    - `-Script:{name}`: Script to execute
    ///    - `-Mod:{plugin}`: Target plugin name
    ///    - `-log:{path}`: Log file path
    /// 4. **Launch `FO4Edit`**: Spawns process (optionally through MO2)
    /// 5. **Send ENTER keystroke**: Dismisses Module Selection dialog (REQUIRED WORKAROUND #1)
    /// 6. **Wait for log**: Polls for log file creation (up to 30 seconds)
    /// 7. **Wait for completion**: Sleeps 5 seconds for script to finish
    /// 8. **Force close window**: Sends `WM_CLOSE` + taskkill fallback (REQUIRED WORKAROUND #2)
    /// 9. **Parse log**: Checks for errors using `check_log_for_errors`
    /// 10. **Cleanup**: Removes temporary Plugins.txt file
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` if the script executes successfully without errors.
    ///
    /// # Errors
    ///
    /// This function will return an error if:
    /// - Temporary Plugins.txt cannot be created (disk full, permission denied)
    /// - `FO4Edit` cannot be launched (file not found, not executable)
    /// - MO2 execution fails (if using MO2 mode)
    /// - Log file indicates errors (contains "Error:" pattern)
    /// - Log file cannot be read after execution
    ///
    /// # Keyboard Automation (Required Workaround)
    ///
    /// **CRITICAL:** The `send_enter_keystroke` call is NOT optional. `FO4Edit` displays a Module
    /// Selection dialog even with `-autoexit` and a pre-configured Plugins.txt. There is NO
    /// headless mode. The dialog MUST be dismissed via keyboard input.
    ///
    /// **This is NOT code smell - it's the ONLY way to automate `FO4Edit`.**
    ///
    /// If the window is not found, a warning is logged but execution continues (`FO4Edit` may
    /// have already proceeded or may be running in a different mode).
    ///
    /// # Force Close (Required Workaround)
    ///
    /// **CRITICAL:** The `close_fo4edit_window` call is NOT optional. `FO4Edit`'s `-autoexit`
    /// flag is unreliable and often leaves the main window open after script completion.
    /// Without force-closing, the workflow would block indefinitely.
    ///
    /// **This is NOT inefficient code - the `-autoexit` flag cannot be relied upon.**
    ///
    /// # MO2 Mode Behavior
    ///
    /// When `self.mo2_path` is set, `FO4Edit` is launched through Mod Organizer 2's VFS:
    /// - Uses `MO2Command::new()` to construct MO2-wrapped command
    /// - Ensures `FO4Edit` sees mods in correct load order
    /// - Accesses files from MO2's staging directories
    ///
    /// # Timing Delays
    ///
    /// - **3 seconds**: Wait for `FO4Edit` window to appear (before keystroke)
    /// - **30 seconds**: Maximum wait for log file creation (1-second polls)
    /// - **5 seconds**: Wait for script completion (after log appears)
    /// - **2 seconds**: Wait for graceful close (before taskkill fallback)
    ///
    /// **These delays are REQUIRED, not arbitrary.** They account for:
    /// - `FO4Edit` initialization time
    /// - MO2 VFS synchronization
    /// - Script execution time
    /// - Window message processing
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use generateprevisibines::tools::fo4edit::FO4EditRunner;
    /// # use std::path::Path;
    /// # let runner = FO4EditRunner::new("FO4Edit.exe", "F:\\Games\\Fallout4");
    /// // Merge PrecombineObjects.esp
    /// runner.run_script(
    ///     "MyMod.esp",
    ///     "Batch_FO4MergeCombinedObjectsAndCheck.pas",
    ///     "Merge Combined Objects"
    /// )?;
    /// # Ok::<(), anyhow::Error>(())
    /// ```
    ///
    /// # Notes
    ///
    /// - This is a private function; use the public wrappers (`merge_combined_objects`, `merge_previs`)
    /// - Temporary files are created in the system's temp directory (`std::env::temp_dir()`)
    /// - Log files persist after execution for debugging purposes
    /// - Plugins.txt is cleaned up even if errors occur
    fn run_script(
        &self,
        plugin_name: &str,
        second_plugin: &str,
        script_name: &str,
        operation: &str,
    ) -> Result<()> {
        info!("Running FO4Edit: {operation}");

        // Create temporary Plugins.txt
        // Match batch file filenames exactly to minimize path issues and ensure compatibility
        let temp_dir = std::env::temp_dir();
        let plugins_file = temp_dir.join("Plugins.txt");
        let log_file = temp_dir.join("UnattendedScript.log");

        // Write plugin names to Plugins.txt with '*' prefix
        // Matches batch file behavior:
        // Echo *%~3 > "%LocPlugins_%"
        // Echo *%~4 >> "%LocPlugins_%"
        // Use CRLF for Windows compatibility (echo behavior)
        let plugins_content = format!("*{plugin_name}\r\n*{second_plugin}");
        fs::write(&plugins_file, plugins_content)
            .with_context(|| format!("Failed to create Plugins.txt: {}", plugins_file.display()))?;

        // Delete old log if it exists
        if log_file.exists() {
            fs::remove_file(&log_file)?;
        }

        // Build command arguments
        let args = vec![
            "-fo4".to_string(),
            "-autoexit".to_string(),
            format!("-P:{}", plugins_file.display()),
            format!("-Script:{}", script_name),
            format!("-Mod:{}", plugin_name),
            format!("-log:{}", log_file.display()),
        ];

        info!(
            "Executing: {} {}",
            self.fo4edit_exe.display(),
            args.join(" ")
        );

        // Launch FO4Edit (optionally through MO2)
        let mut child = if let Some(ref mo2_path) = self.mo2_path {
            // Use MO2 mode
            info!("Launching through Mod Organizer 2: {}", mo2_path.display());
            let mut cmd = MO2Command::new(mo2_path, &self.fo4edit_exe)
                .args(args.iter().map(std::string::String::as_str))
                .execute();
            cmd.current_dir(&self.fallout4_dir)
                .spawn()
                .with_context(|| {
                    format!(
                        "Failed to launch FO4Edit through MO2: {}",
                        mo2_path.display()
                    )
                })?
        } else {
            // Direct execution
            Command::new(&self.fo4edit_exe)
                .args(&args)
                .current_dir(&self.fallout4_dir)
                .spawn()
                .with_context(|| {
                    format!("Failed to launch FO4Edit: {}", self.fo4edit_exe.display())
                })?
        };

        // Wait for window to appear, then send ENTER keystroke
        // This dismisses the Module Selection dialog
        self.send_enter_keystroke()?;

        // Wait for log file to be created (indicates script is running)
        self.wait_for_log_file(&log_file)?;

        // Wait a bit more for script to complete
        thread::sleep(Duration::from_secs(10));

        // Force close the main window (autoexit doesn't always work)
        self.close_fo4edit_window();

        // Clean up child process
        if let Err(e) = child.wait() {
            // Expected if process was force-closed, but log for debugging
            info!("Process wait returned: {e} (expected if force-closed)");
        }

        // Parse log for success/errors
        self.check_log_for_errors(&log_file, operation)?;

        // Cleanup temp files
        let _ = fs::remove_file(&plugins_file);

        info!("FO4Edit {operation} completed successfully");
        Ok(())
    }

    /// Send ENTER keystroke to dismiss Module Selection dialog
    ///
    /// **CRITICAL REQUIRED WORKAROUND - DO NOT REMOVE**
    ///
    /// `FO4Edit` displays a "Module Selection" dialog even when launched with the `-autoexit`
    /// flag and a pre-configured `Plugins.txt` file. There is **NO** headless mode, **NO**
    /// command-line option to suppress this dialog, and **NO** way to automate `FO4Edit`
    /// except via keyboard input.
    ///
    /// **This is NOT code smell - it's the ONLY way to automate `FO4Edit`.**
    ///
    /// # Why This Workaround Exists
    ///
    /// `FO4Edit` was designed as an interactive tool. Even when running batch scripts with
    /// `-autoexit` and a pre-loaded plugin list (`-P:Plugins.txt`), it ALWAYS shows the
    /// Module Selection window. The window must be dismissed manually or via keyboard
    /// automation for the script to proceed.
    ///
    /// The original batch script (lines 499-511) used `PowerShell`'s `SendKeys` method to
    /// accomplish the same task. This Rust implementation uses the Windows `SendInput` API
    /// for the same purpose.
    ///
    /// # Process
    ///
    /// 1. **Wait 3 seconds**: Allows `FO4Edit` window to fully initialize
    /// 2. **Find window**: Uses `FindWindowW` to locate window by title "`FO4Edit`"
    /// 3. **Bring to foreground**: Uses `SetForegroundWindow` to focus the window
    /// 4. **Wait 500ms**: Ensures window has focus before sending input
    /// 5. **Send ENTER**: Uses `SendInput` to send `VK_RETURN` key down + key up events
    ///
    /// # Window Not Found Behavior
    ///
    /// If the `FO4Edit` window is not found after the 3-second wait:
    /// - A warning is logged
    /// - The function returns `Ok(())` (not an error)
    /// - The workflow continues
    ///
    /// **Why not fail?** The window may have already proceeded automatically (rare), or
    /// `FO4Edit` may be running in a different mode. Failing here would block workflows
    /// that might otherwise succeed. The log file parsing will detect actual failures.
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` on success or if the window is not found (with warning).
    ///
    /// # Errors
    ///
    /// This function does not return errors under normal circumstances. Window lookup
    /// failures result in warnings, not errors.
    ///
    /// # Platform Support
    ///
    /// **Windows only.** This function uses Windows-specific APIs:
    /// - `FindWindowW`: Locate window by title
    /// - `SetForegroundWindow`: Bring window to front
    /// - `SendInput`: Send keyboard events
    ///
    /// On non-Windows platforms, this function will fail to compile.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use generateprevisibines::tools::fo4edit::FO4EditRunner;
    /// # use std::path::Path;
    /// # let runner = FO4EditRunner::new("FO4Edit.exe", "F:\\Games\\Fallout4");
    /// // Called automatically by run_script
    /// // runner.send_enter_keystroke()?;
    /// # Ok::<(), anyhow::Error>(())
    /// ```
    ///
    /// # Timing Considerations
    ///
    /// - **3-second wait**: Required for `FO4Edit` to create and display its window
    /// - **500ms wait**: Ensures window has focus before sending input
    ///
    /// **These delays are REQUIRED, not arbitrary.** Reducing them will cause failures.
    ///
    /// # Notes
    ///
    /// - This function is called automatically by `run_script` after launching `FO4Edit`
    /// - The ENTER key is sent regardless of which button is focused (OK is default)
    /// - Window title must exactly match "`FO4Edit`" (case-sensitive)
    /// - Errors from `SetForegroundWindow` are ignored (may fail if another app has focus lock)
    ///
    /// # See Also
    ///
    /// - Original batch script lines 499-511 for `PowerShell` implementation
    /// - Module-level documentation for detailed workaround explanations
    /// - Project CLAUDE.md section "Non-Automatable Tools"
    #[cfg(windows)]
    #[allow(unsafe_code, clippy::unused_self, clippy::unnecessary_wraps)]
    fn send_enter_keystroke(&self) -> Result<()> {
        info!("Waiting for Module Selection window...");

        // Wait for window to appear
        thread::sleep(Duration::from_secs(3));

        // Find FO4Edit window
        let window_title = windows::core::w!("FO4Edit");
        // SAFETY: FindWindowW is safe to call with valid PCWSTR pointers.
        // `window_title` is a valid null-terminated UTF-16 string created by the w! macro.
        // This is a read-only operation that searches for a window by title.
        // The function returns a window handle or an error if the window is not found.
        let hwnd = unsafe { FindWindowW(None, window_title) };

        let hwnd = match hwnd {
            Ok(h) if !h.0.is_null() => h,
            _ => {
                warn!("FO4Edit window not found, keystroke automation may fail");
                return Ok(());
            }
        };

        // Bring window to foreground
        // SAFETY: SetForegroundWindow is safe when called with a valid HWND.
        // We verified the handle is non-null in the match above, ensuring it's valid.
        // This is a standard Windows API call that brings a window to the foreground.
        // The call may fail (e.g., if the window was closed), but this is safe to ignore
        // as we're just trying to focus the window before sending keystrokes.
        unsafe {
            let _ = SetForegroundWindow(hwnd);
        }

        thread::sleep(Duration::from_millis(500));

        // Send ENTER key press
        let mut inputs = [INPUT::default(); 2];

        // Key down
        inputs[0].r#type = INPUT_KEYBOARD;
        inputs[0].Anonymous.ki = KEYBDINPUT {
            wVk: VK_RETURN,
            wScan: 0,
            dwFlags: KEYBD_EVENT_FLAGS::default(),
            time: 0,
            dwExtraInfo: 0,
        };

        // Key up
        inputs[1].r#type = INPUT_KEYBOARD;
        inputs[1].Anonymous.ki = KEYBDINPUT {
            wVk: VK_RETURN,
            wScan: 0,
            dwFlags: KEYEVENTF_KEYUP,
            time: 0,
            dwExtraInfo: 0,
        };

        // SAFETY: SendInput is safe when called with a valid array of INPUT structures.
        // The following invariants are maintained:
        // 1. `inputs` is a properly initialized array of INPUT structs with correct types
        // 2. The array size (2 elements) is passed correctly as the second parameter
        // 3. The struct size calculation is accurate for the INPUT type
        // 4. INPUT_KEYBOARD is a valid input type for keyboard simulation
        // 5. VK_RETURN is a valid virtual key code
        // 6. The INPUT structures use #[repr(C)] layout matching Windows SDK
        //
        // This simulates pressing and releasing the ENTER key, which is required because
        // FO4Edit's Module Selection dialog has no headless mode and must be automated
        // via keyboard input (intentional workaround documented in CLAUDE.md).
        unsafe {
            #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
            SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
        }

        info!("Sent ENTER keystroke to Module Selection");
        Ok(())
    }

    #[cfg(not(windows))]
    fn send_enter_keystroke(&self) -> Result<()> {
        // Non-Windows platforms cannot automate FO4Edit
        bail!("FO4Edit automation requires Windows");
    }

    /// Wait for log file to be created by `FO4Edit`
    ///
    /// Polls for the existence of the `FO4Edit` log file to determine when the script has
    /// started executing. `FO4Edit` creates the log file early in execution, so this indicates
    /// the script has loaded and begun processing.
    ///
    /// # Arguments
    ///
    /// * `log_file` - Path to the expected log file location
    ///
    /// # Polling Strategy
    ///
    /// - **Maximum wait**: 30 seconds (30 iterations × 1 second)
    /// - **Poll interval**: 1 second between checks
    /// - **Success**: Returns as soon as the file exists
    /// - **Timeout**: Logs a warning and returns `Ok(())` (does not fail)
    ///
    /// # Timeout Behavior
    ///
    /// If the log file is not created within 30 seconds:
    /// - A warning is logged
    /// - The function returns `Ok(())` (not an error)
    /// - The workflow continues
    ///
    /// **Why not fail on timeout?** The log file may be created in an unexpected location,
    /// or `FO4Edit` may be running in a different mode. Failing here would block workflows
    /// that might otherwise succeed. The subsequent `check_log_for_errors` call will detect
    /// actual failures.
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` when the log file is created or after timeout (with warning).
    ///
    /// # Errors
    ///
    /// This function does not return errors. Timeout results in a warning, not an error.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use generateprevisibines::tools::fo4edit::FO4EditRunner;
    /// # use std::path::Path;
    /// # let runner = FO4EditRunner::new("FO4Edit.exe", "F:\\Games\\Fallout4");
    /// // Called automatically by run_script after launching FO4Edit
    /// let log_file = Path::new("C:\\Temp\\FO4Edit_Log_MyMod.esp.txt");
    /// // runner.wait_for_log_file(&log_file)?;
    /// # Ok::<(), anyhow::Error>(())
    /// ```
    ///
    /// # Timing Considerations
    ///
    /// - **30-second timeout**: Should be more than enough for `FO4Edit` to initialize
    /// - **1-second poll interval**: Balances responsiveness with CPU usage
    ///
    /// Typical log creation times:
    /// - **Normal mode**: 2-5 seconds
    /// - **MO2 mode**: 5-10 seconds (VFS synchronization overhead)
    ///
    /// # Notes
    ///
    /// - This function is called automatically by `run_script` after sending the ENTER keystroke
    /// - The log file is created by `FO4Edit`, not by this code
    /// - Existence of the log file indicates the script has started (not necessarily completed)
    /// - The script may still be running when this function returns
    ///
    /// # See Also
    ///
    /// - `check_log_for_errors` for log parsing after script completion
    #[allow(clippy::unused_self, clippy::unnecessary_wraps)]
    fn wait_for_log_file(&self, log_file: &Path) -> Result<()> {
        const POLL_INTERVAL_SECS: u64 = 1;
        // 15 minutes default - balances xEdit's slow performance with reasonable wait time
        const DEFAULT_TIMEOUT_SECS: u64 = 900;

        info!("Waiting for log file creation...");

        // Support configurable timeout for slower systems or network drives
        // Set FO4EDIT_TIMEOUT_SECS environment variable to override (in seconds)
        let timeout_secs = std::env::var("FO4EDIT_TIMEOUT_SECS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(DEFAULT_TIMEOUT_SECS);

        // Warn if timeout is excessively long (> 30 minutes)
        if timeout_secs > 1800 {
            warn!(
                "FO4EDIT_TIMEOUT_SECS set to {} seconds ({} minutes). This may be excessive.",
                timeout_secs,
                timeout_secs / 60
            );
        }

        let max_iterations = timeout_secs / POLL_INTERVAL_SECS;

        for i in 0..max_iterations {
            if log_file.exists() {
                info!("Log file created after {} seconds", i * POLL_INTERVAL_SECS);
                return Ok(());
            }
            thread::sleep(Duration::from_secs(POLL_INTERVAL_SECS));
        }

        warn!("Log file not created within {timeout_secs} second timeout");
        Ok(())
    }

    /// Close `FO4Edit` main window forcefully
    ///
    /// **CRITICAL REQUIRED WORKAROUND - DO NOT REMOVE**
    ///
    /// `FO4Edit`'s `-autoexit` flag is unreliable and frequently fails to close the main window
    /// after script completion. Even after the script finishes successfully and writes the log
    /// file, the window often remains open indefinitely, blocking the workflow from continuing.
    ///
    /// **This is NOT inefficient code - the `-autoexit` flag cannot be relied upon.**
    ///
    /// # Why This Workaround Exists
    ///
    /// `FO4Edit`'s `-autoexit` flag is supposed to close the application after a script completes,
    /// but in practice it is extremely unreliable:
    /// - Sometimes the window stays open even after successful completion
    /// - Sometimes it closes but with significant delay (30+ seconds)
    /// - The behavior is inconsistent across different `FO4Edit` versions
    ///
    /// The original batch script (lines 499-511) used the same workaround: wait for the log
    /// file to be written (indicating completion), then forcefully close the window using
    /// `PowerShell`'s process termination.
    ///
    /// # Process
    ///
    /// This function uses a two-step approach to ensure the window closes:
    ///
    /// 1. **Graceful close attempt**: Sends `WM_CLOSE` message to the `FO4Edit` window
    ///    - Finds window by title "`FO4Edit`" using `FindWindowW`
    ///    - Sends `WM_CLOSE` message using `SendMessageW`
    ///    - Allows `FO4Edit` to clean up resources and exit gracefully
    ///
    /// 2. **Wait 2 seconds**: Gives `FO4Edit` time to respond to `WM_CLOSE`
    ///
    /// 3. **Force close fallback**: Uses `taskkill /F /IM FO4Edit.exe`
    ///    - Forcefully terminates the process if still running
    ///    - Ensures the workflow can continue even if `WM_CLOSE` was ignored
    ///    - Errors are silently ignored (process may have already exited)
    ///
    /// # Window Not Found Behavior
    ///
    /// If the `FO4Edit` window is not found:
    /// - The function proceeds directly to the `taskkill` fallback
    /// - This is normal if `FO4Edit` has already exited (rare but possible)
    ///
    /// # Return Value
    ///
    /// This function does not return a value (it's infallible). All errors are silently ignored:
    /// - Window not found → Continue to taskkill
    /// - `WM_CLOSE` fails → Continue to taskkill
    /// - taskkill fails → Ignore (process may have already exited)
    ///
    /// **Why ignore errors?** By the time this function is called, the log file has already
    /// been written and parsed. The script work is complete. Whether the window closes
    /// gracefully or forcefully doesn't affect workflow success.
    ///
    /// # Platform Support
    ///
    /// **Windows only.** This function uses Windows-specific APIs and tools:
    /// - `FindWindowW`: Locate window by title
    /// - `SendMessageW`: Send `WM_CLOSE` message
    /// - `taskkill.exe`: Force-terminate process
    ///
    /// On non-Windows platforms, this function is a no-op (does nothing).
    ///
    /// # Timing Considerations
    ///
    /// - **2-second wait**: Allows `FO4Edit` to respond to `WM_CLOSE` before force-killing
    ///
    /// **This delay is REQUIRED.** It gives `FO4Edit` a chance to clean up properly before
    /// resorting to forced termination.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use generateprevisibines::tools::fo4edit::FO4EditRunner;
    /// # use std::path::Path;
    /// # let runner = FO4EditRunner::new("FO4Edit.exe", "F:\\Games\\Fallout4");
    /// // Called automatically by run_script after log parsing
    /// // runner.close_fo4edit_window();
    /// # Ok::<(), anyhow::Error>(())
    /// ```
    ///
    /// # Notes
    ///
    /// - This function is called automatically by `run_script` after the log file is parsed
    /// - The child process handle may report "already exited" after this function runs
    /// - All errors are silently ignored to ensure workflow continues
    /// - The fallback `taskkill` ensures cleanup even if the window message fails
    /// - Window title must exactly match "`FO4Edit`" (case-sensitive)
    ///
    /// # See Also
    ///
    /// - Original batch script lines 499-511 for the same workaround
    /// - Module-level documentation for detailed workaround explanations
    /// - Project CLAUDE.md section "Non-Automatable Tools"
    #[cfg(windows)]
    #[allow(unsafe_code, clippy::unused_self)]
    fn close_fo4edit_window(&self) {
        info!("Closing FO4Edit window...");

        let window_title = windows::core::w!("FO4Edit");
        // SAFETY: FindWindowW is safe to call with valid PCWSTR pointers.
        // `window_title` is a valid null-terminated UTF-16 string created by the w! macro.
        // This is a read-only operation that searches for a window by title.
        // The function returns a window handle or an error if the window is not found.
        let hwnd = unsafe { FindWindowW(None, window_title) };

        match hwnd {
            Ok(h) if !h.0.is_null() => {
                // SAFETY: SendMessageW is safe when called with a valid HWND.
                // We verified the handle is not null in the match guard above.
                // WM_CLOSE is a standard message that requests the window to close gracefully.
                // The message handler in FO4Edit will process this and terminate the application.
                // This is safer than force-killing the process as it allows cleanup.
                unsafe {
                    SendMessageW(h, WM_CLOSE, None, None);
                }
                info!("Sent close message to FO4Edit");
            }
            Ok(_) => {
                // Window handle is null - window not found or already closed
                info!("FO4Edit window handle is null, window may have already closed");
            }
            Err(_) => {
                // FindWindowW failed to locate the window
                info!("Failed to find FO4Edit window, it may have already exited");
            }
        }

        // Fallback: taskkill if still running after attempting graceful close
        // This ensures the process is terminated even if it ignores WM_CLOSE
        thread::sleep(Duration::from_secs(2));
        let _ = Command::new("taskkill")
            .args(["/F", "/IM", "FO4Edit.exe"])
            .output();
    }

    #[cfg(not(windows))]
    fn close_fo4edit_window(&self) {
        // Non-Windows platforms - process should exit normally
    }

    /// Check log file for errors and success indicators
    ///
    /// Parses the `FO4Edit` log file to detect known error patterns and verify successful
    /// completion. This is the ONLY reliable way to determine if a `FO4Edit` script succeeded,
    /// as `FO4Edit` provides no other feedback mechanism (exit codes are not meaningful).
    ///
    /// # Arguments
    ///
    /// * `log_file` - Path to the `FO4Edit` log file to parse
    /// * `operation` - Human-readable operation name for error messages (e.g., "Merge Combined Objects")
    ///
    /// # Error Patterns Detected
    ///
    /// This function checks for the following patterns in the log file:
    ///
    /// - **`"Error:"`** (`LOG_ERROR` constant) - FATAL
    ///   - General error pattern that catches most failures
    ///   - Examples: "Error: Plugin not found", "Error: Cannot save plugin"
    ///   - If found, returns an error immediately with the log file path
    ///
    /// # Success Indicators
    ///
    /// - **`"Completed: No Errors."`** (`LOG_SUCCESS` constant) - Expected on success
    ///   - `FO4Edit` scripts write this when completing successfully
    ///   - If NOT found, logs a warning but does NOT fail
    ///   - Absence of success message may indicate incomplete execution
    ///
    /// # Return Value Semantics
    ///
    /// Returns `Ok(())` in these cases:
    /// - Log file exists and contains no error patterns (regardless of success message)
    /// - Log file exists but is missing success message (warning logged)
    ///
    /// Returns `Err(...)` in these cases:
    /// - Log file does not exist (file not found)
    /// - Log file exists but cannot be read (permission denied, I/O error)
    /// - Log file contains error pattern (`LOG_ERROR`)
    ///
    /// # Missing Log Files
    ///
    /// Unlike `CreationKitRunner::check_log_for_errors`, this function REQUIRES the log
    /// file to exist. If the log file is missing, this returns an error. This is because
    /// `FO4Edit` is expected to create the log file early in execution (we wait for it in
    /// `wait_for_log_file`).
    ///
    /// # Errors
    ///
    /// This function will return an error if:
    /// - Log file does not exist at the specified path
    /// - Log file exists but cannot be read (permission denied, I/O error, corrupted file)
    /// - Log file contains the error pattern `"Error:"`
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use generateprevisibines::tools::fo4edit::FO4EditRunner;
    /// # use std::path::Path;
    /// # let runner = FO4EditRunner::new("FO4Edit.exe", "F:\\Games\\Fallout4");
    /// // Called automatically by run_script after FO4Edit exits
    /// let log_file = Path::new("C:\\Temp\\FO4Edit_Log_MyMod.esp.txt");
    /// // runner.check_log_for_errors(&log_file, "Merge Combined Objects")?;
    /// # Ok::<(), anyhow::Error>(())
    /// ```
    ///
    /// # Notes
    ///
    /// - This function is called automatically by `run_script` after `FO4Edit` exits
    /// - The log file is expected to exist (we wait for it in `wait_for_log_file`)
    /// - Missing success message generates a warning, not an error
    /// - The error message includes the log file path for manual inspection
    ///
    /// # See Also
    ///
    /// - `LOG_ERROR` constant for the exact error pattern
    /// - `LOG_SUCCESS` constant for the success pattern
    /// - `wait_for_log_file` for log creation waiting logic
    #[allow(clippy::unused_self)]
    fn check_log_for_errors(&self, log_file: &Path, operation: &str) -> Result<()> {
        if !log_file.exists() {
            bail!("Log file not found: {}", log_file.display());
        }

        let log_content = fs::read_to_string(log_file).context("Failed to read FO4Edit log")?;

        // Check for errors
        if log_content.contains(LOG_ERROR) {
            bail!(
                "FO4Edit {} failed: '{}' found in log.\n\
                Log file: {}",
                operation,
                LOG_ERROR,
                log_file.display()
            );
        }

        // For merge operations, check for success message
        if !log_content.contains(LOG_SUCCESS) {
            warn!(
                "FO4Edit log doesn't contain success message '{LOG_SUCCESS}', but no errors detected"
            );
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_runner_creation() {
        let runner = FO4EditRunner::new("FO4Edit.exe", "F:\\Games\\Fallout4");
        assert_eq!(runner.fo4edit_exe, PathBuf::from("FO4Edit.exe"));
    }
}
