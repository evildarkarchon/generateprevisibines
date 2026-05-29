//! Step 1 — generate precombines via Creation Kit (`:Precomb`, `:Precomb1`, `:Precomb2`).

use crate::checks::{self, has_precombined_nifs};
use crate::config::{ProjectConfig, WorkflowStep};
use crate::error::{Error, Result};
use crate::interactive;
use crate::tools::creation_kit::{CkOperation, CreationKitOps};
use crate::tools::ToolContext;

/// Run Step 1 including preamble, CK, and post-checks.
pub fn run_generate_precombines(
    config: &ProjectConfig,
    ctx: &ToolContext,
    ck: &CreationKitOps,
) -> Result<()> {
    maybe_clear_precombined_on_resume(config)?;

    checks::run_precomb_preamble(config)?;

    let qualifiers = checks::precombine_qualifiers(config.build_mode);
    ck.run(
        ctx,
        config,
        CkOperation::GeneratePrecombined,
        &config.plugin.file_name,
        qualifiers,
    )?;

    let ck_log = ctx
        .ck_log_path
        .clone()
        .ok_or_else(|| Error::Other("CK log path not configured".into()))?;

    checks::run_post_precomb_checks(config, &ck_log)?;

    Ok(())
}

/// `:RePrecomb` — when resuming at step 1 with existing precombined meshes.
fn maybe_clear_precombined_on_resume(config: &ProjectConfig) -> Result<()> {
    let resume_step1 = config
        .resume_from
        .is_none_or(|s| s == WorkflowStep::GeneratePrecombines);

    if !resume_step1 {
        return Ok(());
    }

    let precombined = config.precombined_dir();
    if !has_precombined_nifs(&precombined) {
        return Ok(());
    }

    if config.non_interactive {
        return Err(Error::PrecombinedMeshesExist);
    }

    if !interactive::prompt_clear_precombined(&precombined)? {
        return Err(Error::Other(
            "precombined meshes not cleared — choose another resume step".into(),
        ));
    }

    Ok(())
}
