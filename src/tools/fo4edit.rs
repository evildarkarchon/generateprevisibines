//! FO4Edit / xEdit script runner (`:RunScript` in batch).
//!
//! Required behaviors when implemented:
//! - Build plugin list file in `%TEMP%\Plugins.txt`
//! - Launch with `-fo4 -autoexit -P:... -Script:... -Mod:... -log:...`
//! - Optional `-D:<dir>\Data` when `-FO4` override is set (batch V2.96)
//! - PowerShell / SendInput ENTER for Module Selection dialog
//! - Poll for unattended log, close window, TaskKill fallback
//! - MO2 sync delays (5s, 10s, 15s as in batch)

use std::path::Path;

/// Placeholder for future FO4Edit automation.
#[derive(Debug, Default)]
pub struct Fo4EditOps;

impl Fo4EditOps {
    /// Run an xEdit script (not yet implemented).
    pub fn run_script(
        &self,
        _fo4edit_exe: &Path,
        _script_name: &str,
        _target_plugin: &str,
        _source_plugin: &str,
        _mod_data_dir: Option<&Path>,
    ) -> crate::error::Result<()> {
        Err(crate::error::Error::Other(
            "FO4Edit automation not yet implemented".into(),
        ))
    }
}
