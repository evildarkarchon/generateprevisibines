use anyhow::{Context, Result};
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

/// Scan a directory for files matching a pattern
/// Returns list of file paths relative to the search directory
#[allow(dead_code)] // Will be used in later workflow steps
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
#[allow(dead_code)] // Will be used in later workflow steps
pub fn is_directory_empty(dir: &Path) -> bool {
    if !dir.exists() {
        return true;
    }

    fs::read_dir(dir)
        .map(|mut entries| entries.next().is_none())
        .unwrap_or(true)
}

/// Delete all files in a directory matching a pattern
/// Used for cleaning up old previs/precombined files
#[allow(dead_code)] // Will be used in later workflow steps
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
#[allow(dead_code)] // Will be used in later workflow steps
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

/// Find xPrevisPatch plugins in the Data directory
/// Returns list of plugin names that contain "xprevis" (case-insensitive)
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

            // Check if it's a plugin file and contains xprevis
            if (file_name_lower.ends_with(".esp") || file_name_lower.ends_with(".esm"))
                && file_name_lower.contains("xprevis")
            {
                xprevis_plugins.push(file_name.to_string());
            }
        }
    }

    Ok(xprevis_plugins)
}

/// Find working files that should be cleaned up after workflow
/// Returns list of plugin names (Previs.esp, PrecombineObjects.esp, SeventySix*.esp)
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
            if file_name_lower == "previs.esp"
                || file_name_lower == "precombinedobjects.esp"
                || (file_name_lower.starts_with("seventysix") && file_name_lower.ends_with(".esp"))
            {
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
