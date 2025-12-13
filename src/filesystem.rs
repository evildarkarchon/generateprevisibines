use anyhow::{Context, Result};
use log::warn;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Check if required FO4 directories exist
pub fn validate_fo4_directories(fo4_dir: &Path) -> Result<()> {
    let data_dir = fo4_dir.join("Data");
    if !data_dir.exists() {
        anyhow::bail!(
            "Data directory not found at: {}\n\
            This doesn't appear to be a valid Fallout 4 installation.",
            data_dir.display()
        );
    }

    // Check for Fallout4.esm as a sanity check
    let fo4_esm = data_dir.join("Fallout4.esm");
    if !fo4_esm.exists() {
        anyhow::bail!(
            "Fallout4.esm not found in Data directory.\n\
            This doesn't appear to be a valid Fallout 4 installation."
        );
    }

    Ok(())
}

/// Create required directories if they don't exist
/// Returns paths to meshes/precombined and vis directories
pub fn ensure_output_directories(data_dir: &Path) -> Result<(PathBuf, PathBuf)> {
    let meshes_dir = data_dir.join("meshes");
    let precombined_dir = meshes_dir.join("precombined");
    let vis_dir = data_dir.join("vis");

    // Create directories if they don't exist
    if !meshes_dir.exists() {
        fs::create_dir_all(&meshes_dir).context(format!(
            "Failed to create directory: {}",
            meshes_dir.display()
        ))?;
    }

    if !precombined_dir.exists() {
        fs::create_dir_all(&precombined_dir).context(format!(
            "Failed to create directory: {}",
            precombined_dir.display()
        ))?;
    }

    if !vis_dir.exists() {
        fs::create_dir_all(&vis_dir)
            .context(format!("Failed to create directory: {}", vis_dir.display()))?;
    }

    Ok((precombined_dir, vis_dir))
}

/// Scan a directory for files matching a file extension
///
/// Walks through the specified directory (optionally recursively) and collects
/// all files with the given extension. File paths are returned as **absolute paths**,
/// not relative paths.
///
/// # Arguments
///
/// * `dir` - Directory to search
/// * `extension` - File extension to match (without leading dot, e.g., "esp" not ".esp")
/// * `recursive` - If `true`, searches subdirectories; if `false`, searches only the top level
///
/// # Returns
///
/// Returns a vector of absolute file paths matching the extension. Returns an empty
/// vector if the directory doesn't exist.
///
/// # Errors
///
/// This function will return an error if:
/// - Directory exists but cannot be read (permission denied)
/// - Directory traversal encounters I/O errors
///
/// Note: If the directory doesn't exist, this returns `Ok(Vec::new())` without an error.
///
/// # Examples
///
/// ```no_run
/// use std::path::Path;
/// # use anyhow::Result;
/// # use generateprevisibines::filesystem::scan_directory_for_files;
///
/// # fn example() -> Result<()> {
/// // Find all .esp files in Data directory (non-recursive)
/// let data_dir = Path::new("C:\\Games\\Fallout4\\Data");
/// let esp_files = scan_directory_for_files(data_dir, "esp", false)?;
/// println!("Found {} ESP files", esp_files.len());
///
/// // Find all .nif files recursively in meshes
/// let meshes_dir = data_dir.join("meshes");
/// let nif_files = scan_directory_for_files(&meshes_dir, "nif", true)?;
/// println!("Found {} NIF files (recursive)", nif_files.len());
/// # Ok(())
/// # }
/// ```
///
/// # Notes
///
/// - Extension matching is **case-insensitive** (both "ESP" and "esp" will match)
/// - Does not follow symlinks
/// - Skips directories and non-file entries
/// - Returns absolute paths, not relative paths
#[allow(dead_code)] // Part of public filesystem utility API; available for external use
pub fn scan_directory_for_files(
    dir: &Path,
    extension: &str,
    recursive: bool,
) -> Result<Vec<PathBuf>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut files = Vec::new();
    let extension_lower = extension.to_lowercase();

    let walker = if recursive {
        WalkDir::new(dir)
    } else {
        WalkDir::new(dir).max_depth(1)
    };

    for entry in walker.into_iter().filter_map(|e| e.ok()) {
        if entry.file_type().is_file() {
            if let Some(ext) = entry.path().extension() {
                if ext.to_string_lossy().to_lowercase() == extension_lower {
                    files.push(entry.path().to_path_buf());
                }
            }
        }
    }

    Ok(files)
}

