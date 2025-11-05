//! Creation Kit automation with required workarounds
//!
//! This module provides utilities for running the Fallout 4 Creation Kit (CK) in automated
//! workflows. The Creation Kit is notoriously difficult to automate due to several quirks
//! and limitations that require workarounds.
//!
//! # Overview
//!
//! The main interface is [`CreationKitRunner`], which handles:
//! - Precombined mesh generation (`-GeneratePrecombined`)
//! - PSG compression (`-CompressPSG`)
//! - CDX building (`-BuildCDX`)
//! - Previs data generation (`-GeneratePreVisData`)
//!
//! # Critical Workarounds (DO NOT REMOVE)
//!
//! ## Workaround #1: DLL Management
//!
//! **Problem:** CreationKit crashes when ENB or ReShade DLLs are loaded.
//!
//! **Solution:** The [`DllGuard`] automatically renames interfering DLLs before CK runs
//! and restores them afterward. Files renamed:
//! - `d3d11.dll` → `d3d11.dll-PJMdisabled`
//! - `dxgi.dll` → `dxgi.dll-PJMdisabled`
//! - `enbimgui.dll` → `enbimgui.dll-PJMdisabled`
//! - And several others (see `dll_manager` module)
//!
//! **This is NOT optional - CK WILL CRASH without this.**
//!
//! See original batch script lines 422-427, 330-335.
//!
//! ## Workaround #2: Log Parsing Instead of Exit Codes
//!
//! **Problem:** CreationKit exit codes are unreliable and cannot be used to determine
//! success or failure. CK often exits with non-zero codes after successful operations,
//! and sometimes exits with code 0 even when operations failed.
//!
//! **Solution:** Every CK operation:
//! 1. Deletes the old log file before starting
//! 2. Runs CreationKit
//! 3. Parses the new log file for known critical error patterns
//! 4. Ignores exit codes entirely (logs them as warnings only)
//!
//! **This is NOT inefficient code - it's the ONLY way to detect CK errors.**
//!
//! See [`check_log_for_errors`](CreationKitRunner::check_log_for_errors) for details.
//!
//! # Error Detection
//!
//! Two critical error patterns are detected:
//!
//! - [`HANDLE_LIMIT_ERROR`]: CK ran out of object handles (mod too complex)
//! - [`PREVIS_ERROR`]: Previs generation failed for some cells
//!
//! # Mod Organizer 2 Support
//!
//! When configured with [`with_mo2`](CreationKitRunner::with_mo2), CreationKit is launched
//! through MO2's virtual file system. This ensures CK sees mods in the correct load order.
//!
//! # Examples
//!
//! ```no_run
//! use std::path::Path;
//! use generateprevisibines::tools::creation_kit::CreationKitRunner;
//! use generateprevisibines::config::BuildMode;
//!
//! let ck_exe = Path::new("C:\\Games\\Fallout4\\CreationKit.exe");
//! let fo4_dir = Path::new("C:\\Games\\Fallout4");
//! let log_file = Path::new("C:\\Users\\user\\AppData\\Local\\Fallout4\\CreationKit.log");
//!
//! let runner = CreationKitRunner::new(ck_exe, fo4_dir)
//!     .with_log_file(log_file);
//!
//! // Step 2: Generate precombined meshes
//! runner.generate_precombined("MyMod.esp", BuildMode::Clean)?;
//!
//! // Step 7: Generate previs data
//! runner.generate_previs("MyMod.esp")?;
//! # Ok::<(), anyhow::Error>(())
//! ```
//!
//! # See Also
//!
//! - `dll_manager` module for DLL renaming implementation
//! - Original batch script (`GeneratePrevisibines.bat`) for historical context
//! - Project CLAUDE.md for detailed workaround documentation

use anyhow::{Context, Result, bail};
use log::{info, warn};
use mo2_mode::MO2Command;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::BuildMode;
use crate::tools::dll_manager::{DllGuard, DllManager};

