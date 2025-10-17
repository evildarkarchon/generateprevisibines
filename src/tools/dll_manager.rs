//! DLL management for CreationKit compatibility
//!
//! This module provides utilities for managing ENB and ReShade DLLs that interfere
//! with CreationKit execution. CreationKit is incompatible with certain graphics
//! enhancement DLLs and will crash if they are loaded.
//!
//! # The DLL Problem
//!
//! **IMPORTANT: This is NOT code smell or over-engineering. This is a REQUIRED workaround.**
//!
//! CreationKit crashes when the following graphics enhancement DLLs are loaded:
//! - ENB (Enhanced Natural Beauty) DLLs: `d3d11.dll`, `d3d10.dll`, `d3d9.dll`, `dxgi.dll`, `enbimgui.dll`
//! - ReShade DLLs: `d3dcompiler_46e.dll`
//!
//! These DLLs hook DirectX functions to enhance game graphics, but CK's rendering
//! pipeline is incompatible with these hooks and will crash on launch.
//!
//! # The Solution: Temporary Renaming
//!
//! The workaround is to temporarily rename these DLLs by appending `-PJMdisabled` to
//! their extensions, making them invisible to the Windows DLL loader. After CK exits,
//! the DLLs are renamed back to their original names.
//!
//! **Example:**
//! - Before: `d3d11.dll` → While running CK: `d3d11.dll-PJMdisabled` → After: `d3d11.dll`
//!
//! # RAII Guard Pattern
//!
//! This module uses the RAII (Resource Acquisition Is Initialization) pattern via
//! [`DllGuard`] to ensure DLLs are always restored, even if CK crashes or panics occur:
//!
//! ```no_run
//! # use anyhow::Result;
//! # fn run_ck() -> Result<()> { Ok(()) }
//! # use std::path::Path;
//! # let fallout4_dir = Path::new(".");
//! use generateprevisibines::tools::dll_manager::DllManager;
//!
//! let mut manager = DllManager::new(fallout4_dir);
//! {
//!     let _guard = manager.disable_dlls()?; // DLLs disabled here
//!     run_ck()?; // CreationKit runs safely
//! } // Guard drops here - DLLs automatically restored
//! # Ok::<(), anyhow::Error>(())
//! ```
//!
//! # References
//!
//! This implementation replicates the batch script workaround from lines 422-427 and 330-335.
//! See CLAUDE.md for project context.

use anyhow::{Context, Result};
use log::{info, warn};
use std::fs;
use std::path::{Path, PathBuf};

/// DLL files that interfere with CreationKit (ENB/ReShade)
/// These must be renamed before running CK to prevent crashes
/// Matches batch script lines 422-427, 330-335
const INTERFERING_DLLS: &[&str] = &[
    "d3d11.dll",
    "d3d10.dll",
    "d3d9.dll",
    "dxgi.dll",
    "enbimgui.dll",
    "d3dcompiler_46e.dll",
];

/// Suffix used to disable DLLs (matches batch script)
const DISABLED_SUFFIX: &str = "-PJMdisabled";

/// Manages ENB/ReShade DLL disable/restore operations
///
/// **IMPORTANT: This is NOT code smell to be refactored away.**
/// It's a necessary workaround for CreationKit's incompatibility with ENB/ReShade DLLs.
///
/// CreationKit crashes when certain graphics enhancement DLLs are loaded. This manager
/// temporarily renames them to disable, then restores them after CK exits.
///
/// # Usage Pattern
///
/// ```no_run
/// # use std::path::Path;
/// use generateprevisibines::tools::dll_manager::DllManager;
///
/// let mut manager = DllManager::new("C:\\Games\\Fallout4");
///
/// // Disable DLLs before running CK
/// let disabled_count = manager.disable_dlls()?;
/// println!("Disabled {} DLLs", disabled_count);
///
/// // Run CreationKit (safe from ENB/ReShade crashes)
/// // ... run_creation_kit() ...
///
/// // Restore DLLs after CK exits
/// let restored_count = manager.restore_dlls()?;
/// println!("Restored {} DLLs", restored_count);
/// # Ok::<(), anyhow::Error>(())
/// ```
///
/// # Recommended Pattern: RAII Guard
///
/// For automatic cleanup, use [`DllGuard`]:
///
/// ```no_run
/// # use std::path::Path;
/// # use anyhow::Result;
/// use generateprevisibines::tools::dll_manager::{DllManager, DllGuard};
///
/// # fn run_creation_kit() -> Result<()> { Ok(()) }
/// let mut manager = DllManager::new("C:\\Games\\Fallout4");
/// {
///     let _guard = DllGuard::new(&mut manager)?;
///     run_creation_kit()?;
/// } // DLLs automatically restored when guard drops
/// # Ok::<(), anyhow::Error>(())
/// ```
pub struct DllManager {
    fallout4_dir: PathBuf,
    disabled_dlls: Vec<PathBuf>,
}