/// Count files in a directory with a specific extension
pub fn count_files(dir: &Path, extension: &str) -> usize {
    if !dir.exists() {
        return 0;
    }

    WalkDir::new(dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter(|e| {
            e.path()
                .extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| ext.eq_ignore_ascii_case(extension))
                .unwrap_or(false)
        })
        .count()
}

/// Check if a directory is empty
///
/// Determines whether a directory contains any entries (files or subdirectories).
///
/// # Arguments
///
/// * `dir` - Directory path to check
///
/// # Returns
///
/// Returns `true` if:
/// - The directory exists and contains no entries
/// - The directory does not exist
/// - The directory cannot be read (permission denied, I/O error)
///
/// Returns `false` if the directory exists and contains at least one entry.
///
/// # Examples
///
/// ```no_run
/// use std::path::Path;
/// # use generateprevisibines::filesystem::is_directory_empty;
///
/// let data_dir = Path::new("C:\\Games\\Fallout4\\Data");
/// if is_directory_empty(data_dir) {
///     println!("Data directory is empty or inaccessible");
/// } else {
///     println!("Data directory contains files");
/// }
/// ```
///
/// # Notes
///
/// - Non-existent directories are considered "empty" (`true`)
/// - Directories that cannot be read are also considered "empty" (`true`)
/// - Only checks for the presence of entries, not their type (files vs. directories)
pub fn is_directory_empty(dir: &Path) -> bool {
    if !dir.exists() {
        return true;
    }

    match fs::read_dir(dir) {
        Ok(mut entries) => entries.next().is_none(),
        Err(e) => {
            // Log warning to surface permission issues while preserving backwards compatibility
            warn!(
                "Failed to read directory '{}': {}. Treating as empty.",
                dir.display(),
                e
            );
            true
        }
    }
}

/// Delete all files in a directory matching a file extension
///
/// **WARNING: This is a destructive operation.** Recursively searches the directory
/// for files with the specified extension and permanently deletes them. This operation
/// cannot be undone.
///
/// Used for cleaning up old previs/precombined files before regenerating them.
///
/// # Arguments
///
/// * `dir` - Directory to search (recursively)
/// * `extension` - File extension to match (without leading dot, e.g., "nif" not ".nif")
///
/// # Returns
///
/// Returns the number of files successfully deleted. If the directory doesn't exist,
/// returns `Ok(0)`.
///
/// # Errors
///
/// This function will return an error if:
/// - Directory exists but cannot be read (permission denied)
/// - Directory traversal encounters I/O errors
/// - Any file cannot be deleted (file in use, read-only, permission denied)
///
/// **Important:** If deletion fails for any file, the function returns immediately with an error.
/// Some files may have been deleted before the error occurred (partial deletion).
///
/// # Examples
///
/// ```no_run
/// use std::path::Path;
/// # use anyhow::Result;
/// # use generateprevisibines::filesystem::delete_matching_files;
///
/// # fn example() -> Result<()> {
/// // WARNING: This will permanently delete files!
/// let precombined_dir = Path::new("C:\\Games\\Fallout4\\Data\\meshes\\precombined");
/// let deleted_count = delete_matching_files(precombined_dir, "nif")?;
/// println!("Deleted {} .nif files", deleted_count);
/// # Ok(())
/// # }
/// ```
///
/// # Safety Considerations
///
/// - **Always prompt the user before calling this function** in interactive mode
/// - Consider backing up files before deletion
/// - Ensure the correct directory is being targeted
/// - Verify extension parameter is correct (e.g., don't accidentally use "esp" instead of "nif")
/// - Be aware of partial deletion on error - some files may be deleted even if the function fails
///
/// # Notes
///
/// - Extension matching is case-insensitive
/// - Searches recursively through all subdirectories
/// - Non-existent directories return `Ok(0)` without error
#[allow(dead_code)] // Part of public filesystem utility API; available for external use
pub fn delete_matching_files(dir: &Path, extension: &str) -> Result<usize> {
    if !dir.exists() {
        return Ok(0);
    }

    let files = scan_directory_for_files(dir, extension, true)?;
    let count = files.len();

    for file in files {
        fs::remove_file(&file).context(format!("Failed to delete file: {}", file.display()))?;
    }

    Ok(count)
}