/// Critical error pattern: CreationKit handle limit exceeded
///
/// **Error String:** `"OUT OF HANDLE ARRAY ENTRIES"`
///
/// This error appears in CreationKit's log when it runs out of internal object handles.
/// CreationKit uses a fixed-size array to track game objects during processing. When
/// a mod contains too many objects (references, forms, cells), this array overflows.
///
/// # When This Occurs
///
/// - Processing worldspaces with thousands of objects
/// - Large mods with complex cell edits
/// - Mods combining many master files
///
/// # Impact
///
/// This is a **FATAL ERROR** that cannot be worked around. The operation will fail,
/// and the workflow cannot continue.
///
/// # Solutions
///
/// 1. Split the mod into smaller plugins
/// 2. Reduce object count in cells
/// 3. Simplify complex areas
/// 4. Use filtered mode instead of clean mode (reduces object count)
///
/// # Detection
///
/// Checked in `check_log_for_errors()` after every CreationKit operation.
const HANDLE_LIMIT_ERROR: &str = "OUT OF HANDLE ARRAY ENTRIES";

/// Critical error pattern: Previs generation task failed
///
/// **Error String:** `"visibility task did not complete"`
///
/// This error appears when CreationKit's previs (pre-calculated visibility) generation
/// fails for one or more cells. Unlike handle limit errors, this may affect only some
/// cells while others succeed.
///
/// # When This Occurs
///
/// - Cells with malformed geometry
/// - Complex geometry that exceeds previs calculation limits
/// - Internal CreationKit processing errors
/// - Corrupted mesh files
///
/// # Impact
///
/// This is a **FATAL ERROR** for workflow purposes. Even if some cells generated previs
/// successfully, the workflow should not continue with incomplete previs data.
///
/// # Solutions
///
/// 1. Identify problematic cells from the log
/// 2. Simplify geometry in affected cells
/// 3. Fix or remove malformed meshes
/// 4. Check for NIF file corruption
///
/// # Detection
///
/// Checked ONLY in `generate_previs()` after previs generation completes. This error
/// is specific to previs operations and not checked during other CK operations.
const PREVIS_ERROR: &str = "visibility task did not complete";

/// Runner for CreationKit.exe operations
///
/// Provides a safe interface for running the Fallout 4 Creation Kit (CK) in automated
/// workflows. The Creation Kit is notoriously difficult to automate due to several
/// quirks and limitations that require workarounds.
///
/// # Key Features
///
/// - **DLL Guard**: Automatically disables ENB/ReShade DLLs that crash CK
/// - **Log Parsing**: Detects errors from log files since CK exit codes are unreliable
/// - **MO2 Support**: Can launch CK through Mod Organizer 2's virtual file system
/// - **Error Detection**: Identifies critical errors (handle limits, previs failures)
///
/// # Important Workarounds (DO NOT REMOVE)
///
/// **REQUIRED WORKAROUND #1: DLL Management**
///
/// CreationKit crashes when ENB or ReShade DLLs (d3d11.dll, dxgi.dll, etc.) are loaded.
/// The `DllGuard` automatically renames these files before CK runs and restores them afterward.
/// See original batch script lines 422-427, 330-335.
///
/// **This is NOT optional optimization - CK WILL CRASH without this.**
///
/// **REQUIRED WORKAROUND #2: Log Parsing Instead of Exit Codes**
///
/// CreationKit frequently exits with non-zero status codes even on successful operations.
/// Exit codes alone are UNRELIABLE for determining success/failure. Instead, this runner:
/// - Deletes old log files before each operation
/// - Parses the new log file for known critical error patterns
/// - Ignores non-zero exit codes if the log shows success
///
/// **This is NOT code smell - exit codes cannot be trusted.**
///
/// # Critical Errors Detected
///
/// - **`OUT OF HANDLE ARRAY ENTRIES`**: CK ran out of internal object handles (mod too complex)
/// - **`visibility task did not complete`**: Previs generation failed for some cells
///
/// # Examples
///
/// ```no_run
/// use std::path::Path;
/// use generateprevisibines::tools::creation_kit::CreationKitRunner;
/// use generateprevisibines::config::BuildMode;
///
/// let ck_exe = Path::new("C:\\Games\\Fallout4\\CreationKit.exe");
/// let fo4_dir = Path::new("C:\\Games\\Fallout4");
/// let log_file = Path::new("C:\\Users\\user\\AppData\\Local\\Fallout4\\CreationKit.log");
///
/// let runner = CreationKitRunner::new(ck_exe, fo4_dir)
///     .with_log_file(log_file);
///
/// // Generate precombined meshes
/// runner.generate_precombined("MyMod.esp", BuildMode::Clean)?;
///
/// // Generate previs data
/// runner.generate_previs("MyMod.esp")?;
/// # Ok::<(), anyhow::Error>(())
/// ```
///
/// # See Also
///
/// - `DllManager` and `DllGuard` for DLL management implementation
/// - Original batch script documentation for historical context
pub struct CreationKitRunner {
    ck_exe: PathBuf,
    fallout4_dir: PathBuf,
    log_file: Option<PathBuf>,
    mo2_path: Option<PathBuf>,
}