impl DllManager {
    /// Create a new DLL manager for the given Fallout 4 directory
    ///
    /// # Arguments
    ///
    /// * `fallout4_dir` - Path to the Fallout 4 installation directory (e.g., `C:\Games\Fallout4`)
    pub fn new(fallout4_dir: impl AsRef<Path>) -> Self {
        Self {
            fallout4_dir: fallout4_dir.as_ref().to_path_buf(),
            disabled_dlls: Vec::new(),
        }
    }

    /// Scan for interfering DLLs in the Fallout 4 directory
    pub fn scan(&self) -> Vec<PathBuf> {
        let mut found = Vec::new();

        for dll_name in INTERFERING_DLLS {
            let dll_path = self.fallout4_dir.join(dll_name);
            if dll_path.exists() {
                found.push(dll_path);
            }
        }

        found
    }

    /// Disable all interfering DLLs by renaming them
    ///
    /// **REQUIRED WORKAROUND:** CreationKit crashes when ENB or ReShade DLLs are loaded.
    /// This function temporarily disables them by appending `-PJMdisabled` to their file extensions.
    ///
    /// # Disabled DLLs
    ///
    /// Renames the following files if found in the Fallout 4 directory:
    /// - `d3d11.dll` → `d3d11.dll-PJMdisabled`
    /// - `d3d10.dll` → `d3d10.dll-PJMdisabled`
    /// - `d3d9.dll` → `d3d9.dll-PJMdisabled`
    /// - `dxgi.dll` → `dxgi.dll-PJMdisabled`
    /// - `enbimgui.dll` → `enbimgui.dll-PJMdisabled`
    /// - `d3dcompiler_46e.dll` → `d3dcompiler_46e.dll-PJMdisabled`
    ///
    /// # Returns
    ///
    /// Returns the number of DLLs successfully disabled (0 if none found)
    ///
    /// # Errors
    ///
    /// This function will return an error if:
    /// - Any DLL file exists but cannot be renamed (file in use, permission denied, read-only)
    ///
    /// If an error occurs, some DLLs may have been renamed before the failure.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use std::path::Path;
    /// use generateprevisibines::tools::dll_manager::DllManager;
    ///
    /// let mut dll_manager = DllManager::new("C:\\Games\\Fallout4");
    /// let count = dll_manager.disable_dlls()?;
    /// println!("Disabled {} DLL(s)", count);
    /// # Ok::<(), anyhow::Error>(())
    /// ```
    ///
    /// # Notes
    ///
    /// - Disabled DLLs are tracked internally for later restoration
    /// - Call `restore_dlls()` after CreationKit exits to re-enable graphics enhancements
    /// - The `-PJMdisabled` suffix matches the original batch script convention (lines 422-427, 330-335)
    pub fn disable_dlls(&mut self) -> Result<usize> {
        let dlls_to_disable = self.scan();

        if dlls_to_disable.is_empty() {
            info!("No interfering DLLs found");
            return Ok(0);
        }

        let mut disabled_count = 0;

        for dll_path in dlls_to_disable {
            let disabled_path = dll_path.with_extension(
                dll_path
                    .extension()
                    .unwrap_or_default()
                    .to_str()
                    .unwrap_or("")
                    .to_string()
                    + DISABLED_SUFFIX,
            );

            fs::rename(&dll_path, &disabled_path).with_context(|| {
                format!(
                    "Failed to disable DLL: {} -> {}",
                    dll_path.display(),
                    disabled_path.display()
                )
            })?;

            info!(
                "Disabled DLL: {}",
                dll_path.file_name().unwrap().to_string_lossy()
            );
            self.disabled_dlls.push(disabled_path);
            disabled_count += 1;
        }

        Ok(disabled_count)
    }

