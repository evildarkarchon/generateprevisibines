use anyhow::{Context, Result};
use log::{info, warn};
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Helper for MO2 VFS staging directory operations
///
/// When running in MO2 mode, generated files end up in MO2's VFS staging directory
/// (typically the overwrite folder) rather than the actual Fallout 4 Data directory.
/// The archivers don't know about MO2's VFS, so we need to collect these files
/// manually before archiving.
pub struct Mo2Helper {
    staging_dir: PathBuf,
}

impl Mo2Helper {
    /// Create a new MO2 helper with the given staging directory
    pub fn new(staging_dir: impl AsRef<Path>) -> Result<Self> {
        let staging_dir = staging_dir.as_ref().to_path_buf();

        if !staging_dir.exists() {
            anyhow::bail!(
                "MO2 staging directory does not exist: {}",
                staging_dir.display()
            );
        }

        Ok(Self { staging_dir })
    }

    /// Find and collect precombined meshes from MO2 staging directory
    ///
    /// Searches for `meshes/precombined` directory and copies all files to temp location
    /// while maintaining directory hierarchy.
    ///
    /// Returns the path to the temporary directory containing the collected files,
    /// or None if no files were found.
    pub fn collect_precombines(&self, temp_dir: impl AsRef<Path>) -> Result<Option<PathBuf>> {
        self.collect_files_from_subpath("meshes/precombined", temp_dir)
    }

    /// Find and collect previs data from MO2 staging directory
    ///
    /// Searches for `vis` directory and copies all files to temp location
    /// while maintaining directory hierarchy.
    ///
    /// Returns the path to the temporary directory containing the collected files,
    /// or None if no files were found.
    pub fn collect_previs(&self, temp_dir: impl AsRef<Path>) -> Result<Option<PathBuf>> {
        self.collect_files_from_subpath("vis", temp_dir)
    }

    /// Find and collect files from a specific subpath within the staging directory
    fn collect_files_from_subpath(
        &self,
        subpath: &str,
        temp_dir: impl AsRef<Path>,
    ) -> Result<Option<PathBuf>> {
        let temp_dir = temp_dir.as_ref();

        // Search for the subpath in the staging directory
        let search_path = self.staging_dir.join(subpath);

        if !search_path.exists() {
            info!("MO2: Path not found in staging directory: {subpath}");
            return Ok(None);
        }

        if !search_path.is_dir() {
            warn!("MO2: Path exists but is not a directory: {subpath}");
            return Ok(None);
        }

        // Check if directory has any files
        let has_files = WalkDir::new(&search_path)
            .into_iter()
            .filter_map(std::result::Result::ok)
            .any(|e| e.file_type().is_file());

        if !has_files {
            info!("MO2: No files found in {subpath}");
            return Ok(None);
        }

        // Create temp directory
        if temp_dir.exists() {
            fs::remove_dir_all(temp_dir).with_context(|| {
                format!("Failed to clean temp directory: {}", temp_dir.display())
            })?;
        }
        fs::create_dir_all(temp_dir)
            .with_context(|| format!("Failed to create temp directory: {}", temp_dir.display()))?;

        // Copy files while maintaining directory structure
        info!("MO2: Collecting files from {subpath} to temp location");

        let dest_base = temp_dir.join(subpath);
        fs::create_dir_all(&dest_base)?;

        let mut file_count = 0;

        for entry in WalkDir::new(&search_path) {
            let entry = entry?;
            let path = entry.path();

            if !entry.file_type().is_file() {
                continue;
            }

            // Get relative path from search_path
            let relative_path = path
                .strip_prefix(&search_path)
                .with_context(|| format!("Failed to get relative path for: {}", path.display()))?;

            // Security: Verify the relative path doesn't escape outside the target directory
            // This prevents path traversal attacks via symbolic links or malicious path components
            if relative_path
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
            {
                anyhow::bail!(
                    "Security: Path traversal detected in: {}\n\
                    The file path attempts to escape the staging directory using '..' components.",
                    path.display()
                );
            }

            let dest_path = dest_base.join(relative_path);

            // Create parent directories
            if let Some(parent) = dest_path.parent() {
                fs::create_dir_all(parent)?;
            }

            // Copy file
            fs::copy(path, &dest_path).with_context(|| {
                format!(
                    "Failed to copy {} to {}",
                    path.display(),
                    dest_path.display()
                )
            })?;

            file_count += 1;
        }

        info!("MO2: Collected {file_count} files from {subpath}");
        Ok(Some(temp_dir.to_path_buf()))
    }

    /// Get the staging directory path
    pub fn staging_dir(&self) -> &Path {
        &self.staging_dir
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use tempfile::TempDir;

    #[test]
    fn test_mo2_helper_creation() {
        let temp = TempDir::new().unwrap();
        let helper = Mo2Helper::new(temp.path());
        assert!(helper.is_ok());

        let non_existent = PathBuf::from("Z:\\does\\not\\exist");
        let helper = Mo2Helper::new(non_existent);
        assert!(helper.is_err());
    }

    #[test]
    fn test_collect_files_empty_directory() {
        let staging = TempDir::new().unwrap();
        let temp = TempDir::new().unwrap();

        // Create empty meshes/precombined directory
        let precombined = staging.path().join("meshes").join("precombined");
        fs::create_dir_all(&precombined).unwrap();

        let helper = Mo2Helper::new(staging.path()).unwrap();
        let result = helper.collect_precombines(temp.path()).unwrap();

        // Should return None since no files exist
        assert!(result.is_none());
    }

    #[test]
    fn test_collect_files_with_files() {
        let staging = TempDir::new().unwrap();
        let temp = TempDir::new().unwrap();

        // Create meshes/precombined directory with files
        let precombined = staging.path().join("meshes").join("precombined");
        fs::create_dir_all(&precombined).unwrap();

        // Create test files
        File::create(precombined.join("test1.nif")).unwrap();
        File::create(precombined.join("test2.nif")).unwrap();

        let helper = Mo2Helper::new(staging.path()).unwrap();
        let result = helper.collect_precombines(temp.path()).unwrap();

        // Should return Some with temp directory
        assert!(result.is_some());

        let collected_dir = result.unwrap();
        let expected_file1 = collected_dir
            .join("meshes")
            .join("precombined")
            .join("test1.nif");
        let expected_file2 = collected_dir
            .join("meshes")
            .join("precombined")
            .join("test2.nif");

        assert!(expected_file1.exists());
        assert!(expected_file2.exists());
    }

    #[test]
    fn test_collect_maintains_hierarchy() {
        let staging = TempDir::new().unwrap();
        let temp = TempDir::new().unwrap();

        // Create nested directory structure
        let subdir = staging.path().join("vis").join("subdir1").join("subdir2");
        fs::create_dir_all(&subdir).unwrap();

        File::create(subdir.join("test.uvd")).unwrap();

        let helper = Mo2Helper::new(staging.path()).unwrap();
        let result = helper.collect_previs(temp.path()).unwrap();

        assert!(result.is_some());

        let collected_dir = result.unwrap();
        let expected_file = collected_dir
            .join("vis")
            .join("subdir1")
            .join("subdir2")
            .join("test.uvd");

        assert!(expected_file.exists());
    }
}