/// Get the size of a directory in bytes
#[allow(dead_code)] // Part of public filesystem utility API; available for external use
pub fn get_directory_size(dir: &Path) -> u64 {
    if !dir.exists() {
        return 0;
    }

    WalkDir::new(dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter_map(|e| e.metadata().ok())
        .map(|m| m.len())
        .sum()
}

/// Find xPrevisPatch plugin in the Data directory
///
/// This function scans for plugin files (.esp or .esm) containing "xprevis" in their name
/// (case-insensitive).
///
/// # Arguments
///
/// * `data_dir` - Path to the Fallout 4 Data directory
///
/// # Returns
///
/// Returns a vector of plugin filenames (not full paths) that contain "xprevis".
/// Returns an empty vector if the directory doesn't exist or contains no matching plugins.
///
/// # Errors
///
/// This function will return an error if:
/// - Directory exists but cannot be read (permission denied)
/// - Reading directory entries encounters I/O errors
///
/// # Examples
///
/// ```no_run
/// use std::path::Path;
/// # use anyhow::Result;
/// # use generateprevisibines::filesystem::find_xprevis_patch_plugins;
///
/// # fn example() -> Result<()> {
/// let data_dir = Path::new("C:\\Games\\Fallout4\\Data");
/// let xprevis_plugins = find_xprevis_patch_plugins(data_dir)?;
///
/// if !xprevis_plugins.is_empty() {
///     println!("Found xPrevisPatch plugins:");
///     for plugin in &xprevis_plugins {
///         println!("  - {}", plugin);
///     }
/// }
/// # Ok(())
/// # }
/// ```
///
/// # Notes
///
/// - Matching is case-insensitive (finds "xPrevis", "XPREVIS", etc.)
/// - Only checks .esp and .esm files (not .esl)
/// - Returns filenames only, not full paths
/// - Non-existent directories return `Ok(Vec::new())` without error
pub fn find_xprevis_patch_plugins(data_dir: &Path) -> Result<Vec<String>> {
    if !data_dir.exists() {
        return Ok(Vec::new());
    }

    let mut xprevis_plugins = Vec::new();

    for entry in fs::read_dir(data_dir)? {
        let entry = entry?;
        let path = entry.path();

        if !path.is_file() {
            continue;
        }

        if let Some(file_name) = path.file_name().and_then(|n| n.to_str()) {
            let file_name_lower = file_name.to_lowercase();

            // Check if it's a plugin file and contains xprevispatch
            if (file_name_lower.ends_with(".esp") || file_name_lower.ends_with(".esm"))
                && file_name_lower.contains("xprevispatch")
            {
                xprevis_plugins.push(file_name.to_string());
            }
        }
    }

    Ok(xprevis_plugins)
}

/// Find working files that should be cleaned up after workflow
///
/// During the previs generation workflow, several temporary "working files" are created
/// by CreationKit and other tools. These files should be identified and optionally deleted
/// after the workflow completes to keep the Data directory clean.
///
/// # Working File Patterns
///
/// This function searches for the following files:
/// - `Previs.esp` - Temporary plugin created by CreationKit for previs generation
/// - `PrecombinedObjects.esp` - Temporary plugin for precombined mesh generation
/// - `SeventySix*.esp` - Any plugin starting with "SeventySix" (Fallout 76-related temp files)
///
/// All matching is case-insensitive.
///
/// # Arguments
///
/// * `data_dir` - Path to the Fallout 4 Data directory
///
/// # Returns
///
/// Returns a vector of plugin filenames (not full paths) matching the working file patterns.
/// Returns an empty vector if the directory doesn't exist or contains no matching files.
///
/// # Errors
///
/// This function will return an error if:
/// - Directory exists but cannot be read (permission denied)
/// - Reading directory entries encounters I/O errors
///
/// # Examples
///
/// ```no_run
/// use std::path::Path;
/// # use anyhow::Result;
/// # use generateprevisibines::filesystem::find_working_files;
///
/// # fn example() -> Result<()> {
/// let data_dir = Path::new("C:\\Games\\Fallout4\\Data");
/// let working_files = find_working_files(data_dir)?;
///
/// if !working_files.is_empty() {
///     println!("Found working files that can be cleaned up:");
///     for file in &working_files {
///         println!("  - {}", file);
///     }
/// }
/// # Ok(())
/// # }
/// ```
///
/// # Notes
///
/// - Matching is case-insensitive
/// - Only checks .esp files (not .esm or .esl)
/// - Returns filenames only, not full paths
/// - Non-existent directories return `Ok(Vec::new())` without error
/// - These files are safe to delete after the workflow completes
pub fn find_working_files(data_dir: &Path) -> Result<Vec<String>> {
    if !data_dir.exists() {
        return Ok(Vec::new());
    }

    let mut working_files = Vec::new();

    for entry in fs::read_dir(data_dir)? {
        let entry = entry?;
        let path = entry.path();

        if !path.is_file() {
            continue;
        }

        if let Some(file_name) = path.file_name().and_then(|n| n.to_str()) {
            let file_name_lower = file_name.to_lowercase();

            // Check for working files
            if file_name_lower == "previs.esp" || file_name_lower == "combinedobjects.esp" {
                working_files.push(file_name.to_string());
            }
        }
    }

    Ok(working_files)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use tempfile::TempDir;

    #[test]
    fn test_ensure_output_directories() {
        let temp_dir = TempDir::new().unwrap();
        let data_dir = temp_dir.path().join("Data");
        fs::create_dir(&data_dir).unwrap();

        let result = ensure_output_directories(&data_dir);
        assert!(result.is_ok());

        let (precombined, vis) = result.unwrap();
        assert!(precombined.exists());
        assert!(vis.exists());
    }

    #[test]
    fn test_scan_directory_for_files() {
        let temp_dir = TempDir::new().unwrap();

        // Create test files
        File::create(temp_dir.path().join("test1.esp")).unwrap();
        File::create(temp_dir.path().join("test2.esp")).unwrap();
        File::create(temp_dir.path().join("test3.txt")).unwrap();

        let esp_files = scan_directory_for_files(temp_dir.path(), "esp", false).unwrap();
        assert_eq!(esp_files.len(), 2);

        let txt_files = scan_directory_for_files(temp_dir.path(), "txt", false).unwrap();
        assert_eq!(txt_files.len(), 1);
    }

    #[test]
    fn test_count_files() {
        let temp_dir = TempDir::new().unwrap();

        File::create(temp_dir.path().join("test1.nif")).unwrap();
        File::create(temp_dir.path().join("test2.nif")).unwrap();
        File::create(temp_dir.path().join("test3.nif")).unwrap();

        let count = count_files(temp_dir.path(), "nif");
        assert_eq!(count, 3);
    }

    #[test]
    fn test_is_directory_empty() {
        let temp_dir = TempDir::new().unwrap();
        assert!(is_directory_empty(temp_dir.path()));

        File::create(temp_dir.path().join("test.txt")).unwrap();
        assert!(!is_directory_empty(temp_dir.path()));
    }
}