    /// Restore all previously disabled DLLs
    ///
    /// Renames all previously disabled DLLs back to their original names, re-enabling
    /// ENB and ReShade graphics enhancements. This is called after CreationKit exits.
    ///
    /// # Restored DLLs
    ///
    /// Reverses the renaming performed by `disable_dlls()`:
    /// - `d3d11.dll-PJMdisabled` → `d3d11.dll`
    /// - `d3d10.dll-PJMdisabled` → `d3d10.dll`
    /// - `d3d9.dll-PJMdisabled` → `d3d9.dll`
    /// - `dxgi.dll-PJMdisabled` → `dxgi.dll`
    /// - `enbimgui.dll-PJMdisabled` → `enbimgui.dll`
    /// - `d3dcompiler_46e.dll-PJMdisabled` → `d3dcompiler_46e.dll`
    ///
    /// # Returns
    ///
    /// Returns the number of DLLs successfully restored (0 if none were disabled)
    ///
    /// # Errors
    ///
    /// This function will return an error if:
    /// - Any disabled DLL file cannot be renamed back (file in use, permission denied, read-only)
    ///
    /// If an error occurs, some DLLs may have been restored before the failure.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use std::path::Path;
    /// use generateprevisibines::tools::dll_manager::DllManager;
    ///
    /// let mut dll_manager = DllManager::new("C:\\Games\\Fallout4");
    /// dll_manager.disable_dlls()?;
    /// // ... run CreationKit ...
    /// let count = dll_manager.restore_dlls()?;
    /// println!("Restored {} DLL(s)", count);
    /// # Ok::<(), anyhow::Error>(())
    /// ```
    ///
    /// # Notes
    ///
    /// - Clears the internal list of disabled DLLs after restoration
    /// - Logs a warning (not an error) if a disabled DLL file is missing (may have been deleted manually)
    /// - Safe to call multiple times (returns 0 after first call)
    /// - The `-PJMdisabled` suffix is removed to restore original functionality (batch script lines 422-427, 330-335)
    pub fn restore_dlls(&mut self) -> Result<usize> {
        if self.disabled_dlls.is_empty() {
            return Ok(0);
        }

        let mut restored_count = 0;

        for disabled_path in &self.disabled_dlls {
            // Remove the -PJMdisabled suffix
            let original_name = disabled_path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|s| s.replace(DISABLED_SUFFIX, ""))
                .context("Invalid DLL filename")?;

            let original_path = disabled_path.with_file_name(original_name);

            if disabled_path.exists() {
                fs::rename(disabled_path, &original_path).with_context(|| {
                    format!(
                        "Failed to restore DLL: {} -> {}",
                        disabled_path.display(),
                        original_path.display()
                    )
                })?;

                info!(
                    "Restored DLL: {}",
                    original_path.file_name().unwrap().to_string_lossy()
                );
                restored_count += 1;
            } else {
                warn!(
                    "Disabled DLL not found, skipping: {}",
                    disabled_path.display()
                );
            }
        }

        self.disabled_dlls.clear();
        Ok(restored_count)
    }
}