impl CreationKitRunner {
    /// Create a new CreationKit runner
    pub fn new(ck_exe: impl AsRef<Path>, fallout4_dir: impl AsRef<Path>) -> Self {
        Self {
            ck_exe: ck_exe.as_ref().to_path_buf(),
            fallout4_dir: fallout4_dir.as_ref().to_path_buf(),
            log_file: None,
            mo2_path: None,
        }
    }

    /// Set the log file path (from CKPE config)
    pub fn with_log_file(mut self, log_file: impl AsRef<Path>) -> Self {
        self.log_file = Some(log_file.as_ref().to_path_buf());
        self
    }

    /// Set Mod Organizer 2 path for VFS execution
    pub fn with_mo2(mut self, mo2_path: impl AsRef<Path>) -> Self {
        self.mo2_path = Some(mo2_path.as_ref().to_path_buf());
        self
    }

    /// Generate precombined meshes
    ///
    /// Executes CreationKit with the `-GeneratePrecombined` command to create optimized
    /// combined meshes for improved game performance. This is Step 2 of the workflow.
    ///
    /// # Arguments
    ///
    /// * `plugin_name` - Name of the plugin file (e.g., "MyMod.esp")
    /// * `build_mode` - Build mode determining the generation strategy:
    ///   - `BuildMode::Clean`: Uses "clean all" (generates for all cells, no filtering)
    ///   - `BuildMode::Filtered`: Uses "filtered all" (filters based on usage)
    ///   - `BuildMode::Xbox`: Uses "filtered all" (Xbox uses filtered mode)
    ///
    /// # Command Executed
    ///
    /// ```text
    /// CreationKit.exe -GeneratePrecombined:MyMod.esp "clean all"
    /// ```
    /// or
    /// ```text
    /// CreationKit.exe -GeneratePrecombined:MyMod.esp "filtered all"
    /// ```
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` if precombined mesh generation completes successfully.
    /// Success is determined by log file analysis, NOT exit codes.
    ///
    /// # Errors
    ///
    /// This function will return an error if:
    /// - CreationKit cannot be launched (file not found, permission denied)
    /// - DLL renaming fails (files in use, permission denied)
    /// - Log file indicates critical errors (handle limit exceeded)
    /// - Old log file cannot be deleted before starting
    /// - MO2 execution fails (if using Mod Organizer 2 mode)
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use generateprevisibines::config::BuildMode;
    /// # use generateprevisibines::tools::creation_kit::CreationKitRunner;
    /// # use std::path::Path;
    /// # let runner = CreationKitRunner::new("ck.exe", "fo4");
    ///
    /// // Clean mode - generate for all cells
    /// runner.generate_precombined("MyMod.esp", BuildMode::Clean)?;
    ///
    /// // Filtered mode - only generate for used cells
    /// runner.generate_precombined("MyMod.esp", BuildMode::Filtered)?;
    /// # Ok::<(), anyhow::Error>(())
    /// ```
    ///
    /// # Notes
    ///
    /// - Exit codes are ignored; only log file errors are considered fatal
    /// - DLL guard is automatically applied (ENB/ReShade DLLs disabled during execution)
    /// - Generated files are placed in `Data/meshes/precombined/`
    pub fn generate_precombined(&self, plugin_name: &str, build_mode: BuildMode) -> Result<()> {
        let (arg1, arg2) = match build_mode {
            BuildMode::Clean => ("clean", "all"),
            BuildMode::Filtered => ("filtered", "all"),
            BuildMode::Xbox => ("filtered", "all"), // Xbox uses filtered
        };

        self.run_with_dll_guard(
            &[&format!("-GeneratePrecombined:{}", plugin_name), arg1, arg2],
            "Generate Precombined",
        )
    }

