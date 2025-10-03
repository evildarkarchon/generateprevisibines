use anyhow::{bail, Context, Result};
use log::info;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::ArchiveTool;

/// Archive manager that abstracts Archive2 and BSArch
///
/// Archive2 limitations (IMPORTANT - NOT code smell to work around):
/// - Cannot append to existing archives
/// - Must extract, modify, then re-archive (see batch lines 390-414)
///
/// BSArch advantages:
/// - Can append to existing archives
/// - Uses temporary staging directory
pub struct ArchiveManager {
    tool: ArchiveTool,
    archive2_exe: Option<PathBuf>,
    bsarch_exe: Option<PathBuf>,
    fallout4_dir: PathBuf,
}

impl ArchiveManager {
    /// Create a new archive manager
    pub fn new(
        tool: ArchiveTool,
        archive2_exe: Option<PathBuf>,
        bsarch_exe: Option<PathBuf>,
        fallout4_dir: impl AsRef<Path>,
    ) -> Result<Self> {
        // Validate tool availability
        match tool {
            ArchiveTool::Archive2 => {
                if archive2_exe.is_none() {
                    bail!("Archive2.exe not found");
                }
            }
            ArchiveTool::BSArch => {
                if bsarch_exe.is_none() {
                    bail!("BSArch.exe not found");
                }
            }
        }

        Ok(Self {
            tool,
            archive2_exe,
            bsarch_exe,
            fallout4_dir: fallout4_dir.as_ref().to_path_buf(),
        })
    }

    /// Create a new archive from a directory
    ///
    /// Archive2: Creates archive, deletes source files
    /// BSArch: Creates archive, keeps source files
    pub fn create_archive(
        &self,
        source_dir: impl AsRef<Path>,
        archive_name: &str,
        is_xbox: bool,
    ) -> Result<()> {
        let source_dir = source_dir.as_ref();
        let data_dir = self.fallout4_dir.join("Data");
        let archive_path = data_dir.join(archive_name);

        match self.tool {
            ArchiveTool::Archive2 => {
                self.archive2_create(source_dir, &archive_path, is_xbox)?;

                // Archive2: Delete source files after archiving
                info!("Deleting source files: {}", source_dir.display());
                fs::remove_dir_all(source_dir)
                    .with_context(|| format!("Failed to delete source: {}", source_dir.display()))?;
            }
            ArchiveTool::BSArch => {
                self.bsarch_pack(source_dir, &archive_path)?;
                // BSArch: Keep source files
            }
        }

        Ok(())
    }

    /// Add files to an existing archive
    ///
    /// Archive2: Extract, add files, re-archive (NO APPEND SUPPORT)
    /// BSArch: Can append directly to existing archive
    pub fn add_to_archive(
        &self,
        source_dir: impl AsRef<Path>,
        archive_name: &str,
        is_xbox: bool,
    ) -> Result<()> {
        let source_dir = source_dir.as_ref();
        let data_dir = self.fallout4_dir.join("Data");
        let archive_path = data_dir.join(archive_name);

        if !archive_path.exists() {
            bail!("Archive does not exist: {}", archive_path.display());
        }

        match self.tool {
            ArchiveTool::Archive2 => {
                // REQUIRED WORKAROUND: Archive2 cannot append
                // Must extract, add files, then re-archive
                info!("Archive2: Extracting archive to add files (no append support)");

                let temp_extract = data_dir.join("_temp_archive_extract");

                // Create temp directory
                if temp_extract.exists() {
                    fs::remove_dir_all(&temp_extract)?;
                }
                fs::create_dir_all(&temp_extract)?;

                // Extract existing archive
                self.archive2_extract(&archive_path, &temp_extract)?;

                // Copy new files to extracted directory
                self.copy_dir_recursive(source_dir, &temp_extract)?;

                // Delete old archive
                fs::remove_file(&archive_path)?;

                // Re-create archive with all files
                self.archive2_create(&temp_extract, &archive_path, is_xbox)?;

                // Cleanup
                fs::remove_dir_all(&temp_extract)?;
                fs::remove_dir_all(source_dir)?;
            }
            ArchiveTool::BSArch => {
                // BSArch can append
                self.bsarch_pack(source_dir, &archive_path)?;
            }
        }

        Ok(())
    }

