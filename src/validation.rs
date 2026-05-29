//! Input and environment validation matching batch V2.96 rules.

use std::path::Path;

use crate::config::{BuildMode, CkpeConfigKind, PluginIdentity};
use crate::error::{Error, Result};

const RESERVED_NAMES: &[&str] = &["previs", "combinedobjects", "xprevispatch"];

const REQUIRED_XEDIT_SCRIPTS: &[(&str, &str)] = &[
    ("Batch_FO4MergePrevisandCleanRefr.pas", "V2.3"),
    ("Batch_FO4MergeCombinedObjectsAndCheck.pas", "V1.5"),
];

/// Plugin token allowed by batch `findstr` check (`^[a-z0-9_\.]*$`, case-insensitive).
pub fn validate_plugin_name_token(name: &str) -> Result<()> {
    if name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
    {
        Ok(())
    } else {
        Err(Error::InvalidPluginName {
            name: name.to_string(),
        })
    }
}

/// Full plugin validation after parsing identity.
pub fn validate_plugin(plugin: &PluginIdentity, build_mode: BuildMode) -> Result<()> {
    if build_mode == BuildMode::Clean && plugin.base_name.contains(' ') {
        return Err(Error::PluginNameContainsSpaces);
    }

    // Batch applies `^[a-z0-9_\.]*$` only to CLI plugin tokens, not interactive names with spaces.
    if !plugin.base_name.contains(' ') {
        validate_plugin_name_token(&plugin.base_name)?;
    }

    for reserved in RESERVED_NAMES {
        if plugin.base_name.eq_ignore_ascii_case(reserved) {
            return Err(Error::ReservedPluginName {
                name: plugin.base_name.clone(),
            });
        }
    }

    Ok(())
}

/// Detect which CKPE config file the install uses (batch lines 81–87).
#[must_use]
pub fn detect_ckpe_config_kind(fallout4_dir: &Path) -> CkpeConfigKind {
    if fallout4_dir.join("CKPE.Fallout4.dll").is_file() {
        return CkpeConfigKind::Toml;
    }
    if fallout4_dir.join("VoltekLib.MemoryManager.dll").is_file() {
        return CkpeConfigKind::Ini;
    }
    CkpeConfigKind::LegacyTestIni
}

/// Parse CK log file setting from config contents (batch `:TrimLog` behavior).
#[must_use]
pub fn parse_ck_log_setting(contents: &str, log_key: &str) -> Option<String> {
    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') || trimmed.starts_with(';') {
            continue;
        }
        let Some((key, value)) = trimmed.split_once('=') else {
            continue;
        };
        if key.trim().eq_ignore_ascii_case(log_key) {
            let log = value
                .split('#')
                .next()
                .unwrap_or(value)
                .trim()
                .trim_matches('\'')
                .trim()
                .to_string();
            if log.is_empty() || log.eq_ignore_ascii_case("none") {
                return None;
            }
            return Some(log);
        }
    }
    None
}

/// Whether increased reference limit is enabled (batch `Findstr` on handle setting).
#[must_use]
pub fn ckpe_handle_limit_enabled(contents: &str, handle_key: &str) -> bool {
    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') || trimmed.starts_with(';') {
            continue;
        }
        let Some((key, value)) = trimmed.split_once('=') else {
            continue;
        };
        if key.trim().eq_ignore_ascii_case(handle_key) {
            return value.trim().eq_ignore_ascii_case("true");
        }
    }
    false
}

/// Validate CKPE config file exists and required settings are present.
pub fn validate_ckpe_config(
    fallout4_dir: &Path,
    contents: &str,
) -> Result<(CkpeConfigKind, String)> {
    let kind = detect_ckpe_config_kind(fallout4_dir);
    let log_key = kind.log_setting_key();
    let handle_key = kind.handle_setting_key();

    let log_file = parse_ck_log_setting(contents, log_key).ok_or_else(|| {
        Error::CkpeConfig(format!(
            "CK logging not set in this ini. To fix, set {log_key}=CK.log in it."
        ))
    })?;

    if !ckpe_handle_limit_enabled(contents, handle_key) {
        tracing::warn!(
            "Increased Reference Limit not enabled, Precombine Step may fail. \
             To fix, set {handle_key}=true in {}.",
            kind.file_name()
        );
    }

    Ok((kind, log_file))
}

/// Check xEdit script exists and contains required version marker.
pub fn validate_xedit_script(
    edit_scripts_dir: &Path,
    script_name: &str,
    required_version: &str,
) -> Result<()> {
    let path = edit_scripts_dir.join(script_name);
    if !path.is_file() {
        return Err(Error::MissingXeditScript(script_name.to_string()));
    }

    let contents = std::fs::read_to_string(&path)?;
    if !contents.contains(required_version) {
        return Err(Error::XeditScriptVersion {
            script: script_name.to_string(),
            required: required_version.to_string(),
        });
    }

    Ok(())
}

/// Validate all required xEdit scripts.
pub fn validate_required_xedit_scripts(edit_scripts_dir: &Path) -> Result<()> {
    for (script, version) in REQUIRED_XEDIT_SCRIPTS {
        validate_xedit_script(edit_scripts_dir, script, version)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::PluginIdentity;

    #[test]
    fn rejects_reserved_plugin_names() {
        let plugin = PluginIdentity::parse("previs");
        let err = validate_plugin(&plugin, BuildMode::Clean).unwrap_err();
        assert!(matches!(err, Error::ReservedPluginName { .. }));
    }

    #[test]
    fn clean_mode_rejects_spaces() {
        let plugin = PluginIdentity::parse("My Mod");
        let err = validate_plugin(&plugin, BuildMode::Clean).unwrap_err();
        assert!(matches!(err, Error::PluginNameContainsSpaces));
    }

    #[test]
    fn filtered_mode_allows_spaces() {
        let plugin = PluginIdentity::parse("My Mod");
        assert!(validate_plugin(&plugin, BuildMode::Filtered).is_ok());
    }

    #[test]
    fn parses_ck_log_setting() {
        let ini = "[Log]\nsOutputFile = 'CK.log'\n";
        assert_eq!(
            parse_ck_log_setting(ini, "sOutputFile").as_deref(),
            Some("CK.log")
        );
    }

    #[test]
    fn detects_handle_limit_flag() {
        let ini = "bBSPointerHandleExtremly=true\n";
        assert!(ckpe_handle_limit_enabled(ini, "bBSPointerHandleExtremly"));
    }
}