    /// Compress PSG file (clean mode only)
    ///
    /// Runs: `CreationKit -CompressPSG:"<plugin>"`
    ///
    /// Note: The output file name (e.g., "MyMod - Geometry.csg") is NOT part of the command.
    /// CK determines the output filename automatically from the plugin name.
    pub fn compress_psg(&self, plugin_name: &str) -> Result<()> {
        self.run_with_dll_guard(
            &[&format!("-CompressPSG:{}", plugin_name)],
            "Compress PSG",
        )
    }

    /// Build CDX file (clean mode only)
    ///
    /// Runs: `CreationKit -BuildCDX:"<plugin>"`
    ///
    /// Note: The command takes the PLUGIN name, not the CDX filename.
    /// CK automatically creates the .cdx file from the plugin name.
    pub fn build_cdx(&self, plugin_name: &str) -> Result<()> {
        self.run_with_dll_guard(
            &[&format!("-BuildCDX:{}", plugin_name)],
            "Build CDX"
        )
    }

    /// Generate previs data
    ///
    /// Executes CreationKit with the `-GeneratePreVisData` command to create previs
    /// (pre-calculated visibility) data for improved game performance. This is Step 7
    /// of the workflow.
    ///
    /// # Arguments
    ///
    /// * `plugin_name` - Name of the plugin file (e.g., "MyMod.esp")
    ///
    /// # Command Executed
    ///
    /// ```text
    /// CreationKit.exe -GeneratePreVisData:MyMod.esp "clean all"
    /// ```
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` if previs data generation completes successfully without critical errors.
    ///
    /// # Errors
    ///
    /// This function will return an error if:
    /// - CreationKit cannot be launched (file not found, permission denied)
    /// - DLL renaming fails (files in use, permission denied)
    /// - Log file indicates critical errors:
    ///   - **`OUT OF HANDLE ARRAY ENTRIES`**: Handle limit exceeded
    ///   - **`visibility task did not complete`**: Previs generation failed for some cells
    /// - Old log file cannot be deleted before starting
    /// - Log file cannot be read after execution
    /// - MO2 execution fails (if using Mod Organizer 2 mode)
    ///
    /// # Special Error Detection
    ///
    /// This function performs ADDITIONAL log checking beyond the standard error detection.
    /// After running CreationKit, it specifically searches for the error pattern
    /// `"visibility task did not complete"` which indicates previs generation failures.
    ///
    /// This error typically occurs when:
    /// - Cells are too complex for previs calculation
    /// - Geometry is malformed or has issues
    /// - CreationKit encounters internal processing errors
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use generateprevisibines::tools::creation_kit::CreationKitRunner;
    /// # use std::path::Path;
    /// # let runner = CreationKitRunner::new("ck.exe", "fo4");
    ///
    /// // Generate previs data
    /// runner.generate_previs("MyMod.esp")?;
    /// # Ok::<(), anyhow::Error>(())
    /// ```
    ///
    /// # Notes
    ///
    /// - Always uses "clean all" mode (no filtered option for previs)
    /// - Exit codes are ignored; only log file errors are considered fatal
    /// - DLL guard is automatically applied (ENB/ReShade DLLs disabled during execution)
    /// - Generated files are placed in `Data/vis/`
    pub fn generate_previs(&self, plugin_name: &str) -> Result<()> {
        self.run_with_dll_guard(
            &[&format!("-GeneratePreVisData:{}", plugin_name), "clean", "all"],
            "Generate Previs",
        )?;

        // Check for specific previs failure in log
        if let Some(ref log_path) = self.log_file {
            if log_path.exists() {
                let log_content =
                    fs::read_to_string(log_path).context("Failed to read CreationKit log")?;

                if log_content.contains(PREVIS_ERROR) {
                    bail!(
                        "Previs generation failed: '{}' found in log.\n\
                        This usually indicates cells that couldn't generate previs data.",
                        PREVIS_ERROR
                    );
                }
            }
        }

        Ok(())
    }