    /// Create archive using Archive2
    fn archive2_create(
        &self,
        source_dir: &Path,
        archive_path: &Path,
        is_xbox: bool,
    ) -> Result<()> {
        let Some(ref archive2_exe) = self.archive2_exe else {
            bail!("Archive2.exe not configured");
        };

        info!("Creating archive with Archive2: {}", archive_path.display());

        let mut args = vec![
            source_dir.to_string_lossy().to_string(),
            format!("-c={}", archive_path.display()),
            "-f=General".to_string(),
            "-q".to_string(), // Quiet mode
        ];

        if is_xbox {
            args.push("-compression=XBox".to_string());
        }

        let output = Command::new(archive2_exe)
            .args(&args)
            .current_dir(&self.fallout4_dir)
            .output()
            .with_context(|| format!("Failed to run Archive2: {}", archive2_exe.display()))?;

        if !output.status.success() {
            bail!(
                "Archive2 failed: {}\nStderr: {}",
                output.status,
                String::from_utf8_lossy(&output.stderr)
            );
        }

        Ok(())
    }

    /// Extract archive using Archive2
    fn archive2_extract(&self, archive_path: &Path, dest_dir: &Path) -> Result<()> {
        let Some(ref archive2_exe) = self.archive2_exe else {
            bail!("Archive2.exe not configured");
        };

        info!("Extracting archive with Archive2: {}", archive_path.display());

        let output = Command::new(archive2_exe)
            .args(&[
                archive_path.to_string_lossy().to_string(),
                format!("-e={}", dest_dir.display()),
                "-q".to_string(),
            ])
            .current_dir(&self.fallout4_dir)
            .output()
            .with_context(|| format!("Failed to run Archive2: {}", archive2_exe.display()))?;

        if !output.status.success() {
            bail!(
                "Archive2 extraction failed: {}\nStderr: {}",
                output.status,
                String::from_utf8_lossy(&output.stderr)
            );
        }

        Ok(())
    }

    /// Pack archive using BSArch
    fn bsarch_pack(&self, source_dir: &Path, archive_path: &Path) -> Result<()> {
        let Some(ref bsarch_exe) = self.bsarch_exe else {
            bail!("BSArch.exe not configured");
        };

        info!("Packing archive with BSArch: {}", archive_path.display());

        let output = Command::new(bsarch_exe)
            .args(&[
                "pack",
                &source_dir.to_string_lossy(),
                &archive_path.to_string_lossy(),
                "-mt",   // Multi-threaded
                "-fo4",  // Fallout 4 format
                "-z",    // Compress
            ])
            .current_dir(&self.fallout4_dir)
            .output()
            .with_context(|| format!("Failed to run BSArch: {}", bsarch_exe.display()))?;

        if !output.status.success() {
            bail!(
                "BSArch failed: {}\nStderr: {}",
                output.status,
                String::from_utf8_lossy(&output.stderr)
            );
        }

        Ok(())
    }

    /// Recursively copy directory contents
    fn copy_dir_recursive(&self, src: &Path, dst: &Path) -> Result<()> {
        if !dst.exists() {
            fs::create_dir_all(dst)?;
        }

        for entry in fs::read_dir(src)? {
            let entry = entry?;
            let file_type = entry.file_type()?;
            let src_path = entry.path();
            let dst_path = dst.join(entry.file_name());

            if file_type.is_dir() {
                self.copy_dir_recursive(&src_path, &dst_path)?;
            } else {
                fs::copy(&src_path, &dst_path)?;
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_archive_manager_requires_exe() {
        // Archive2 without exe should fail
        let result = ArchiveManager::new(
            ArchiveTool::Archive2,
            None,
            None,
            "F:\\Games\\Fallout4",
        );
        assert!(result.is_err());

        // BSArch without exe should fail
        let result = ArchiveManager::new(
            ArchiveTool::BSArch,
            None,
            None,
            "F:\\Games\\Fallout4",
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_archive_manager_with_exe() {
        let result = ArchiveManager::new(
            ArchiveTool::Archive2,
            Some(PathBuf::from("Archive2.exe")),
            None,
            "F:\\Games\\Fallout4",
        );
        assert!(result.is_ok());

        let result = ArchiveManager::new(
            ArchiveTool::BSArch,
            None,
            Some(PathBuf::from("BSArch.exe")),
            "F:\\Games\\Fallout4",
        );
        assert!(result.is_ok());
    }
}
