use anyhow::{bail, Result};

/// Reserved plugin name patterns that are forbidden
/// These match the batch script lines 147-154
const RESERVED_NAMES: &[&str] = &["previs", "combinedobjects", "xprevispatch"];

/// Validate plugin name according to rules from batch script
///
/// Rules (from batch lines 134-158):
/// 1. Cannot be empty
/// 2. Must end with .esp or .esm
/// 3. Cannot contain reserved names (previs, combinedobjects, xprevispatch)
/// 4. In clean mode, cannot contain spaces
pub fn validate_plugin_name(name: &str, clean_mode: bool) -> Result<()> {
    if name.is_empty() {
        bail!("Plugin name cannot be empty");
    }

    // Check file extension
    let name_lower = name.to_lowercase();
    if !name_lower.ends_with(".esp") && !name_lower.ends_with(".esm") {
        bail!("Plugin name must end with .esp or .esm");
    }

    // Check for reserved names (case insensitive)
    for reserved in RESERVED_NAMES {
        if name_lower.contains(reserved) {
            bail!(
                "Plugin name cannot contain reserved word '{}'\n\
                Reserved names: previs, combinedobjects, xprevispatch",
                reserved
            );
        }
    }

    // In clean mode, check for spaces (batch line 155)
    if clean_mode && name.contains(' ') {
        bail!(
            "Plugin name cannot contain spaces in clean mode.\n\
            Please rename your plugin or use -filtered mode instead."
        );
    }

    Ok(())
}

/// Extract the plugin name without extension
#[allow(dead_code)] // Will be used in later workflow steps
pub fn get_plugin_base_name(name: &str) -> &str {
    name.trim_end_matches(".esp")
        .trim_end_matches(".esm")
        .trim_end_matches(".ESP")
        .trim_end_matches(".ESM")
}

/// Check if a plugin file exists in the Data directory
pub fn plugin_exists(data_dir: &std::path::Path, plugin_name: &str) -> bool {
    let plugin_path = data_dir.join(plugin_name);
    plugin_path.exists() && plugin_path.is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_plugin_names() {
        assert!(validate_plugin_name("MyMod.esp", true).is_ok());
        assert!(validate_plugin_name("MyMod.esm", true).is_ok());
        assert!(validate_plugin_name("My_Mod_123.esp", true).is_ok());
    }

    #[test]
    fn test_invalid_extensions() {
        assert!(validate_plugin_name("MyMod.txt", true).is_err());
        assert!(validate_plugin_name("MyMod", true).is_err());
        assert!(validate_plugin_name("MyMod.esl", true).is_err());
    }

    #[test]
    fn test_reserved_names() {
        assert!(validate_plugin_name("previs.esp", true).is_err());
        assert!(validate_plugin_name("Previs.esp", true).is_err());
        assert!(validate_plugin_name("MyPrevis.esp", true).is_err());
        assert!(validate_plugin_name("combinedobjects.esp", true).is_err());
        assert!(validate_plugin_name("xprevispatch.esp", true).is_err());
    }

    #[test]
    fn test_spaces_in_clean_mode() {
        // Spaces not allowed in clean mode
        assert!(validate_plugin_name("My Mod.esp", true).is_err());

        // Spaces allowed in filtered mode
        assert!(validate_plugin_name("My Mod.esp", false).is_ok());
    }

    #[test]
    fn test_get_plugin_base_name() {
        assert_eq!(get_plugin_base_name("MyMod.esp"), "MyMod");
        assert_eq!(get_plugin_base_name("MyMod.esm"), "MyMod");
        assert_eq!(get_plugin_base_name("MyMod.ESP"), "MyMod");
        assert_eq!(get_plugin_base_name("My_Mod_123.esp"), "My_Mod_123");
    }
}
