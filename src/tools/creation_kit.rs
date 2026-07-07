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

        if let Some(ck_log) = &ctx.ck_log_path
            && ck_log.is_file()
        {
            std::fs::remove_file(ck_log)?;
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

        if !status.success() {
            tracing::warn!(
                code = ?status.code(),
                "Creation Kit exited with non-zero status; workflow operation postconditions determine success"
            );
        }

        Ok(())
    }
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
}
