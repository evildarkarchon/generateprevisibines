use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use winreg::RegKey;
use winreg::enums::{HKEY_CLASSES_ROOT, HKEY_LOCAL_MACHINE};

/// Find `FO4Edit` path by checking multiple locations
///
/// Searches for FO4Edit.exe in the following order:
/// 1. Current working directory (`./FO4Edit.exe`)
/// 2. Windows Registry key `HKCR\FO4Script\DefaultIcon` (set by `FO4Edit` installer)
///
/// The registry value contains a path in the format `"C:\Path\To\FO4Edit.exe,0"`
/// which is parsed to extract the executable path.
///
/// # Returns
///
/// Returns the full path to `FO4Edit.exe` if found
///
/// # Errors
///
/// This function will return an error if:
/// - FO4Edit.exe is not in the current directory AND registry key doesn't exist
/// - Registry key exists but cannot be read (access denied)
/// - Registry value is in an unexpected format (missing comma separator)
/// - Path from registry does not exist on disk (stale installation)
///
/// # Examples
///
/// ```no_run
/// let fo4edit = find_fo4edit_path()?;
/// println!("Found FO4Edit at: {}", fo4edit.display());
/// # Ok::<(), anyhow::Error>(())
/// ```
///
/// # Platform Support
///
/// **Windows only.** This function uses Windows Registry APIs.
pub fn find_fo4edit_path() -> Result<PathBuf> {
    // First check current directory
    let current_dir = std::env::current_dir()?;
    let fo4edit_exe = current_dir.join("FO4Edit.exe");
    if fo4edit_exe.exists() {
        return Ok(fo4edit_exe);
    }

    // Check registry HKCR\FO4Script\DefaultIcon
    // This key contains a path like "C:\Path\To\FO4Edit.exe,0"
    let hkcr = RegKey::predef(HKEY_CLASSES_ROOT);
    let fo4script = hkcr
        .open_subkey("FO4Script\\DefaultIcon")
        .context("FO4Edit not found. Registry key HKCR\\FO4Script\\DefaultIcon not found")?;

    let icon_path: String = fo4script
        .get_value("")
        .context("Could not read DefaultIcon value")?;

    // The registry value contains the path followed by ",0" - strip that
    let path = icon_path
        .split(',')
        .next()
        .context("Invalid FO4Edit registry path format")?
        .trim_matches('"');

    let fo4edit_path = PathBuf::from(path);
    if !fo4edit_path.exists() {
        anyhow::bail!("FO4Edit path from registry does not exist: {path}");
    }

    Ok(fo4edit_path)
}

/// Find Fallout 4 installation directory from Windows Registry
///
/// Reads the installation path from the registry key created by the Fallout 4 installer:
/// `HKLM\SOFTWARE\Wow6432Node\Bethesda Softworks\Fallout4` with value `"Installed Path"`.
///
/// This is the standard location for 64-bit installations on 64-bit Windows (`WOW6432Node`).
///
/// # Returns
///
/// Returns the full path to the Fallout 4 installation directory (e.g., `C:\Program Files (x86)\Steam\steamapps\common\Fallout 4`)
///
/// # Errors
///
/// This function will return an error if:
/// - Fallout 4 is not installed (registry key doesn't exist)
/// - Registry key exists but cannot be read (insufficient permissions)
/// - Registry value `"Installed Path"` is missing or empty
/// - Path from registry does not exist on disk (uninstalled but registry not cleaned)
///
/// # Examples
///
/// ```no_run
/// let fo4_dir = find_fo4_directory()?;
/// println!("Fallout 4 installed at: {}", fo4_dir.display());
/// # Ok::<(), anyhow::Error>(())
/// ```
///
/// # Platform Support
///
/// **Windows only.** Requires Windows Registry access.
///
/// # See Also
///
/// Use the `--FO4` command-line argument to override this auto-detection.
pub fn find_fo4_directory() -> Result<PathBuf> {
    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let fo4_key = hklm
        .open_subkey("SOFTWARE\\Wow6432Node\\Bethesda Softworks\\Fallout4")
        .context("Fallout 4 not found in registry. Is it installed?")?;

    let install_path: String = fo4_key
        .get_value("Installed Path")
        .context("Could not read Fallout 4 installation path from registry")?;

    let fo4_dir = PathBuf::from(install_path.trim_matches('"'));
    if !fo4_dir.exists() {
        anyhow::bail!(
            "Fallout 4 directory from registry does not exist: {}",
            fo4_dir.display()
        );
    }

    Ok(fo4_dir)
}

