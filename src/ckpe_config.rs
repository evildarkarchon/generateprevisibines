use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

/// CKPE configuration settings we care about
/// IMPORTANT: The bBSPointerHandle setting is REQUIRED for precombine generation
/// (batch script lines 177-185, 216-243)
#[derive(Debug)]
pub struct CKPEConfig {
    /// Path to the configuration file
    pub config_path: PathBuf,

    /// Whether bBSPointerHandleExtremly (or variant) is set to true
    pub pointer_handle_enabled: bool,

    /// Path to Creation Kit log file (if specified)
    pub log_file_path: Option<PathBuf>,

    /// Config file format (TOML, INI, or fallout4_test.ini)
    pub config_type: ConfigType,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ConfigType {
    TOML,       // CreationKitPlatformExtended.toml - highest priority
    INI,        // CreationKitPlatformExtended.ini - second priority
    Fallout4TestINI, // fallout4_test.ini - legacy, lowest priority
}

impl CKPEConfig {
    /// Parse a CKPE configuration file
    /// Priority: .toml > .ini > fallout4_test.ini
    /// This ensures newer config formats take precedence over legacy ones
    pub fn parse(config_path: &Path) -> Result<Self> {
        let content = fs::read_to_string(config_path)
            .context(format!("Failed to read CKPE config: {}", config_path.display()))?;

        // Determine config type based on file extension and name
        // Priority: TOML > INI > Fallout4TestINI (to prefer newer formats)
        let config_type = if config_path.extension().and_then(|e| e.to_str()) == Some("toml") {
            ConfigType::TOML
        } else if config_path.extension().and_then(|e| e.to_str()) == Some("ini") {
            // Check if it's the legacy fallout4_test.ini file
            if config_path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.eq_ignore_ascii_case("fallout4_test.ini"))
                .unwrap_or(false)
            {
                ConfigType::Fallout4TestINI
            } else {
                ConfigType::INI
            }
        } else {
            // Default to INI for unknown extensions
            ConfigType::INI
        };

        let pointer_handle_enabled = Self::check_pointer_handle_setting(&content, config_type);
        let log_file_path = Self::extract_log_file_path(&content, config_type);

        Ok(CKPEConfig {
            config_path: config_path.to_path_buf(),
            pointer_handle_enabled,
            log_file_path,
            config_type,
        })
    }

    /// Check if bBSPointerHandle setting is enabled
    /// The setting name varies:
    /// - bBSPointerHandleExtremly (typo in original CKPE)
    /// - bBSPointerHandleExtremely (fixed spelling)
    /// - bBSPointerHandle (short version)
    fn check_pointer_handle_setting(content: &str, config_type: ConfigType) -> bool {
        let patterns = [
            "bBSPointerHandleExtremly",
            "bBSPointerHandleExtremely",
            "bBSPointerHandle",
        ];

        for line in content.lines() {
            let line_trimmed = line.trim();

            // Skip comments
            if line_trimmed.starts_with(';') || line_trimmed.starts_with('#') {
                continue;
            }

            // Check for any variant of the setting
            for pattern in &patterns {
                match config_type {
                    ConfigType::TOML => {
                        // TOML format: bBSPointerHandle = true
                        if line_trimmed.starts_with(pattern) {
                            if let Some(value) = line_trimmed.split('=').nth(1) {
                                let value_trimmed = value.trim();
                                if value_trimmed.eq_ignore_ascii_case("true")
                                    || value_trimmed == "1"
                                {
                                    return true;
                                }
                            }
                        }
                    }
                    ConfigType::INI | ConfigType::Fallout4TestINI => {
                        // INI format: bBSPointerHandle=1 or bBSPointerHandle=true
                        if line_trimmed.starts_with(pattern) {
                            if let Some(value) = line_trimmed.split('=').nth(1) {
                                let value_trimmed = value.trim();
                                if value_trimmed == "1"
                                    || value_trimmed.eq_ignore_ascii_case("true")
                                {
                                    return true;
                                }
                            }
                        }
                    }
                }
            }
        }

        false
    }

    /// Extract log file path from config
    fn extract_log_file_path(content: &str, _config_type: ConfigType) -> Option<PathBuf> {
        for line in content.lines() {
            let line_trimmed = line.trim();

            // Skip comments
            if line_trimmed.starts_with(';') || line_trimmed.starts_with('#') {
                continue;
            }

            // Look for log file path setting
            if line_trimmed.starts_with("sLogFile") || line_trimmed.starts_with("LogFile") {
                if let Some(value) = line_trimmed.split('=').nth(1) {
                    let path_str = value.trim().trim_matches('"');
                    if !path_str.is_empty() {
                        return Some(PathBuf::from(path_str));
                    }
                }
            }
        }

        None
    }

    /// Validate that required settings are present
    pub fn validate(&self) -> Result<()> {
        if !self.pointer_handle_enabled {
            anyhow::bail!(
                "CKPE configuration error: bBSPointerHandleExtremly is not set to true\n\
                \n\
                This setting is REQUIRED for precombine generation.\n\
                \n\
                Please edit: {}\n\
                \n\
                Add or modify this line in the [CreationKit] section:\n\
                bBSPointerHandleExtremly=1\n\
                \n\
                Note: The setting name has a typo ('Extremly' not 'Extremely') - this is intentional.",
                self.config_path.display()
            );
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;
    use tempfile::TempDir;

    #[test]
    fn test_parse_toml_config() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("CreationKitPlatformExtended.toml");

        let mut file = File::create(&config_path).unwrap();
        writeln!(file, "[CreationKit]").unwrap();
        writeln!(file, "bBSPointerHandleExtremly = true").unwrap();
        writeln!(file, "sLogFile = \"CK.log\"").unwrap();
        drop(file);

        let config = CKPEConfig::parse(&config_path).unwrap();
        assert!(config.pointer_handle_enabled);
        assert_eq!(config.config_type, ConfigType::TOML);
        assert!(config.log_file_path.is_some());
    }

    #[test]
    fn test_parse_ini_config() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("CreationKitPlatformExtended.ini");

        let mut file = File::create(&config_path).unwrap();
        writeln!(file, "[CreationKit]").unwrap();
        writeln!(file, "bBSPointerHandle=1").unwrap();
        drop(file);

        let config = CKPEConfig::parse(&config_path).unwrap();
        assert!(config.pointer_handle_enabled);
        assert_eq!(config.config_type, ConfigType::INI);
    }

    #[test]
    fn test_fallout4_test_ini() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("fallout4_test.ini");

        let mut file = File::create(&config_path).unwrap();
        writeln!(file, "bBSPointerHandleExtremly=1").unwrap();
        drop(file);

        let config = CKPEConfig::parse(&config_path).unwrap();
        assert!(config.pointer_handle_enabled);
        assert_eq!(config.config_type, ConfigType::Fallout4TestINI);
    }

    #[test]
    fn test_missing_setting() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("CreationKitPlatformExtended.toml");

        let mut file = File::create(&config_path).unwrap();
        writeln!(file, "[CreationKit]").unwrap();
        writeln!(file, "SomeOtherSetting = true").unwrap();
        drop(file);

        let config = CKPEConfig::parse(&config_path).unwrap();
        assert!(!config.pointer_handle_enabled);
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_setting_disabled() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("test.ini");

        let mut file = File::create(&config_path).unwrap();
        writeln!(file, "bBSPointerHandleExtremly=0").unwrap();
        drop(file);

        let config = CKPEConfig::parse(&config_path).unwrap();
        assert!(!config.pointer_handle_enabled);
    }
}
