//! Archive management abstraction for Fallout 4 BA2 archives
//!
//! This module provides a unified interface for managing Fallout 4 BA2 archives using
//! either Archive2.exe (Bethesda's official tool) or BSArch.exe (third-party tool).
//! The choice of tool significantly impacts workflow performance and capabilities.
//!
//! # Supported Archive Tools
//!
//! ## Archive2.exe (Bethesda Official)
//!
//! Archive2 is the official Bethesda archive tool included with the Creation Kit.
//!
//! **CRITICAL LIMITATION: NO APPEND SUPPORT**
//!
//! Archive2 **cannot** append files to existing archives. To add files to an existing
//! archive, Archive2 requires:
//! 1. Extract the entire archive to a temporary directory
//! 2. Copy new files into the extracted directory
//! 3. Delete the old archive
//! 4. Re-create the archive from the combined directory
//! 5. Clean up temporary files
//!
//! **This is NOT inefficient code - it's a fundamental limitation of Archive2.exe.**
//! See the original batch script lines 390-414 for the same workaround.
//!
//! ## BSArch.exe (Third-Party)
//!
//! BSArch is a community-created archive tool with superior automation support.
//!
//! **Advantages:**
//! - **Direct append support** - can add files to archives without extraction
//! - Multi-threaded compression (faster)
//! - Better command-line interface for automation
//!
//! BSArch is the recommended tool when available, especially for workflows that
//! add files to existing archives (Step 8: adding previs data to precombined archives).
//!
//! # MO2 Virtual File System Considerations
//!
//! When running through Mod Organizer 2 (MO2), archive tools cannot see files in
//! MO2's Virtual File System (VFS). This module handles MO2 by:
//! 1. Detecting when an MO2 staging directory is provided
//! 2. Collecting files from the VFS to a temporary real directory
//! 3. Archiving from the real directory
//! 4. Cleaning up temporary files
//!
//! See [`Mo2Helper`] for VFS file collection details.
//!
//! # Examples
//!
//! ```no_run
//! use std::path::PathBuf;
//! use generateprevisibines::tools::ArchiveManager;
//! use generateprevisibines::config::ArchiveTool;
//!
//! // Create manager with Archive2
//! let manager = ArchiveManager::new(
//!     ArchiveTool::Archive2,
//!     Some(PathBuf::from("C:\\CreationKit\\Tools\\Archive2.exe")),
//!     None,
//!     "C:\\Games\\Fallout4"
//! )?;
//!
//! // Create archive from precombined meshes
//! manager.create_archive_from_precombines("MyMod - Main.ba2", false, None)?;
//!
//! // Add previs data to the archive
//! manager.add_previs_to_archive("MyMod - Main.ba2", false, None)?;
//! # Ok::<(), anyhow::Error>(())
//! ```

use anyhow::{Context, Result, bail};
use log::info;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::ArchiveTool;
use crate::mo2_helper::Mo2Helper;

/// Archive manager that abstracts Archive2 and BSArch operations
///
/// Provides a unified interface for creating and modifying Fallout 4 BA2 archives
/// using either Archive2.exe or BSArch.exe. The implementation automatically handles
/// the significant differences between these tools.
///
/// # Key Differences Between Tools
///
/// | Feature | Archive2 | BSArch |
/// |---------|----------|--------|
/// | Append support | **NO** (must extract/repack) | **YES** (direct append) |
/// | Multi-threading | No | Yes |
/// | Source | Bethesda official | Community tool |
/// | Performance (append) | Slow (extract entire archive) | Fast (direct write) |
///
/// # Archive2 Limitations
///
/// **CRITICAL:** Archive2.exe has **NO APPEND FUNCTIONALITY**. To add files to an
/// existing archive, Archive2 must:
/// 1. Extract the entire archive
/// 2. Add new files to extracted directory
/// 3. Delete old archive
/// 4. Re-create archive from scratch
///
/// **This is NOT code smell - it's an Archive2.exe limitation.** The original batch
/// script (lines 390-414) uses the same workaround. Do not attempt to "optimize" this.
///
/// # BSArch Advantages
///
/// BSArch can append files directly to existing archives without extraction, making
/// it significantly faster for workflows that add files to archives (e.g., Step 8:
/// adding previs data to precombined archives).
///
/// # MO2 VFS Handling
///
/// When an MO2 staging directory is provided, this manager automatically:
/// - Collects files from MO2's VFS to a temporary real directory
/// - Archives from the real directory
/// - Cleans up temporary files
///
/// This is necessary because archive tools cannot see files in MO2's virtual filesystem.
///
/// # Examples
///
/// ```no_run
/// use std::path::{Path, PathBuf};
/// # use anyhow::Result;
/// # use generateprevisibines::config::ArchiveTool;
/// # use generateprevisibines::tools::ArchiveManager;
///
/// // Create with Archive2
/// let archive2_manager = ArchiveManager::new(
///     ArchiveTool::Archive2,
///     Some(PathBuf::from("C:\\CreationKit\\Tools\\Archive2.exe")),
///     None,
///     "C:\\Games\\Fallout4"
/// )?;
///
/// // Create with BSArch (recommended for automation)
/// let bsarch_manager = ArchiveManager::new(
///     ArchiveTool::BSArch,
///     None,
///     Some(PathBuf::from("C:\\Tools\\BSArch.exe")),
///     "C:\\Games\\Fallout4"
/// )?;
/// # Ok::<(), anyhow::Error>(())
/// ```
pub struct ArchiveManager {
    tool: ArchiveTool,
    archive2_exe: Option<PathBuf>,
    bsarch_exe: Option<PathBuf>,
    fallout4_dir: PathBuf,
}