    /// Run CreationKit with DLL guard and log management
    ///
    /// Internal wrapper that handles all the necessary workarounds for running CreationKit
    /// safely in an automated environment. This function coordinates DLL management, log
    /// file handling, process execution, and error detection.
    ///
    /// # Arguments
    ///
    /// * `args` - Command-line arguments to pass to CreationKit.exe
    /// * `operation` - Human-readable operation name for logging (e.g., "Generate Precombined")
    ///
    /// # Process Flow
    ///
    /// 1. **Log Cleanup**: Deletes old log file (if exists) to ensure fresh error detection
    /// 2. **DLL Guard**: Creates `DllGuard` to temporarily rename ENB/ReShade DLLs
    /// 3. **Execution**: Runs CreationKit (optionally through MO2)
    /// 4. **Error Detection**: Parses log file for critical errors
    /// 5. **Exit Code Handling**: Logs exit code but does NOT fail on non-zero codes
    /// 6. **DLL Restoration**: `DllGuard` automatically restores DLLs when dropped
    ///
    /// # Why Exit Codes Can't Be Trusted
    ///
    /// **CRITICAL INFORMATION:**
    ///
    /// CreationKit frequently exits with non-zero status codes even after successful operations.
    /// This is not a bug in this code - it's a known limitation of CreationKit itself.
    ///
    /// **Example scenarios where CK exits non-zero on success:**
    /// - Precombined generation completes but CK exits with code 1
    /// - Previs generation succeeds but CK exits with code 3221225477 (0xC0000005)
    /// - CDX building finishes correctly but CK exits with code 2
    ///
    /// **This function relies ENTIRELY on log file parsing for error detection.**
    /// Non-zero exit codes generate warnings but do NOT cause failures.
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` if:
    /// - CreationKit executes (regardless of exit code)
    /// - Log file shows no critical errors
    ///
    /// # Errors
    ///
    /// This function will return an error if:
    /// - Old log file exists but cannot be deleted (permission denied, file in use)
    /// - DLL guard fails to rename DLLs (files in use, permission denied, read-only)
    /// - CreationKit.exe cannot be launched (file not found, not executable)
    /// - MO2 execution fails (if using MO2 mode)
    /// - Log file parsing detects critical errors (`check_log_for_errors`)
    ///
    /// # Notes
    ///
    /// - This is a private function; use the public wrappers (`generate_precombined`, etc.)
    /// - DLL restoration happens automatically via RAII (DllGuard drop)
    /// - If log file is not configured, error checking is skipped (warning logged)
    /// - MO2 mode is automatically used if `mo2_path` is set
    fn run_with_dll_guard(&self, args: &[&str], operation: &str) -> Result<()> {
        info!("Running CreationKit: {}", operation);

        // Delete old log file if it exists
        // NOTE: This operation may fail if the log is open in another process or
        // another instance is running. We treat this as a hard error to prevent
        // mixing logs from multiple runs.
        if let Some(ref log_path) = self.log_file {
            if log_path.exists() {
                fs::remove_file(log_path).with_context(|| {
                    format!(
                        "Failed to delete old log: {}\n\
                        \n\
                        The file may be locked by another process. Common causes:\n\
                        - Log file is open in a text editor or log viewer\n\
                        - Another instance of CreationKit is running\n\
                        - Antivirus software is scanning the file\n\
                        \n\
                        Please close any programs viewing the log and try again.",
                        log_path.display()
                    )
                })?;
                info!("Deleted old CK log file");
            }
        }

        // Create DLL manager and guard
        let mut dll_manager = DllManager::new(&self.fallout4_dir);
        let _guard = DllGuard::new(&mut dll_manager)?;

        // Run CreationKit (optionally through MO2)
        info!("Executing: {} {}", self.ck_exe.display(), args.join(" "));

        let status = if let Some(ref mo2_path) = self.mo2_path {
            // Use MO2 mode
            info!("Launching through Mod Organizer 2: {}", mo2_path.display());
            let mut cmd = MO2Command::new(mo2_path, &self.ck_exe)
                .args(args.iter().copied())
                .execute();
            cmd.current_dir(&self.fallout4_dir)
                .status()
                .with_context(|| {
                    format!(
                        "Failed to execute CreationKit through MO2: {}",
                        mo2_path.display()
                    )
                })?
        } else {
            // Direct execution
            Command::new(&self.ck_exe)
                .args(args)
                .current_dir(&self.fallout4_dir)
                .status()
                .with_context(|| {
                    format!("Failed to execute CreationKit: {}", self.ck_exe.display())
                })?
        };

        // Parse log for errors (even if exit code is non-zero)
        self.check_log_for_errors()?;

        // CreationKit may exit with non-zero but still succeed
        // We rely on log parsing for actual error detection
        if !status.success() {
            warn!(
                "CreationKit exited with code: {:?} (may be normal)",
                status.code()
            );
        }

        info!("CreationKit {} completed", operation);
        Ok(())
    }

    /// Check log file for critical errors
    ///
    /// Parses the CreationKit log file to detect known critical error patterns that indicate
    /// workflow failure. This is **REQUIRED** because CreationKit's exit codes are unreliable
    /// and cannot be used to determine success/failure.
    ///
    /// # Why Log Parsing is Required (Not Code Smell)
    ///
    /// **CRITICAL INFORMATION:**
    ///
    /// CreationKit frequently exits with non-zero status codes even after successful operations.
    /// Conversely, it sometimes exits with code 0 even when operations failed. The ONLY reliable
    /// way to detect failures is by parsing the log file for known error patterns.
    ///
    /// **This is NOT inefficient code - it's the ONLY way to detect CK errors.**
    ///
    /// # Critical Errors Detected
    ///
    /// Currently detects the following fatal errors:
    ///
    /// - **`OUT OF HANDLE ARRAY ENTRIES`** (`HANDLE_LIMIT_ERROR` constant)
    ///   - Indicates CreationKit ran out of internal object handles
    ///   - Means the mod is too complex for CK's internal data structures
    ///   - **Solution**: Split the mod into smaller pieces or reduce object count
    ///   - This error is ALWAYS fatal and cannot be worked around
    ///
    /// # Return Value Semantics
    ///
    /// Returns `Ok(())` in these cases:
    /// - No log file is configured (`self.log_file` is `None`) - logs warning
    /// - Log file doesn't exist after CK runs - logs warning (may indicate CK crashed immediately)
    /// - Log file exists and contains no critical error patterns - success
    ///
    /// Returns `Err(...)` in these cases:
    /// - Log file exists but cannot be read (permission denied, I/O error)
    /// - Critical error pattern is found in the log content
    ///
    /// # Missing Log Files vs. Critical Errors
    ///
    /// **Important distinction:**
    ///
    /// - **Missing log file**: Returns `Ok(())` with a warning
    ///   - CK may have crashed before creating the log
    ///   - Or log path may be misconfigured
    ///   - Logged as warning, not error (user should investigate)
    ///
    /// - **Log file with critical errors**: Returns `Err(...)`
    ///   - CK ran but encountered fatal errors
    ///   - Clear error message with remediation steps
    ///   - Workflow cannot continue
    ///
    /// # Errors
    ///
    /// This function will return an error if:
    /// - Log file exists but cannot be read (permission denied, I/O error, corrupted file)
    /// - Log contains `HANDLE_LIMIT_ERROR` pattern
    ///
    /// # Notes
    ///
    /// - Called automatically after every CreationKit operation
    /// - `generate_previs` performs ADDITIONAL checks for `PREVIS_ERROR` pattern
    /// - Log file is deleted before each CK run to ensure fresh error detection
    /// - Future enhancements may add detection for additional error patterns
    ///
    /// # See Also
    ///
    /// - `HANDLE_LIMIT_ERROR` constant for the exact error string
    /// - `PREVIS_ERROR` constant for previs-specific errors (checked in `generate_previs`)
    fn check_log_for_errors(&self) -> Result<()> {
        let Some(ref log_path) = self.log_file else {
            warn!("No log file configured, skipping error check");
            return Ok(());
        };

        if !log_path.exists() {
            warn!("Log file not created: {}", log_path.display());
            return Ok(());
        }

        let log_content = fs::read_to_string(log_path).context("Failed to read CreationKit log")?;

        // Check for handle limit errors
        if log_content.contains(HANDLE_LIMIT_ERROR) {
            bail!(
                "CreationKit hit handle limit: '{}' found in log.\n\
                This indicates too many objects for CK to process.\n\
                You may need to split your mod or reduce complexity.",
                HANDLE_LIMIT_ERROR
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
        let runner = CreationKitRunner::new("CreationKit.exe", "F:\\Games\\Fallout4");
        assert_eq!(runner.ck_exe, PathBuf::from("CreationKit.exe"));
    }

    #[test]
    fn test_with_log_file() {
        let runner = CreationKitRunner::new("CreationKit.exe", "F:\\Games\\Fallout4")
            .with_log_file("CreationKit.log");

        assert_eq!(runner.log_file, Some(PathBuf::from("CreationKit.log")));
    }
}