/// Find Creation Kit executable in the Fallout 4 directory
///
/// Searches for `CreationKit.exe` in the specified Fallout 4 installation directory.
/// The Creation Kit is Bethesda's official level editor for Fallout 4, required for
/// generating previs and precombined data.
///
/// # Arguments
///
/// * `fo4_dir` - Path to the Fallout 4 installation directory
///
/// # Returns
///
/// Returns the full path to `CreationKit.exe` (e.g., `C:\Program Files (x86)\Steam\steamapps\common\Fallout 4\CreationKit.exe`)
///
/// # Errors
///
/// This function will return an error if:
/// - `CreationKit.exe` does not exist in the specified directory (not installed or wrong path)
///
/// # Examples
///
/// ```no_run
/// use std::path::PathBuf;
///
/// let fo4_dir = PathBuf::from("C:\\Program Files (x86)\\Steam\\steamapps\\common\\Fallout 4");
/// let ck_path = find_creation_kit(&fo4_dir)?;
/// println!("Creation Kit found at: {}", ck_path.display());
/// # Ok::<(), anyhow::Error>(())
/// ```
///
/// # Notes
///
/// - The Creation Kit is a separate download from Bethesda and is not included with the base game
/// - Some Fallout 4 installations may not have the Creation Kit installed
pub fn find_creation_kit(fo4_dir: &Path) -> Result<PathBuf> {
    let ck_path = fo4_dir.join("CreationKit.exe");
    if !ck_path.exists() {
        anyhow::bail!(
            "CreationKit.exe not found in Fallout 4 directory: {}",
            fo4_dir.display()
        );
    }
    Ok(ck_path)
}

/// Find Archive2.exe in the Fallout 4 directory
///
/// Searches for Bethesda's Archive2 tool in two standard locations within the
/// Fallout 4 installation. Archive2 is used to create and manipulate BA2 archive files
/// (Bethesda Archive v2 format).
///
/// The search order is:
/// 1. `Tools/Archive2/Archive2.exe` (official installation location)
/// 2. `Archive2.exe` (root FO4 directory, for custom installations)
///
/// # Arguments
///
/// * `fo4_dir` - Path to the Fallout 4 installation directory
///
/// # Returns
///
/// Returns the full path to `Archive2.exe` if found in either location
///
/// # Errors
///
/// This function will return an error if:
/// - `Archive2.exe` is not found in either the Tools directory or root FO4 directory
/// - The Creation Kit tools are not installed (Archive2 is part of the CK installation)
///
/// # Examples
///
/// ```no_run
/// use std::path::PathBuf;
///
/// let fo4_dir = PathBuf::from("C:\\Program Files (x86)\\Steam\\steamapps\\common\\Fallout 4");
/// let archive2_path = find_archive2(&fo4_dir)?;
/// println!("Archive2 found at: {}", archive2_path.display());
/// # Ok::<(), anyhow::Error>(())
/// ```
///
/// # Notes
///
/// - Archive2.exe is included with the Creation Kit, not the base game
/// - Some users manually copy Archive2.exe to the root FO4 directory for convenience
/// - Archive2 has NO append functionality (must extract, add files, and re-archive)
pub fn find_archive2(fo4_dir: &Path) -> Result<PathBuf> {
    let archive2_path = fo4_dir.join("Tools").join("Archive2").join("Archive2.exe");
    if archive2_path.exists() {
        return Ok(archive2_path);
    }

    // Also check in the root FO4 directory
    let archive2_root = fo4_dir.join("Archive2.exe");
    if archive2_root.exists() {
        return Ok(archive2_root);
    }

    anyhow::bail!("Archive2.exe not found in Fallout 4 Tools directory")
}

