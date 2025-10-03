use anyhow::{bail, Context, Result};
use log::{info, warn};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::BuildMode;
use crate::tools::dll_manager::{DllGuard, DllManager};

/// Errors that indicate CreationKit hit handle limits
const HANDLE_LIMIT_ERROR: &str = "OUT OF HANDLE ARRAY ENTRIES";

/// Errors that indicate previs generation failed
const PREVIS_ERROR: &str = "visibility task did not complete";

/// Runner for CreationKit.exe operations
///
/// Handles:
/// - DLL management (disable ENB/ReShade before running)
/// - Log file deletion and parsing
/// - Process execution with timeout
/// - Exit code handling (CK may exit non-zero but still succeed)
pub struct CreationKitRunner {
    ck_exe: PathBuf,
    fallout4_dir: PathBuf,
    log_file: Option<PathBuf>,
}

impl CreationKitRunner {
    /// Create a new CreationKit runner
    pub fn new(ck_exe: impl AsRef<Path>, fallout4_dir: impl AsRef<Path>) -> Self {
        Self {
            ck_exe: ck_exe.as_ref().to_path_buf(),
            fallout4_dir: fallout4_dir.as_ref().to_path_buf(),
            log_file: None,
        }
    }

    /// Set the log file path (from CKPE config)
    pub fn with_log_file(mut self, log_file: impl AsRef<Path>) -> Self {
        self.log_file = Some(log_file.as_ref().to_path_buf());
        self
    }

    /// Generate precombined meshes
    ///
    /// Runs: `CreationKit -GeneratePrecombined:<plugin> "clean/filtered all"`
    pub fn generate_precombined(
        &self,
        plugin_name: &str,
        build_mode: BuildMode,
    ) -> Result<()> {
        let mode_arg = match build_mode {
            BuildMode::Clean => "clean all",
            BuildMode::Filtered => "filtered all",
            BuildMode::Xbox => "filtered all", // Xbox uses filtered
        };

        self.run_with_dll_guard(
            &[
                &format!("-GeneratePrecombined:{}", plugin_name),
                mode_arg,
            ],
            "Generate Precombined",
        )
    }

    /// Compress PSG file (clean mode only)
    ///
    /// Runs: `CreationKit -CompressPSG:<plugin> - Geometry.csg ""`
    pub fn compress_psg(&self, plugin_name: &str) -> Result<()> {
        let plugin_base = plugin_name.trim_end_matches(".esp").trim_end_matches(".esm");
        let csg_file = format!("{} - Geometry.csg", plugin_base);

        self.run_with_dll_guard(
            &[&format!("-CompressPSG:{}", plugin_name), &csg_file, ""],
            "Compress PSG",
        )
    }

    /// Build CDX file (clean mode only)
    ///
    /// Runs: `CreationKit -BuildCDX:<plugin>.cdx ""`
    pub fn build_cdx(&self, plugin_name: &str) -> Result<()> {
        let plugin_base = plugin_name.trim_end_matches(".esp").trim_end_matches(".esm");
        let cdx_file = format!("{}.cdx", plugin_base);

        self.run_with_dll_guard(&[&format!("-BuildCDX:{}", cdx_file), ""], "Build CDX")
    }

    /// Generate previs data
    ///
    /// Runs: `CreationKit -GeneratePreVisData:<plugin> "clean all"`
    pub fn generate_previs(&self, plugin_name: &str) -> Result<()> {
        self.run_with_dll_guard(
            &[&format!("-GeneratePreVisData:{}", plugin_name), "clean all"],
            "Generate Previs",
        )?;

        // Check for specific previs failure in log
        if let Some(ref log_path) = self.log_file {
            if log_path.exists() {
                let log_content = fs::read_to_string(log_path)
                    .context("Failed to read CreationKit log")?;

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
    fn run_with_dll_guard(&self, args: &[&str], operation: &str) -> Result<()> {
        info!("Running CreationKit: {}", operation);

        // Delete old log file if it exists
        if let Some(ref log_path) = self.log_file {
            if log_path.exists() {
                fs::remove_file(log_path)
                    .with_context(|| format!("Failed to delete old log: {}", log_path.display()))?;
                info!("Deleted old CK log file");
            }
        }

        // Create DLL manager and guard
        let mut dll_manager = DllManager::new(&self.fallout4_dir);
        let _guard = DllGuard::new(&mut dll_manager)?;

        // Run CreationKit
        info!("Executing: {} {}", self.ck_exe.display(), args.join(" "));

        let status = Command::new(&self.ck_exe)
            .args(args)
            .current_dir(&self.fallout4_dir)
            .status()
            .with_context(|| format!("Failed to execute CreationKit: {}", self.ck_exe.display()))?;

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
    fn check_log_for_errors(&self) -> Result<()> {
        let Some(ref log_path) = self.log_file else {
            warn!("No log file configured, skipping error check");
            return Ok(());
        };

        if !log_path.exists() {
            warn!("Log file not created: {}", log_path.display());
            return Ok(());
        }

        let log_content = fs::read_to_string(log_path)
            .context("Failed to read CreationKit log")?;

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

        assert_eq!(
            runner.log_file,
            Some(PathBuf::from("CreationKit.log"))
        );
    }
}
