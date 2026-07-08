//! Creation Kit invocation (`:RunCK` in batch).

use std::process::Command;

use crate::error::Result;
use crate::logging;
use crate::timing::{self, MO2_DELAY_AFTER_CK_SECS};
use crate::tools::ToolContext;
use crate::tools::dll::DllGuard;

/// Creation Kit command-line operations from the batch workflow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
        operation: CkOperation,
        plugin_file: &str,
        qualifiers: &str,
    ) -> Result<()> {
        let _dll_guard = DllGuard::disable(&ctx.fallout4_dir)?;

        if ctx.ck_log_path.is_file() {
            std::fs::remove_file(&ctx.ck_log_path)?;
        }

        let arg = operation_arg(operation, plugin_file);
        let mut command = Command::new(&ctx.creation_kit);
        command.current_dir(&ctx.fallout4_dir).arg(&arg);
        for qualifier in qualifier_args(qualifiers) {
            command.arg(qualifier);
        }
        let status = command.status()?;

        timing::mo2_sync_delay(MO2_DELAY_AFTER_CK_SECS);

        if let Some(session) = &ctx.session_log {
            logging::append_ck_log(session, &ctx.ck_log_path)?;
        }

        if !status.success() {
            tracing::warn!(
                code = ?status.code(),
                "Creation Kit exited with non-zero status; workflow operation postconditions determine success"
            );
        }

        Ok(())
    }
}

fn operation_arg(operation: CkOperation, plugin_file: &str) -> String {
    format!("-{}:\"{plugin_file}\"", operation.flag())
}

fn qualifier_args(qualifiers: &str) -> impl Iterator<Item = &str> {
    qualifiers.split_whitespace()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ck_flag_format() {
        assert_eq!(
            CkOperation::GeneratePrecombined.flag(),
            "GeneratePrecombined"
        );
    }

    #[test]
    fn qualifier_args_split_batch_words() {
        assert_eq!(
            qualifier_args("filtered all").collect::<Vec<_>>(),
            vec!["filtered", "all"]
        );
        assert_eq!(
            qualifier_args("  clean   all  ").collect::<Vec<_>>(),
            vec!["clean", "all"]
        );
        assert!(qualifier_args("").collect::<Vec<_>>().is_empty());
    }

    #[test]
    fn operation_arg_preserves_plugin_name_as_one_argument() {
        assert_eq!(
            operation_arg(CkOperation::GeneratePrecombined, "My Mod.esp"),
            "-GeneratePrecombined:\"My Mod.esp\""
        );
    }
}
