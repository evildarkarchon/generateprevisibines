//! Creation Kit invocation (`:RunCK` in batch).

use std::ffi::OsString;

use crate::error::Result;
use crate::files::FileSpace;
use crate::logging;
use crate::tools::ToolContext;
use crate::tools::dll::DllGuard;
use crate::tools::process::{ProcessRunner, SystemProcessRunner};
use crate::tools::wait::{MO2_DELAY_AFTER_CK_SECS, SystemWait, Wait};

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
    ///
    /// `files` is the space the DLL guard renames through and the Creation Kit log is folded
    /// into the session log through. The stale-log removal between them still goes straight
    /// to `std::fs`; moving it across the seam is the reshape issue's work, not this one's.
    // The receiver is unused only because this adapter still reaches for
    // `SystemProcessRunner`/`SystemWait` directly. Narrowing the method to `pub(crate)` is
    // what exposed it to `unused_self`; it stays a method because the reshape that injects
    // those ports puts them in `self`.
    #[allow(
        clippy::unused_self,
        reason = "the injected process and wait ports become fields of this adapter"
    )]
    pub(crate) fn run(
        &self,
        ctx: &ToolContext,
        operation: CkOperation,
        plugin_file: &str,
        qualifiers: &str,
        files: &dyn FileSpace,
    ) -> Result<()> {
        let _dll_guard = DllGuard::disable(files, &ctx.fallout4_dir)?;

        if ctx.ck_log_path.is_file() {
            std::fs::remove_file(&ctx.ck_log_path)?;
        }

        let mut args = vec![OsString::from(operation_arg(operation, plugin_file))];
        args.extend(qualifier_args(qualifiers).map(OsString::from));
        // Still a direct `SystemProcessRunner` / `SystemWait`: the ports exist, but injecting
        // them is part of reshaping this adapter into a deep module, not of adding them.
        let status = SystemProcessRunner.run(&ctx.creation_kit, &args, &ctx.fallout4_dir)?;

        SystemWait.sync_delay(MO2_DELAY_AFTER_CK_SECS);

        if let Some(session) = &ctx.session_log {
            logging::append_ck_log(session, &ctx.ck_log_path, files)?;
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
