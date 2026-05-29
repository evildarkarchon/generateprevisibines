//! Tool discovery via current directory and Windows registry (batch lines 24–41).

use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

const FO4EDIT_CANDIDATES: &[&str] = &["FO4Edit64.exe", "xEdit64.exe", "FO4Edit.exe", "xEdit.exe"];

/// Discovered external tool paths.
#[derive(Debug, Clone, Default)]
pub struct ToolPaths {
    pub fo4edit: Option<PathBuf>,
    pub fallout4_dir: Option<PathBuf>,
    pub creation_kit: Option<PathBuf>,
    pub archive2: Option<PathBuf>,
    pub bsarch: Option<PathBuf>,
}

/// Resolve FO4Edit from the executable directory first, then registry on Windows.
pub fn discover_fo4edit(exe_dir: &Path) -> Result<PathBuf> {
    for name in FO4EDIT_CANDIDATES {
        let candidate = exe_dir.join(name);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }

    #[cfg(windows)]
    {
        if let Ok(path) = fo4edit_from_registry() {
            if path.is_file() {
                return Ok(path);
            }
        }
    }

    Err(Error::Other(
        "FO4Edit/xEdit directory not found. Run this program from its directory or install xEdit."
            .to_string(),
    ))
}

#[cfg(windows)]
fn fo4edit_from_registry() -> Result<PathBuf> {
    use winreg::enums::HKEY_CLASSES_ROOT;
    use winreg::RegKey;

    let hkcr = RegKey::predef(HKEY_CLASSES_ROOT);
    let key = hkcr.open_subkey("FO4Script\\DefaultIcon")?;
    let path: String = key.get_value("")?;
    Ok(PathBuf::from(path))
}

#[cfg(not(windows))]
fn fo4edit_from_registry() -> Result<PathBuf> {
    Err(Error::Other(
        "registry discovery requires Windows".to_string(),
    ))
}

/// Resolve Fallout 4 install directory from registry on Windows.
#[cfg(windows)]
pub fn discover_fallout4_dir() -> Result<PathBuf> {
    use winreg::enums::HKEY_LOCAL_MACHINE;
    use winreg::RegKey;

    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let key = hklm.open_subkey(r"SOFTWARE\Wow6432Node\Bethesda Softworks\Fallout4")?;
    let path: String = key.get_value("installed path")?;
    Ok(PathBuf::from(path))
}

#[cfg(not(windows))]
pub fn discover_fallout4_dir() -> Result<PathBuf> {
    Err(Error::Other(
        "Fallout 4 registry discovery requires Windows".to_string(),
    ))
}

/// Fill tool paths from install layout.
pub fn discover_tools(exe_dir: &Path, fallout4_override: Option<PathBuf>) -> Result<ToolPaths> {
    let fo4edit = discover_fo4edit(exe_dir).ok();

    let fallout4_dir = match fallout4_override {
        Some(dir) => dir,
        None => discover_fallout4_dir()?,
    };

    let creation_kit = fallout4_dir.join("CreationKit.exe");
    let archive2 = fallout4_dir
        .join("Tools")
        .join("archive2")
        .join("archive2.exe");

    let bsarch = fo4edit
        .as_ref()
        .and_then(|p| p.parent().map(|d| d.join("BSArch.exe")))
        .filter(|p| p.is_file());

    Ok(ToolPaths {
        fo4edit,
        fallout4_dir: Some(fallout4_dir.clone()),
        creation_kit: creation_kit.is_file().then_some(creation_kit),
        archive2: archive2.is_file().then_some(archive2),
        bsarch,
    })
}

/// Product version string for an executable (batch uses PowerShell `VersionInfo`).
#[must_use]
pub fn format_version_line(label: &str, path: &Path, version: Option<&str>) -> String {
    match version {
        Some(v) => format!("Using {label} {} V{v}", path.display()),
        None => format!("Using {label} {}", path.display()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn finds_fo4edit_in_exe_dir() {
        let dir = tempdir().unwrap();
        let exe = dir.path().join("FO4Edit64.exe");
        fs::write(&exe, b"").unwrap();
        let found = discover_fo4edit(dir.path()).unwrap();
        assert_eq!(found, exe);
    }
}