/// RAII guard that ensures DLLs are restored when dropped
///
/// This guard implements the RAII (Resource Acquisition Is Initialization) pattern
/// to guarantee that disabled DLLs are always restored, even if CreationKit crashes
/// or a panic occurs during execution.
///
/// # How It Works
///
/// 1. **Acquire:** Creating the guard disables all interfering DLLs
/// 2. **Use:** CreationKit runs safely with DLLs disabled
/// 3. **Release:** When the guard goes out of scope (drops), DLLs are automatically restored
///
/// The restoration happens in the `Drop` implementation, which is called even during
/// unwinding from panics or early returns.
///
/// # Examples
///
/// ```no_run
/// # use std::path::Path;
/// # use anyhow::Result;
/// use generateprevisibines::tools::dll_manager::DllManager;
///
/// # fn run_creation_kit() -> Result<()> { Ok(()) }
/// let mut manager = DllManager::new("C:\\Games\\Fallout4");
/// {
///     let _guard = DllGuard::new(&mut manager)?;
///     // DLLs are now disabled - CreationKit can run safely
///     run_creation_kit()?;
/// } // Guard drops here - DLLs automatically restored
/// # Ok::<(), anyhow::Error>(())
/// ```
///
/// ## Even On Panic
///
/// ```no_run
/// # use std::path::Path;
/// use generateprevisibines::tools::dll_manager::DllManager;
///
/// let mut manager = DllManager::new("C:\\Games\\Fallout4");
/// {
///     let _guard = DllGuard::new(&mut manager)?;
///     // Even if this panics...
///     panic!("Something went wrong!");
/// } // ...DLLs are still restored during unwinding
/// # Ok::<(), anyhow::Error>(())
/// ```
///
/// # Notes
///
/// - The guard takes a mutable reference to `DllManager`, preventing other access while active
/// - Restoration errors are logged as warnings, not propagated (to avoid panic-during-panic)
/// - The underscore prefix (`_guard`) prevents "unused variable" warnings for the guard variable
pub struct DllGuard<'a> {
    manager: &'a mut DllManager,
}

impl<'a> DllGuard<'a> {
    /// Create a new guard and disable DLLs
    ///
    /// # Errors
    ///
    /// Returns an error if any DLL cannot be disabled (see `DllManager::disable_dlls` for details)
    pub fn new(manager: &'a mut DllManager) -> Result<Self> {
        let count = manager.disable_dlls()?;
        if count > 0 {
            info!("DLL Guard: Disabled {} DLL(s)", count);
        }
        Ok(Self { manager })
    }
}

impl Drop for DllGuard<'_> {
    fn drop(&mut self) {
        match self.manager.restore_dlls() {
            Ok(count) if count > 0 => {
                info!("DLL Guard: Restored {} DLL(s)", count);
            }
            Ok(_) => {}
            Err(e) => {
                warn!("DLL Guard: Failed to restore DLLs: {}", e);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use tempfile::TempDir;

    #[test]
    fn test_scan_finds_interfering_dlls() {
        let temp_dir = TempDir::new().unwrap();
        let temp_path = temp_dir.path();

        // Create some test DLL files
        File::create(temp_path.join("d3d11.dll")).unwrap();
        File::create(temp_path.join("dxgi.dll")).unwrap();
        File::create(temp_path.join("other.dll")).unwrap(); // Should not be found

        let manager = DllManager::new(temp_path);
        let found = manager.scan();

        assert_eq!(found.len(), 2);
        assert!(found.iter().any(|p| p.file_name().unwrap() == "d3d11.dll"));
        assert!(found.iter().any(|p| p.file_name().unwrap() == "dxgi.dll"));
    }

    #[test]
    fn test_disable_and_restore_dlls() {
        let temp_dir = TempDir::new().unwrap();
        let temp_path = temp_dir.path();

        // Create test DLL
        File::create(temp_path.join("d3d11.dll")).unwrap();

        let mut manager = DllManager::new(temp_path);

        // Disable
        let disabled = manager.disable_dlls().unwrap();
        assert_eq!(disabled, 1);
        assert!(!temp_path.join("d3d11.dll").exists());
        assert!(temp_path.join("d3d11.dll-PJMdisabled").exists());

        // Restore
        let restored = manager.restore_dlls().unwrap();
        assert_eq!(restored, 1);
        assert!(temp_path.join("d3d11.dll").exists());
        assert!(!temp_path.join("d3d11.dll-PJMdisabled").exists());
    }

    #[test]
    fn test_dll_guard_raii() {
        let temp_dir = TempDir::new().unwrap();
        let temp_path = temp_dir.path();

        File::create(temp_path.join("d3d11.dll")).unwrap();

        let mut manager = DllManager::new(temp_path);

        {
            let _guard = DllGuard::new(&mut manager).unwrap();
            // DLL should be disabled
            assert!(!temp_path.join("d3d11.dll").exists());
            assert!(temp_path.join("d3d11.dll-PJMdisabled").exists());
        } // Guard dropped here

        // DLL should be restored
        assert!(temp_path.join("d3d11.dll").exists());
        assert!(!temp_path.join("d3d11.dll-PJMdisabled").exists());
    }
}
