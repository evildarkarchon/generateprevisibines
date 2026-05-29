//! Session logging to a temp file (batch uses `%TEMP%\%PluginName%.log`).

use std::io::Write;
use std::path::{Path, PathBuf};

use crate::config::PluginIdentity;
use crate::error::Result;

/// Path for the per-run log file.
#[must_use]
pub fn session_log_path(plugin: &PluginIdentity) -> PathBuf {
    let temp = std::env::temp_dir();
    temp.join(format!("{}.log", plugin.base_name))
}

/// Path matching batch unattended xEdit log location.
#[must_use]
pub fn unattended_log_path() -> PathBuf {
    std::env::temp_dir().join("UnattendedScript.log")
}

/// Append a banner line to the session log (creates file if needed).
pub fn append_log_line(path: &Path, line: &str) -> Result<()> {
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(file, "{line}")?;
    Ok(())
}

/// Batch-aligned session log header (`:Precomb` line 247, V2.95 reference).
#[must_use]
pub fn build_session_header(build_mode: &str, plugin_file: &str) -> String {
    format!("Starting {build_mode} Build V2.95 of {plugin_file}")
}

/// Initialize session log with build header (batch `Starting %BuildMode_% Build`).
pub fn init_session_log(path: &Path, build_mode: &str, plugin_file: &str) -> Result<()> {
    let header = build_session_header(build_mode, plugin_file);
    std::fs::write(path, format!("{header}\n"))?;
    Ok(())
}

/// Append Creation Kit log contents to the session log (batch `:RunCK` lines 447–448).
pub fn append_ck_log(session_log: &Path, ck_log: &Path) -> Result<()> {
    if !ck_log.is_file() {
        return Ok(());
    }
    let contents = std::fs::read_to_string(ck_log)?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(session_log)?;
    writeln!(file)?;
    writeln!(file, "----- Creation Kit log -----")?;
    write!(file, "{contents}")?;
    if !contents.ends_with('\n') {
        writeln!(file)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_log_uses_plugin_base_name() {
        let plugin = PluginIdentity::parse("MyMod.esp");
        let path = session_log_path(&plugin);
        assert!(path.to_string_lossy().contains("MyMod.log"));
    }

    #[test]
    fn session_header_matches_batch_version() {
        let header = build_session_header("clean", "MyMod.esp");
        assert!(header.contains("V2.95"));
        assert!(header.contains("MyMod.esp"));
    }
}