impl ArchiveManager {
    /// Create a new archive manager
    ///
    /// Initializes an archive manager configured to use either Archive2 or BSArch.
    /// The appropriate executable path must be provided for the selected tool.
    ///
    /// # Arguments
    ///
    /// * `tool` - Which archive tool to use ([`ArchiveTool::Archive2`] or [`ArchiveTool::BSArch`])
    /// * `archive2_exe` - Path to Archive2.exe (required if `tool` is Archive2, ignored otherwise)
    /// * `bsarch_exe` - Path to BSArch.exe (required if `tool` is BSArch, ignored otherwise)
    /// * `fallout4_dir` - Path to Fallout 4 installation directory (e.g., `C:\Games\Fallout4`)
    ///
    /// # Returns
    ///
    /// Returns a configured [`ArchiveManager`] instance
    ///
    /// # Errors
    ///
    /// This function will return an error if:
    /// - `tool` is [`ArchiveTool::Archive2`] but `archive2_exe` is `None`
    /// - `tool` is [`ArchiveTool::BSArch`] but `bsarch_exe` is `None`
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::path::PathBuf;
    /// use generateprevisibines::config::ArchiveTool;
    /// use generateprevisibines::tools::ArchiveManager;
    ///
    /// // Create Archive2 manager
    /// let archive2 = ArchiveManager::new(
    ///     ArchiveTool::Archive2,
    ///     Some(PathBuf::from("C:\\CreationKit\\Tools\\Archive2.exe")),
    ///     None,
    ///     "C:\\Games\\Fallout4"
    /// )?;
    ///
    /// // Create BSArch manager
    /// let bsarch = ArchiveManager::new(
    ///     ArchiveTool::BSArch,
    ///     None,
    ///     Some(PathBuf::from("C:\\Tools\\BSArch.exe")),
    ///     "C:\\Games\\Fallout4"
    /// )?;
    /// # Ok::<(), anyhow::Error>(())
    /// ```
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
    /// Creates a BA2 archive from all files in the specified directory. The behavior
    /// differs between Archive2 and BSArch regarding source file cleanup.
    ///
    /// # Arguments
    ///
    /// * `source_dir` - Directory containing files to archive
    /// * `archive_name` - Name of the archive to create (e.g., `"MyMod - Main.ba2"`)
    /// * `is_xbox` - If `true`, uses Xbox compression format; if `false`, uses PC format
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` if the archive was created successfully
    ///
    /// # Errors
    ///
    /// This function will return an error if:
    /// - Source directory does not exist or cannot be read
    /// - Archive creation fails (disk full, permission denied, invalid format)
    /// - **Archive2:** Source directory cannot be deleted after archiving
    ///
    /// # Tool-Specific Behavior
    ///
    /// ## Archive2
    /// 1. Creates the archive from `source_dir`
    /// 2. **Deletes** the source directory and all its contents
    ///
    /// This is Archive2's standard behavior - it assumes you want to replace loose files
    /// with archived versions.
    ///
    /// ## BSArch
    /// 1. Creates the archive from `source_dir`
    /// 2. **Preserves** the source directory and files
    ///
    /// BSArch keeps the source files, allowing you to verify the archive before cleanup.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::path::Path;
    /// # use generateprevisibines::tools::ArchiveManager;
    /// # use generateprevisibines::config::ArchiveTool;
    /// # use std::path::PathBuf;
    /// # let manager = ArchiveManager::new(
    /// #     ArchiveTool::Archive2,
    /// #     Some(PathBuf::from("Archive2.exe")),
    /// #     None,
    /// #     "C:\\Games\\Fallout4"
    /// # )?;
    ///
    /// let precombined_dir = Path::new("C:\\Games\\Fallout4\\Data\\meshes\\precombined");
    /// manager.create_archive(precombined_dir, "MyMod - Main.ba2", false)?;
    /// # Ok::<(), anyhow::Error>(())
    /// ```
    ///
    /// # Notes
    ///
    /// - Archive is always created in `Data/` directory
    /// - **Archive2:** Source files are permanently deleted - ensure workflow completed successfully
    /// - **BSArch:** You may want to manually delete source files after verification
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
                fs::remove_dir_all(source_dir).with_context(|| {
                    format!("Failed to delete source: {}", source_dir.display())
                })?;
            }
            ArchiveTool::BSArch => {
                self.bsarch_pack(source_dir, &archive_path)?;
                // BSArch: Keep source files
            }
        }

        Ok(())
    }

    /// Create a new archive from precombined meshes (MO2-aware)
    ///
    /// Archives all `.nif` files from the `meshes/precombined` directory. When running
    /// in Mod Organizer 2 mode, this handles MO2's Virtual File System (VFS) by collecting
    /// files from the staging directory instead of the real Data directory.
    ///
    /// This is typically used in **Step 5** of the workflow after CreationKit generates
    /// precombined meshes.
    ///
    /// # Arguments
    ///
    /// * `archive_name` - Name of the archive to create (e.g., `"MyMod - Main.ba2"`)
    /// * `is_xbox` - If `true`, uses Xbox compression format; if `false`, uses PC format
    /// * `mo2_data_dir` - Optional path to MO2's VFS staging directory (e.g., `overwrite` folder).
    ///   When `Some`, files are collected from MO2's VFS. When `None`, files are read directly
    ///   from `Data/meshes/precombined`.
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` if the archive was created successfully
    ///
    /// # Errors
    ///
    /// This function will return an error if:
    /// - **MO2 mode:** MO2 staging directory does not exist or cannot be accessed
    /// - **MO2 mode:** No precombined meshes found in staging directory (workflow incomplete)
    /// - **Standard mode:** No precombined meshes found in `Data/meshes/precombined`
    /// - Archive creation fails (disk full, permission denied, invalid archive format)
    /// - Temporary directory cannot be created or cleaned up
    ///
    /// # MO2 Virtual File System Behavior
    ///
    /// When `mo2_data_dir` is provided:
    /// 1. Creates a temporary collection directory in `Data/_temp_mo2_collect`
    /// 2. Copies all files from `mo2_data_dir/meshes/precombined` to temp directory
    /// 3. Archives the collected files using the selected tool
    /// 4. Deletes the temporary collection directory
    ///
    /// This is necessary because Archive2 and BSArch cannot see files in MO2's Virtual
    /// File System. The files must be in a real directory for archiving.
    ///
    /// # File Collection Process
    ///
    /// - **Standard mode:** Archives directly from `Data/meshes/precombined`
    /// - **MO2 mode:** Uses [`Mo2Helper::collect_precombines`](crate::mo2_helper::Mo2Helper::collect_precombines)
    ///   to gather files from the VFS
    /// - After archiving, source files are deleted (Archive2) or kept (BSArch)
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::path::Path;
    /// # use generateprevisibines::tools::ArchiveManager;
    /// # use generateprevisibines::config::ArchiveTool;
    /// # use std::path::PathBuf;
    /// # let manager = ArchiveManager::new(
    /// #     ArchiveTool::Archive2,
    /// #     Some(PathBuf::from("Archive2.exe")),
    /// #     None,
    /// #     "C:\\Games\\Fallout4"
    /// # )?;
    ///
    /// // Standard mode (no MO2)
    /// manager.create_archive_from_precombines("MyMod - Main.ba2", false, None)?;
    ///
    /// // MO2 mode - collect from VFS
    /// let mo2_overwrite = Path::new("C:\\MO2\\overwrite");
    /// manager.create_archive_from_precombines(
    ///     "MyMod - Main.ba2",
    ///     false,
    ///     Some(mo2_overwrite)
    /// )?;
    ///
    /// // Xbox format
    /// manager.create_archive_from_precombines("MyMod - Main.ba2", true, None)?;
    /// # Ok::<(), anyhow::Error>(())
    /// ```
    ///
    /// # Notes
    ///
    /// - The archive is created in `Data/` directory regardless of MO2 mode
    /// - For Archive2, source files are deleted after archiving
    /// - For BSArch, source files are preserved
    /// - Temporary MO2 collection directories are always cleaned up
    pub fn create_archive_from_precombines(
        &self,
        archive_name: &str,
        is_xbox: bool,
        mo2_data_dir: Option<&Path>,
    ) -> Result<()> {
        let data_dir = self.fallout4_dir.join("Data");

        if let Some(mo2_staging) = mo2_data_dir {
            // MO2 mode: Collect files from staging directory
            let mo2_helper = Mo2Helper::new(mo2_staging)?;
            info!(
                "MO2 mode: Collecting precombined meshes from staging directory: {}",
                mo2_helper.staging_dir().display()
            );

            let temp_collect = data_dir.join("_temp_mo2_collect");

            let collected_dir = mo2_helper
                .collect_precombines(&temp_collect)
                .context("Failed to collect precombines from MO2 staging directory")?;

            if let Some(collected) = collected_dir {
                // Archive from collected files
                self.create_archive(&collected, archive_name, is_xbox)?;

                // Cleanup temp directory
                if temp_collect.exists() {
                    fs::remove_dir_all(&temp_collect)?;
                }
            } else {
                bail!("No precombined meshes found in MO2 staging directory");
            }
        } else {
            // Standard mode: Use files from Data directory
            let precombined_dir = data_dir.join("meshes").join("precombined");
            self.create_archive(&precombined_dir, archive_name, is_xbox)?;
        }

        Ok(())
    }

    /// Add files to an existing archive
    ///
    /// Appends new files to an existing BA2 archive. The implementation differs
    /// significantly between Archive2 and BSArch due to **Archive2's critical limitation**.
    ///
    /// This is typically used in **Step 8** of the workflow to add previs data (`.uvd` files)
    /// to the archive containing precombined meshes.
    ///
    /// # Arguments
    ///
    /// * `source_dir` - Directory containing files to add to the archive (typically `Data/vis`)
    /// * `archive_name` - Name of the existing archive (e.g., `"MyMod - Main.ba2"`). **Must exist.**
    /// * `is_xbox` - If `true`, uses Xbox compression format; if `false`, uses PC format
    ///   (only relevant for Archive2 re-archiving)
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` if files were successfully added to the archive
    ///
    /// # Errors
    ///
    /// This function will return an error if:
    /// - Archive does not exist (must be created first)
    /// - **Archive2:** Extraction fails (corrupted archive, insufficient disk space, permission denied)
    /// - **Archive2:** Cannot delete old archive after extraction (file locked, permission denied)
    /// - **Archive2:** Cannot create temporary extraction directory
    /// - File copying fails (disk full, permission denied, I/O error)
    /// - Archive creation/packing fails (corrupted data, invalid format, disk full)
    /// - **Archive2:** Partial operation failure may leave archive in inconsistent state
    ///
    /// # Archive2 Limitation - Extract/Repack Workaround
    ///
    /// **CRITICAL: Archive2.exe has NO APPEND FUNCTIONALITY.**
    ///
    /// This is **NOT inefficient code** - it's a fundamental limitation of Archive2.exe itself.
    /// The original batch script (lines 390-414) uses the exact same workaround.
    ///
    /// ## Archive2 Workflow (REQUIRED)
    ///
    /// To add files to an existing archive using Archive2, we must:
    /// 1. **Extract** the entire archive to `Data/_temp_archive_extract`
    /// 2. **Copy** new files into the extracted directory
    /// 3. **Delete** the old archive file
    /// 4. **Re-create** the archive from the combined directory (existing + new files)
    /// 5. **Clean up** the temporary extraction directory
    /// 6. **Delete** the source directory
    ///
    /// This process extracts and re-compresses the entire archive, even if only adding
    /// a few small files. For large archives (e.g., 500MB+), this can take several minutes.
    ///
    /// **Do not attempt to "optimize" this - Archive2.exe provides no append functionality.**
    ///
    /// ## Why Not Just Use BSArch?
    ///
    /// BSArch is recommended when available, but:
    /// - Not all users have BSArch installed
    /// - Archive2 ships with the Creation Kit (guaranteed availability)
    /// - Some users prefer official Bethesda tools
    ///
    /// # BSArch Behavior
    ///
    /// BSArch can **append files directly** to existing archives without extraction:
    /// 1. Add new files to the archive using `bsarch pack` command
    /// 2. Delete source directory
    ///
    /// This is **much faster** than Archive2's extract/repack process, often completing
    /// in seconds rather than minutes for large archives.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::path::Path;
    /// # use generateprevisibines::tools::ArchiveManager;
    /// # use generateprevisibines::config::ArchiveTool;
    /// # use std::path::PathBuf;
    /// # let manager = ArchiveManager::new(
    /// #     ArchiveTool::Archive2,
    /// #     Some(PathBuf::from("Archive2.exe")),
    /// #     None,
    /// #     "C:\\Games\\Fallout4"
    /// # )?;
    ///
    /// // Add previs data to existing precombined archive
    /// let vis_dir = Path::new("C:\\Games\\Fallout4\\Data\\vis");
    /// manager.add_to_archive(vis_dir, "MyMod - Main.ba2", false)?;
    ///
    /// // With Xbox compression
    /// manager.add_to_archive(vis_dir, "MyMod - Main.ba2", true)?;
    /// # Ok::<(), anyhow::Error>(())
    /// ```
    ///
    /// # Performance Comparison
    ///
    /// For a 500MB archive with 10MB of new files:
    /// - **Archive2:** ~3-5 minutes (extract 500MB, compress 510MB)
    /// - **BSArch:** ~5-10 seconds (compress and append 10MB)
    ///
    /// # Notes
    ///
    /// - The source directory is **always deleted** after successful archiving (both tools)
    /// - For Archive2, temporary directories are cleaned up even if errors occur
    /// - **Archive2 only:** If an error occurs during re-archiving, the original archive
    ///   may be lost. Consider backing up important archives before modification.
    /// - The archive must exist before calling this function (use `create_archive` first)
    ///
    /// # See Also
    ///
    /// - Original batch script lines 390-414 for the same Archive2 workaround
    /// - [`create_archive_from_precombines`](Self::create_archive_from_precombines) for creating new archives
    /// - [`add_previs_to_archive`](Self::add_previs_to_archive) for MO2-aware previs addition
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

                // Use closure to ensure cleanup on both success and error paths
                let result = (|| -> Result<()> {
                    // Extract existing archive
                    self.archive2_extract(&archive_path, &temp_extract)?;

                    // Copy new files to extracted directory
                    self.copy_dir_recursive(source_dir, &temp_extract)?;

                    // Delete old archive
                    fs::remove_file(&archive_path)?;

                    // Re-create archive with all files
                    self.archive2_create(&temp_extract, &archive_path, is_xbox)?;

                    Ok(())
                })();

                // Cleanup temp directory regardless of success/failure
                if temp_extract.exists() {
                    let _ = fs::remove_dir_all(&temp_extract);
                }

                // Propagate any error from the operation
                result?;

                // Clean up source directory on success
                fs::remove_dir_all(source_dir)?;
            }
            ArchiveTool::BSArch => {
                // BSArch can append
                self.bsarch_pack(source_dir, &archive_path)?;
            }
        }

        Ok(())
    }

    /// Add previs files to an existing archive (MO2-aware)
    ///
    /// Adds all `.uvd` files from the `vis` directory to an existing BA2 archive.
    /// When running in Mod Organizer 2 mode, this handles MO2's Virtual File System (VFS)
    /// by collecting files from the staging directory.
    ///
    /// This is typically used in **Step 8** of the workflow to combine previs data with
    /// the precombined meshes archive created in Step 5.
    ///
    /// # Arguments
    ///
    /// * `archive_name` - Name of the existing archive (e.g., `"MyMod - Main.ba2"`). **Must exist.**
    /// * `is_xbox` - If `true`, uses Xbox compression format; if `false`, uses PC format
    ///   (only relevant for Archive2 re-archiving)
    /// * `mo2_data_dir` - Optional path to MO2's VFS staging directory (e.g., `overwrite` folder).
    ///   When `Some`, files are collected from MO2's VFS. When `None`, files are read directly
    ///   from `Data/vis`.
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` if previs files were successfully added to the archive
    ///
    /// # Errors
    ///
    /// This function will return an error if:
    /// - Archive does not exist (must be created first via `create_archive_from_precombines`)
    /// - **MO2 mode:** MO2 staging directory does not exist or cannot be accessed
    /// - **MO2 mode:** No previs data found in staging directory (workflow incomplete)
    /// - **Standard mode:** No previs data found in `Data/vis`
    /// - Archive modification fails (see [`add_to_archive`](Self::add_to_archive) for details)
    /// - Temporary directory cannot be created or cleaned up
    ///
    /// # MO2 Virtual File System Behavior
    ///
    /// When `mo2_data_dir` is provided:
    /// 1. Creates a temporary collection directory in `Data/_temp_mo2_collect`
    /// 2. Copies all files from `mo2_data_dir/vis` to temp directory
    /// 3. Adds collected files to the archive using the selected tool
    /// 4. Deletes the temporary collection directory
    ///
    /// This is necessary because Archive2 and BSArch cannot see files in MO2's Virtual
    /// File System. The files must be in a real directory for archiving.
    ///
    /// # Archive Tool Behavior
    ///
    /// - **Archive2:** Extracts entire archive, adds previs files, re-archives everything
    ///   (see [`add_to_archive`](Self::add_to_archive) for details on the extract/repack process)
    /// - **BSArch:** Directly appends previs files to existing archive (much faster)
    ///
    /// # File Collection Process
    ///
    /// - **Standard mode:** Archives directly from `Data/vis`
    /// - **MO2 mode:** Uses [`Mo2Helper::collect_previs`](crate::mo2_helper::Mo2Helper::collect_previs)
    ///   to gather files from the VFS
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::path::Path;
    /// # use generateprevisibines::tools::ArchiveManager;
    /// # use generateprevisibines::config::ArchiveTool;
    /// # use std::path::PathBuf;
    /// # let manager = ArchiveManager::new(
    /// #     ArchiveTool::Archive2,
    /// #     Some(PathBuf::from("Archive2.exe")),
    /// #     None,
    /// #     "C:\\Games\\Fallout4"
    /// # )?;
    ///
    /// // Standard mode (no MO2)
    /// manager.add_previs_to_archive("MyMod - Main.ba2", false, None)?;
    ///
    /// // MO2 mode - collect from VFS
    /// let mo2_overwrite = Path::new("C:\\MO2\\overwrite");
    /// manager.add_previs_to_archive(
    ///     "MyMod - Main.ba2",
    ///     false,
    ///     Some(mo2_overwrite)
    /// )?;
    ///
    /// // Xbox format
    /// manager.add_previs_to_archive("MyMod - Main.ba2", true, None)?;
    /// # Ok::<(), anyhow::Error>(())
    /// ```
    ///
    /// # Performance Notes
    ///
    /// For a 500MB precombined archive with 10MB of previs data:
    /// - **Archive2:** ~3-5 minutes (must extract and re-compress entire 510MB)
    /// - **BSArch:** ~5-10 seconds (directly appends 10MB)
    ///
    /// BSArch is **strongly recommended** for this operation due to the significant
    /// performance difference.
    ///
    /// # Notes
    ///
    /// - The archive **must exist** before calling this function
    /// - Typically called after `create_archive_from_precombines`
    /// - Temporary MO2 collection directories are always cleaned up
    /// - Source files in `Data/vis` (or MO2 staging) are deleted after archiving
    ///
    /// # See Also
    ///
    /// - [`add_to_archive`](Self::add_to_archive) for detailed Archive2 limitation documentation
    /// - [`create_archive_from_precombines`](Self::create_archive_from_precombines) for creating the initial archive
    pub fn add_previs_to_archive(
        &self,
        archive_name: &str,
        is_xbox: bool,
        mo2_data_dir: Option<&Path>,
    ) -> Result<()> {
        let data_dir = self.fallout4_dir.join("Data");

        if let Some(mo2_staging) = mo2_data_dir {
            // MO2 mode: Collect files from staging directory
            let mo2_helper = Mo2Helper::new(mo2_staging)?;
            info!(
                "MO2 mode: Collecting previs data from staging directory: {}",
                mo2_helper.staging_dir().display()
            );

            let temp_collect = data_dir.join("_temp_mo2_collect");

            let collected_dir = mo2_helper
                .collect_previs(&temp_collect)
                .context("Failed to collect previs from MO2 staging directory")?;

            if let Some(collected) = collected_dir {
                // Add collected files to archive
                self.add_to_archive(&collected, archive_name, is_xbox)?;

                // Cleanup temp directory
                if temp_collect.exists() {
                    fs::remove_dir_all(&temp_collect)?;
                }
            } else {
                bail!("No previs data found in MO2 staging directory");
            }
        } else {
            // Standard mode: Use files from Data directory
            let vis_dir = data_dir.join("vis");
            self.add_to_archive(&vis_dir, archive_name, is_xbox)?;
        }

        Ok(())
    }

    /// Create archive using Archive2
    ///
    /// Internal helper that invokes Archive2.exe to create a BA2 archive from a directory.
    ///
    /// # Arguments
    ///
    /// * `source_dir` - Directory containing files to archive
    /// * `archive_path` - Full path to the archive file to create
    /// * `is_xbox` - If `true`, uses Xbox compression; otherwise uses PC compression
    ///
    /// # Archive2 Command
    ///
    /// Executes: `Archive2.exe <source_dir> -c=<archive_path> -f=General -q [-compression=XBox]`
    ///
    /// # Errors
    ///
    /// Returns an error if Archive2.exe fails or cannot be executed
    fn archive2_create(&self, source_dir: &Path, archive_path: &Path, is_xbox: bool) -> Result<()> {
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
    ///
    /// Internal helper that invokes Archive2.exe to extract a BA2 archive.
    ///
    /// This is used as part of the extract/repack workaround for adding files to
    /// existing archives (see [`add_to_archive`](Self::add_to_archive)).
    ///
    /// # Arguments
    ///
    /// * `archive_path` - Full path to the archive file to extract
    /// * `dest_dir` - Directory where files will be extracted
    ///
    /// # Archive2 Command
    ///
    /// Executes: `Archive2.exe <archive_path> -e=<dest_dir> -q`
    ///
    /// # Errors
    ///
    /// Returns an error if Archive2.exe fails or cannot be executed
    fn archive2_extract(&self, archive_path: &Path, dest_dir: &Path) -> Result<()> {
        let Some(ref archive2_exe) = self.archive2_exe else {
            bail!("Archive2.exe not configured");
        };

        info!(
            "Extracting archive with Archive2: {}",
            archive_path.display()
        );

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
    ///
    /// Internal helper that invokes BSArch.exe to create or append to a BA2 archive.
    ///
    /// Unlike Archive2, BSArch can both create new archives and append to existing ones
    /// using the same command. If the archive exists, files are appended; if not, it's created.
    ///
    /// # Arguments
    ///
    /// * `source_dir` - Directory containing files to archive or append
    /// * `archive_path` - Full path to the archive file (created if doesn't exist)
    ///
    /// # BSArch Command
    ///
    /// Executes: `BSArch.exe pack <source_dir> <archive_path> -mt -fo4 -z`
    ///
    /// Flags:
    /// - `-mt`: Multi-threaded compression
    /// - `-fo4`: Fallout 4 archive format
    /// - `-z`: Compress files
    ///
    /// # Errors
    ///
    /// Returns an error if BSArch.exe fails or cannot be executed
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
                "-mt",  // Multi-threaded
                "-fo4", // Fallout 4 format
                "-z",   // Compress
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
    ///
    /// Internal helper that copies all files and subdirectories from source to destination.
    /// Used as part of the Archive2 extract/repack workaround to merge new files with
    /// extracted archive contents.
    ///
    /// # Arguments
    ///
    /// * `src` - Source directory to copy from
    /// * `dst` - Destination directory to copy to (created if doesn't exist)
    ///
    /// # Behavior
    ///
    /// - Creates destination directory if it doesn't exist
    /// - Recursively copies all subdirectories
    /// - Overwrites existing files at destination
    ///
    /// # Errors
    ///
    /// Returns an error if any file or directory operation fails
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
        let result = ArchiveManager::new(ArchiveTool::Archive2, None, None, "F:\\Games\\Fallout4");
        assert!(result.is_err());

        // BSArch without exe should fail
        let result = ArchiveManager::new(ArchiveTool::BSArch, None, None, "F:\\Games\\Fallout4");
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
