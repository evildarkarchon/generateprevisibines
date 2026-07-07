//! Creation Kit invocation (`:RunCK` in batch).

use std::process::Command;

use crate::config::ProjectConfig;
use crate::error::{Error, Result};
use crate::logging;
use crate::timing::{self, MO2_DELAY_AFTER_CK_SECS};
use crate::tools::ToolContext;
use crate::tools::dll::DllGuard;

/// Creation Kit command-line operations from the batch workflow.
#[derive(Debug, Clone, Copy)]
pub enum CkOperation {
    GeneratePrecombined,
    CompressPsg,
    BuildCdx,
    GeneratePreVisData,
}

impl CkOperation {
    #[must_use]
    pub const fn flag(self) -> &'static str {
        match self {
            Self::GeneratePrecombined => "GeneratePrecombined",
            Self::CompressPsg => "CompressPSG",
            Self::BuildCdx => "BuildCDX",
            Self::GeneratePreVisData => "GeneratePreVisData",
        }
    }
}

/// Creation Kit process launcher with DLL guard and MO2 sync delay.
#[derive(Debug, Default)]
pub struct CreationKitOps;

impl CreationKitOps {
    /// Run a CK operation (`START /wait` equivalent).
    pub fn run(
        &self,
        ctx: &ToolContext,
        config: &ProjectConfig,
        operation: CkOperation,
        plugin_file: &str,
        qualifiers: &str,
    ) -> Result<()> {
        let _dll_guard = DllGuard::disable(&ctx.fallout4_dir)?;

        if let Some(ck_log) = &ctx.ck_log_path {
            if ck_log.is_file() {
                std::fs::remove_file(ck_log)?;
            }
        }

        let arg = format!("-{}:\"{plugin_file}\"", operation.flag());
        let status = Command::new(&ctx.creation_kit)
            .current_dir(&ctx.fallout4_dir)
            .arg(&arg)
            .arg(qualifiers)
            .status()?;

        timing::mo2_sync_delay(MO2_DELAY_AFTER_CK_SECS);

        if let (Some(session), Some(ck_log)) = (&ctx.session_log, &ctx.ck_log_path) {
            logging::append_ck_log(session, ck_log)?;
        }

        let combined = config.fo4edit_data_dir().join("CombinedObjects.esp");
        if !combined.is_file() {
            return Err(Error::MissingCombinedObjects);
        }

        if !status.success() {
            tracing::warn!(
                code = ?status.code(),
                "Creation Kit exited with non-zero status but CombinedObjects.esp exists"
            );
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::checks::precombine_qualifiers;
    use crate::config::{ArchiveTool, BuildMode, PluginIdentity};

    #[test]
    fn qualifier_strings_match_batch() {
        assert_eq!(precombine_qualifiers(BuildMode::Clean), "clean all");
        assert_eq!(precombine_qualifiers(BuildMode::Filtered), "filtered all");
    }

    #[test]
    fn ck_flag_format() {
        assert_eq!(
            CkOperation::GeneratePrecombined.flag(),
            "GeneratePrecombined"
        );
    }

    fn sample_config() -> ProjectConfig {
        ProjectConfig {
            build_mode: BuildMode::Clean,
            archive_tool: ArchiveTool::Archive2,
            fallout4_dir: Path::new("C:\\Fallout4").to_path_buf(),
            plugin: PluginIdentity::parse("TestMod"),
            non_interactive: true,
            resume_from: None,
            fo4edit_path: None,
            xedit_data_dir: None,
            ck_log_path: None,
        }
    }

    #[test]
    fn combined_objects_path_under_data() {
        let config = sample_config();
        let combined = config.fo4edit_data_dir().join("CombinedObjects.esp");
        assert!(combined.to_string_lossy().contains("CombinedObjects.esp"));
    }
}
