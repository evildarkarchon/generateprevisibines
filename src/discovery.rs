//! Tool discovery via current directory and Windows registry (batch lines 24–40 for xEdit,
//! 46–50 for the Fallout 4 install path, 67–68 for CK and Archive2, 513 for BSArch).
//!
//! Both registry lookups sit behind the batch's `reg.exe` probe (21–22) and its `RegErr_`
//! guards (38, 49), which skip the registry entirely when `reg.exe` is absent — V2.98's
//! "Support for Wine". This module has no equivalent guard; see issue `13`.

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

/// Resolve `FO4Edit` from the executable directory first, then registry on Windows.
pub fn discover_fo4edit(exe_dir: &Path) -> Result<PathBuf> {
    for name in FO4EDIT_CANDIDATES {
        let candidate = exe_dir.join(name);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }

    #[cfg(windows)]
    {
        if let Ok(path) = fo4edit_from_registry()
            && path.is_file()
        {
            return Ok(path);
        }
    }

    Err(Error::Other(
        "FO4Edit/xEdit directory not found. Run this program from its directory or install xEdit."
            .to_string(),
    ))
}

#[cfg(windows)]
fn fo4edit_from_registry() -> Result<PathBuf> {
    use winreg::RegKey;
    use winreg::enums::HKEY_CLASSES_ROOT;

    let hkcr = RegKey::predef(HKEY_CLASSES_ROOT);
    let key = hkcr.open_subkey("FO4Script\\DefaultIcon")?;
    let path: String = key.get_value("")?;
    Ok(PathBuf::from(clean_default_icon_path(&path)))
}

fn clean_default_icon_path(value: &str) -> String {
    let trimmed = value.trim();
    if let Some(rest) = trimmed.strip_prefix('"')
        && let Some((path, _)) = rest.split_once('"')
    {
        return path.to_string();
    }

    trimmed
        .split(',')
        .next()
        .unwrap_or(trimmed)
        .trim()
        .trim_matches('"')
        .to_string()
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
    use winreg::RegKey;
    use winreg::enums::HKEY_LOCAL_MACHINE;

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

    #[test]
    fn cleans_default_icon_registry_path() {
        assert_eq!(
            clean_default_icon_path(r#""C:\Games\FO4Edit.exe",0"#),
            r"C:\Games\FO4Edit.exe"
        );
        assert_eq!(
            clean_default_icon_path(r#""C:\Path, With Comma\FO4Edit.exe",1"#),
            r"C:\Path, With Comma\FO4Edit.exe"
        );
        assert_eq!(
            clean_default_icon_path(r"C:\Games\FO4Edit.exe,0"),
            r"C:\Games\FO4Edit.exe"
        );
    }
}
