//! Tool discovery via current directory and Windows registry (batch lines 24–40 for xEdit,
//! 46–50 for the Fallout 4 install path, 67–68 for CK and Archive2, 513 for BSArch).
//!
//! Both registry lookups sit behind the batch's `reg.exe` probe (21–22) and its `RegErr_`
//! guards (38, 49), which skip the registry entirely when `reg.exe` is absent — V2.98's
//! "Support for Wine", which the port supports. Rather than probe for `reg.exe` (a `PATH` test
//! that a stub binary passes while still answering nothing), both lookups here treat an
//! unanswerable registry as an empty answer: xEdit falls through to its own not-found error,
//! and Fallout 4 falls through to the missing-directory message at batch line 63.

use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// Executable names `:xEditCheck` probes in the script's own directory, in the batch's own order
/// (V2.98 lines 26–35). Order is a parity contract: an install carrying more than one of these
/// must resolve to the same binary the batch would have picked.
const FO4EDIT_CANDIDATES: &[&str] = &[
    "xFOEdit.exe",
    "FO4Edit64.exe",
    "xEdit64.exe",
    "FO4Edit.exe",
    "xEdit.exe",
];

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

/// Resolve the Fallout 4 install directory from the registry on Windows (batch lines 49–50).
///
/// Returns `None` when the registry cannot answer — the key is missing because
/// `Fallout4Launcher.exe` was never run, or there is no usable registry at all (Wine). Both are
/// the batch's empty `locCreationKit_`, not an error: the caller's missing-directory message
/// carries the remedies.
#[cfg(windows)]
#[must_use]
pub fn discover_fallout4_dir() -> Option<PathBuf> {
    use winreg::RegKey;
    use winreg::enums::HKEY_LOCAL_MACHINE;

    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let value = hklm
        .open_subkey(r"SOFTWARE\Wow6432Node\Bethesda Softworks\Fallout4")
        .and_then(|key| key.get_value::<String, _>("installed path"))
        .ok();

    fallout4_dir_from_registry_value(value)
}

/// Non-Windows counterpart: there is no registry to consult (batch line 49 skips the query).
#[cfg(not(windows))]
#[must_use]
pub fn discover_fallout4_dir() -> Option<PathBuf> {
    // A skipped query and an unanswerable one are the same empty result, so both go through the
    // same interpretation rather than each inventing its own notion of "absent".
    fallout4_dir_from_registry_value(None)
}

/// Interpret the raw `installed path` registry value, if the query produced one at all.
///
/// A missing value and a blank one collapse to the same `None`: the batch's `WHERE /Q reg.exe`
/// probe (line 21) only proves the binary is on `PATH`, so a stub `reg.exe` answers with nothing
/// useful and must not be mistaken for a real install directory.
fn fallout4_dir_from_registry_value(value: Option<String>) -> Option<PathBuf> {
    let value = value?;
    let trimmed = value.trim();

    (!trimmed.is_empty()).then(|| PathBuf::from(trimmed))
}

/// Fill tool paths from install layout.
///
/// Never fails: an undiscoverable Fallout 4 directory yields `None` here and is reported by
/// toolchain preparation, mirroring the batch's fall-through to its line-63 check.
#[must_use]
pub fn discover_tools(exe_dir: &Path, fallout4_override: Option<PathBuf>) -> ToolPaths {
    let fo4edit = discover_fo4edit(exe_dir).ok();

    // `-FO4:<dir>` wins outright, so an explicit override never consults the registry.
    let fallout4_dir = fallout4_override.or_else(discover_fallout4_dir);

    let creation_kit = fallout4_dir
        .as_ref()
        .map(|dir| dir.join("CreationKit.exe"))
        .filter(|path| path.is_file());
    let archive2 = fallout4_dir
        .as_ref()
        .map(|dir| dir.join("Tools").join("archive2").join("archive2.exe"))
        .filter(|path| path.is_file());

    let bsarch = fo4edit
        .as_ref()
        .and_then(|p| p.parent().map(|d| d.join("BSArch.exe")))
        .filter(|p| p.is_file());

    ToolPaths {
        fo4edit,
        fallout4_dir,
        creation_kit,
        archive2,
        bsarch,
    }
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

    /// `:xEditCheck` probes `xFOEdit.exe` (batch line 26) before `FO4Edit64.exe` (line 28), so a
    /// directory holding both must resolve to `xFOEdit.exe` — the candidate order is a parity
    /// contract with the batch, not an incidental list ordering.
    #[test]
    fn prefers_xfoedit_over_fo4edit64() {
        let dir = tempdir().unwrap();
        let preferred = dir.path().join("xFOEdit.exe");
        fs::write(&preferred, b"").unwrap();
        fs::write(dir.path().join("FO4Edit64.exe"), b"").unwrap();
        let found = discover_fo4edit(dir.path()).unwrap();
        assert_eq!(found, preferred);
    }

    /// Batch lines 21–22 and 49: `WHERE /Q reg.exe` only proves the binary is on `PATH`, so a
    /// Wine prefix with a stub `reg.exe` passes the probe and still answers nothing. "Query
    /// returned nothing" must therefore be treated exactly like "no registry at all".
    #[test]
    fn treats_blank_registry_value_as_no_fallout4_dir() {
        assert_eq!(fallout4_dir_from_registry_value(None), None);
        assert_eq!(fallout4_dir_from_registry_value(Some(String::new())), None);
        assert_eq!(
            fallout4_dir_from_registry_value(Some("   ".to_string())),
            None
        );
        assert_eq!(
            fallout4_dir_from_registry_value(Some(r"C:\Games\Fallout 4\".to_string())),
            Some(PathBuf::from(r"C:\Games\Fallout 4\"))
        );
    }

    /// Batch lines 49–50: a host that cannot answer the registry query leaves `locCreationKit_`
    /// empty and carries on to the line-63 check. `discover_tools` returning `ToolPaths` rather
    /// than `Result` makes that unfailable by construction; what this pins is the consequence —
    /// an unknown directory derives no tools from it.
    #[test]
    fn absent_registry_yields_no_fallout4_dir_rather_than_an_error() {
        let dir = tempdir().unwrap();

        let tools = discover_tools(dir.path(), None);

        // On a developer machine with Fallout 4 installed the registry does answer, so the
        // directory itself cannot be asserted on every host.
        if tools.fallout4_dir.is_none() {
            assert!(tools.creation_kit.is_none());
            assert!(tools.archive2.is_none());
        }
    }

    /// An explicit `-FO4:<dir>` never consults the registry, so the override's own layout is
    /// what gets probed — the batch reaches line 63 with `locCreationKit_` already set.
    #[test]
    fn override_supplies_fallout4_dir_without_the_registry() {
        let dir = tempdir().unwrap();
        let fo4 = dir.path().join("Fallout 4");
        fs::create_dir_all(&fo4).unwrap();
        fs::write(fo4.join("CreationKit.exe"), b"").unwrap();

        let tools = discover_tools(dir.path(), Some(fo4.clone()));

        assert_eq!(tools.fallout4_dir, Some(fo4.clone()));
        assert_eq!(tools.creation_kit, Some(fo4.join("CreationKit.exe")));
        assert_eq!(tools.archive2, None);
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
