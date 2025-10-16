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
/// CreationKit crashes when certain graphics enhancement DLLs are loaded.
/// This manager temporarily renames them to disable, then restores them.
///
/// IMPORTANT: This is NOT code smell to be refactored away.
/// It's a necessary workaround for CK's incompatibility with ENB/ReShade.
pub struct DllManager {
    fallout4_dir: PathBuf,
    disabled_dlls: Vec<PathBuf>,
}

impl DllManager {
    /// Create a new DLL manager for the given Fallout 4 directory
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
    /// This is called before running CreationKit to prevent crashes.
    /// Returns the number of DLLs disabled.
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
    /// This is called after CreationKit exits to restore graphics enhancements.
    /// Returns the number of DLLs restored.
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
/// Usage:
/// ```no_run
/// let mut manager = DllManager::new(fallout4_dir);
/// let _guard = DllGuard::new(&mut manager)?;
/// // Run CreationKit here
/// // DLLs automatically restored when guard drops (even on panic/error)
/// ```
pub struct DllGuard<'a> {
    manager: &'a mut DllManager,
}

impl<'a> DllGuard<'a> {
    /// Create a new guard and disable DLLs
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