/// Find BSArch.exe archiving tool
///
/// Searches for `BSArch` (Bethesda Archive command-line tool) in multiple locations.
/// `BSArch` is an alternative to Archive2 with better append support and is often preferred
/// for automation workflows. Unlike Archive2, `BSArch` can add files to existing archives
/// without extracting and re-archiving.
///
/// The search order is:
/// 1. Current working directory (`./BSArch.exe`)
/// 2. Executable's directory (same folder as this program)
/// 3. Fallout 4 root directory (`<FO4>/BSArch.exe`)
/// 4. Fallout 4 Tools directory (`<FO4>/Tools/BSArch.exe`)
/// 5. Fallout 4 Tools subdirectory (`<FO4>/Tools/BSArch/BSArch.exe`)
///
/// # Arguments
///
/// * `fo4_dir` - Path to the Fallout 4 installation directory
///
/// # Returns
///
/// Returns the full path to `BSArch.exe` if found in any of the search locations
///
/// # Errors
///
/// This function will return an error if:
/// - `BSArch.exe` is not found in any of the searched locations
/// - `BSArch` is not installed or not in the expected locations
///
/// # Examples
///
/// ```no_run
/// use std::path::PathBuf;
///
/// let fo4_dir = PathBuf::from("C:\\Program Files (x86)\\Steam\\steamapps\\common\\Fallout 4");
/// let bsarch_path = find_bsarch(&fo4_dir)?;
/// println!("BSArch found at: {}", bsarch_path.display());
/// # Ok::<(), anyhow::Error>(())
/// ```
///
/// # Notes
///
/// - `BSArch` is a third-party tool and is NOT included with Fallout 4 or Creation Kit
/// - Users must download `BSArch` separately from community sources
/// - `BSArch` can be used via the `--bsarch` command-line flag to prefer it over Archive2
/// - `BSArch` supports direct append operations, making it more efficient than Archive2
///
/// # See Also
///
/// Use the `--bsarch` command-line argument to enable `BSArch` mode instead of Archive2
pub fn find_bsarch(fo4_dir: &Path) -> Result<PathBuf> {
    let mut locations = vec![
        // Check current directory first
        std::env::current_dir().ok().map(|p| p.join("BSArch.exe")),
    ];

    // Check executable's directory
    if let Ok(exe_path) = std::env::current_exe()
        && let Some(exe_dir) = exe_path.parent()
    {
        locations.push(Some(exe_dir.join("BSArch.exe")));
    }

    // Check FO4 directory locations
    locations.extend(vec![
        Some(fo4_dir.join("BSArch.exe")),
        Some(fo4_dir.join("Tools").join("BSArch.exe")),
        Some(fo4_dir.join("Tools").join("BSArch").join("BSArch.exe")),
    ]);

    for path in locations.into_iter().flatten() {
        if path.exists() {
            return Ok(path);
        }
    }

    anyhow::bail!(
        "BSArch.exe not found.\n\
        Searched locations:\n\
        - Current directory\n\
        - Executable directory\n\
        - Fallout 4 directory\n\
        - Fallout 4\\Tools directory"
    )
}

/// Find Creation Kit Platform Extended (CKPE) configuration file
///
/// Searches for CKPE configuration files in priority order. Creation Kit Platform Extended
/// is a community patch that fixes bugs and extends the Creation Kit's capabilities.
/// CKPE can use different configuration file formats and names depending on the version.
///
/// The search priority order is:
/// 1. `CreationKitPlatformExtended.toml` (newest format, TOML configuration)
/// 2. `CreationKitPlatformExtended.ini` (newer format, INI configuration)
/// 3. `fallout4_test.ini` (legacy format, used by older CKPE versions)
///
/// # Arguments
///
/// * `fo4_dir` - Path to the Fallout 4 installation directory
///
/// # Returns
///
/// Returns `Some(PathBuf)` with the path to the first configuration file found,
/// or `None` if no CKPE configuration file exists (CKPE not installed).
///
/// # Examples
///
/// ```no_run
/// use std::path::PathBuf;
///
/// let fo4_dir = PathBuf::from("C:\\Program Files (x86)\\Steam\\steamapps\\common\\Fallout 4");
/// if let Some(config_path) = find_ckpe_config(&fo4_dir) {
///     println!("CKPE config found at: {}", config_path.display());
/// } else {
///     println!("CKPE not installed");
/// }
/// ```
///
/// # Notes
///
/// - Returning `None` does not indicate an error; it means CKPE is not installed
/// - The configuration file contains critical settings like `bBSPointerHandleExtremly=true`
///   which is required for precombined workflow to succeed
/// - Different CKPE versions use different file formats, hence the priority search
/// - TOML format is the most recent and preferred configuration format
///
/// # See Also
///
/// - `ckpe_config::check_pointer_handle_setting()` - Validates required CKPE settings
pub fn find_ckpe_config(fo4_dir: &Path) -> Option<PathBuf> {
    let locations = vec![
        fo4_dir.join("CreationKitPlatformExtended.toml"),
        fo4_dir.join("CreationKitPlatformExtended.ini"),
        fo4_dir.join("fallout4_test.ini"),
    ];

    locations.into_iter().find(|path| path.exists())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore] // Requires actual Fallout 4 installation
    fn test_find_fo4_directory() {
        let result = find_fo4_directory();
        assert!(result.is_ok());
        let fo4_dir = result.unwrap();
        assert!(fo4_dir.exists());
    }

    #[test]
    #[ignore] // Requires actual FO4Edit installation
    fn test_find_fo4edit_path() {
        let result = find_fo4edit_path();
        assert!(result.is_ok());
        let fo4edit_path = result.unwrap();
        assert!(fo4edit_path.exists());
    }
}
