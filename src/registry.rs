use anyhow::{Context, Result};
use std::path::PathBuf;
use winreg::enums::*;
use winreg::RegKey;

/// Find FO4Edit path by checking:
/// 1. Current directory
/// 2. Registry: HKCR\FO4Script\DefaultIcon
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
        anyhow::bail!("FO4Edit path from registry does not exist: {}", path);
    }

    Ok(fo4edit_path)
}

/// Find Fallout 4 installation directory from registry
/// Registry key: HKLM\SOFTWARE\Wow6432Node\Bethesda Softworks\Fallout4
/// Value: "Installed Path"
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

/// Find Creation Kit executable in the FO4 directory
pub fn find_creation_kit(fo4_dir: &PathBuf) -> Result<PathBuf> {
    let ck_path = fo4_dir.join("CreationKit.exe");
    if !ck_path.exists() {
        anyhow::bail!(
            "CreationKit.exe not found in Fallout 4 directory: {}",
            fo4_dir.display()
        );
    }
    Ok(ck_path)
}

/// Find Archive2.exe in the FO4 directory
pub fn find_archive2(fo4_dir: &PathBuf) -> Result<PathBuf> {
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

/// Find BSArch.exe (typically in FO4 directory or Tools)
pub fn find_bsarch(fo4_dir: &PathBuf) -> Result<PathBuf> {
    // Check common locations
    let locations = vec![
        fo4_dir.join("BSArch.exe"),
        fo4_dir.join("Tools").join("BSArch.exe"),
        fo4_dir.join("Tools").join("BSArch").join("BSArch.exe"),
    ];

    for path in locations {
        if path.exists() {
            return Ok(path);
        }
    }

    anyhow::bail!("BSArch.exe not found in Fallout 4 directory")
}

/// Find CKPE configuration file
/// Checks for:
/// 1. CreationKitPlatformExtended.toml (new format)
/// 2. CreationKitPlatformExtended.ini (new format)
/// 3. fallout4_test.ini (old format)
pub fn find_ckpe_config(fo4_dir: &PathBuf) -> Option<PathBuf> {
    let locations = vec![
        fo4_dir.join("CreationKitPlatformExtended.toml"),
        fo4_dir.join("CreationKitPlatformExtended.ini"),
        fo4_dir.join("fallout4_test.ini"),
    ];

    for path in locations {
        if path.exists() {
            return Some(path);
        }
    }

    None
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
