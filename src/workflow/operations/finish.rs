//! Finish: the run epilogue (batch `:Fin` / `:Cleanup`, V2.99 lines 337–358).
//!
//! Finish announces the build, lists the Patch Files by Build Mode, and offers to remove the
//! Working Files. It is **not** a Workflow Operation: it has no Workflow Step, no toolchain
//! requirements and no registration, because it is not a step a run can resume at or stop
//! short of. It is what a run does once every planned step has completed, so
//! `WorkflowRun::execute_with_ports` calls it after the last dispatch and before `See Log at`,
//! and only when the plan runs every planned step. It lives beside the operations because it
//! shares their [`OperationPorts`] and workspaces.

use std::path::Path;

use crate::config::{BuildMode, ProjectConfig};
use crate::logging;
use crate::run::WorkflowRun;
use crate::warning::BuildWarning;

use super::build_cdx::cdx_name;
use super::compress_psg::{geometry_csg_name, geometry_psg_name};
use super::precombine_workspace::PrecombineWorkspace;
use super::previs_workspace::PrevisWorkspace;
use super::{Confirmation, OperationPorts};

/// Run the Finish epilogue for a Workflow Run that completed every planned step.
///
/// Appends `Build of Patch <base> Complete.` to the session log, prints that line and the Patch
/// Files manifest, then removes the Working Files when the operator agrees, or without asking
/// on a non-interactive run.
///
/// Returns nothing, because Finish never stops a run (decided on the Finish spec ticket, #48).
/// Every Patch File is already built by the time it starts, so nothing that goes wrong here may
/// turn a finished build into a failed one or change its exit code 0. A failed session-log
/// append is therefore only a console warning — a Finish-only exception to the convention that
/// it is an `Io` error — a failed delete is a Build Warning, and a question that cannot be asked
/// keeps the files.
pub(crate) fn finish(run: &WorkflowRun, ports: &OperationPorts<'_>) {
    let config = run.config();

    let complete = logging::build_complete_line(&config.plugin.base_name);
    if let Err(error) = logging::append_log_line(run.log_path(), &complete, ports.files) {
        tracing::warn!(
            "Could not record the completed build in the session log {}: {error}",
            run.log_path().display()
        );
    }

    // `println!` rather than `tracing`, as `main`'s banner is: its prefixes would break the
    // batch's text.
    println!("{}", patch_files_report(config));

    if !consents_to_remove_working_files(config, ports) {
        return;
    }

    // Each path from the workspace that owns it, so Finish spells neither file name itself.
    // `CombinedObjects.esp` first, then `Previs.esp` (357, 358). No MO2 wait first: the batch
    // has none, and nothing launched since the last step's own wait.
    let working_files = [
        PrecombineWorkspace::new(config, ports.files).combined_objects_path(),
        PrevisWorkspace::new(config, ports.files).previs_plugin_path(),
    ];
    for path in working_files {
        remove_working_file(&path, ports);
    }
}

/// Whether the Working Files should be removed (batch 354–356).
///
/// A non-interactive run asks nothing and removes them, as the batch's `NoPrompt_` jumps
/// straight to `:Cleanup` (354): an unattended run must never block, and it cleans up as the
/// batch does. A refusal says nothing, as the batch's `goto Done` (356) does.
fn consents_to_remove_working_files(config: &ProjectConfig, ports: &OperationPorts<'_>) -> bool {
    if config.non_interactive {
        return true;
    }

    match ports.prompts.confirm(&Confirmation::RemoveWorkingFiles) {
        Ok(consented) => consented,
        // Keep the files: deleting them without an answer would be worse than leaving them,
        // and Finish never stops the run over it.
        Err(error) => {
            tracing::warn!(
                "Could not ask whether to remove the working files, so kept them: {error}"
            );
            false
        }
    }
}

/// Delete one Working File if it exists (batch `If Exist … DEL`, 357–358).
///
/// A file that is already gone, as after a resume that never created it, is skipped silently.
/// One that cannot be deleted raises [`BuildWarning::WorkingFileNotRemoved`], a divergence from
/// the batch, whose failed `DEL` means nothing to its exit code. The I/O error itself is
/// dropped, as the Archive episode's cleanup warnings drop theirs: the warning names the path,
/// which is what the operator acts on.
fn remove_working_file(path: &Path, ports: &OperationPorts<'_>) {
    if !ports.files.exists(path) || ports.files.remove_file(path).is_ok() {
        return;
    }

    // The collector keeps and prints the warning before it appends, so a failed append loses
    // nothing but the session-log line, and Finish carries on to the next file.
    let warning = BuildWarning::WorkingFileNotRemoved {
        path: path.to_path_buf(),
    };
    if let Err(error) = ports.warnings.raise(warning) {
        tracing::warn!("Could not record a Build Warning in the session log: {error}");
    }
}

/// The rule above and below the Patch Files manifest.
///
/// 76 `=`, the width of `main`'s startup banner. The batch's two rules are 74 (342) and 75
/// (353) wide, which reads as a typo rather than a choice, so both are normalised to the
/// banner's: a cosmetic divergence, decided on the Finish spec ticket (#48).
const RULE: &str = "============================================================================";

/// The console block Finish prints: the Complete line, then the Patch Files manifest between
/// two rules (batch 339–353), with no trailing newline.
///
/// The words, the list order and the three-space indent are the batch's. The manifest is a
/// fixed list by Build Mode and deliberately not checked on disk (design item 2 on #30): every
/// planned step completed, and each one already stopped the run if its own output was missing.
fn patch_files_report(config: &ProjectConfig) -> String {
    let plugin = &config.plugin;
    let base = &plugin.base_name;

    // Each name from the step that writes it, so the manifest cannot drift from what Steps 4
    // and 5 produce.
    let mut patch_files = vec![plugin.file_name.clone()];
    // Exhaustive, so a new Build Mode has to decide its manifest here (345–349).
    match config.build_mode {
        BuildMode::Clean => {
            patch_files.push(geometry_csg_name(config));
            patch_files.push(cdx_name(config));
        }
        // Xbox skips `CompressPSG`, so it keeps the uncompressed `.psg` (V2.99).
        BuildMode::Xbox => {
            patch_files.push(geometry_psg_name(config));
            patch_files.push(cdx_name(config));
        }
        BuildMode::Filtered => {}
    }
    patch_files.push(plugin.archive_name());

    let mut lines = vec![
        logging::build_complete_line(base),
        String::new(),
        RULE.to_string(),
        "Patch Files created:".to_string(),
    ];
    lines.extend(patch_files.into_iter().map(|file| format!("   {file}")));
    lines.extend([
        String::new(),
        "Move these files into a zip/7z archive and install it, or activate the esp".to_string(),
        RULE.to_string(),
    ]);
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::config::{ArchiveTool, PluginIdentity};

    /// A resolved project configuration for `MyMod` in `build_mode`; the report reads only the
    /// Build Mode and the plugin, so nothing here needs to exist.
    fn project_config(build_mode: BuildMode) -> ProjectConfig {
        ProjectConfig {
            build_mode,
            archive_tool: ArchiveTool::Archive2,
            fallout4_dir: PathBuf::from("Fallout4"),
            data_dir: PathBuf::from("Data"),
            plugin: PluginIdentity::parse("MyMod"),
            non_interactive: true,
            resume_from: None,
        }
    }

    // The expected blocks are spelled out by hand, rules included, rather than built from the
    // helper's own pieces: a test that reused them would agree with a changed rule width or a
    // reordered list instead of catching it.

    #[test]
    fn a_clean_report_lists_the_csg_and_the_cdx() {
        assert_eq!(
            patch_files_report(&project_config(BuildMode::Clean)),
            "Build of Patch MyMod Complete.\n\
             \n\
             ============================================================================\n\
             Patch Files created:\n   \
             MyMod.esp\n   \
             MyMod - Geometry.csg\n   \
             MyMod.cdx\n   \
             MyMod - Main.ba2\n\
             \n\
             Move these files into a zip/7z archive and install it, or activate the esp\n\
             ============================================================================"
        );
    }

    /// Xbox keeps the uncompressed `.psg` since V2.99, because it skips `CompressPSG`.
    #[test]
    fn an_xbox_report_lists_the_psg_and_the_cdx() {
        assert_eq!(
            patch_files_report(&project_config(BuildMode::Xbox)),
            "Build of Patch MyMod Complete.\n\
             \n\
             ============================================================================\n\
             Patch Files created:\n   \
             MyMod.esp\n   \
             MyMod - Geometry.psg\n   \
             MyMod.cdx\n   \
             MyMod - Main.ba2\n\
             \n\
             Move these files into a zip/7z archive and install it, or activate the esp\n\
             ============================================================================"
        );
    }

    #[test]
    fn a_filtered_report_lists_no_geometry_and_no_cdx() {
        assert_eq!(
            patch_files_report(&project_config(BuildMode::Filtered)),
            "Build of Patch MyMod Complete.\n\
             \n\
             ============================================================================\n\
             Patch Files created:\n   \
             MyMod.esp\n   \
             MyMod - Main.ba2\n\
             \n\
             Move these files into a zip/7z archive and install it, or activate the esp\n\
             ============================================================================"
        );
    }

    /// Both rules are the startup banner's 76 `=`, not the batch's 74 (342) and 75 (353). Pinned
    /// by count as well as by text, so nobody "fixes" either back to the batch's width.
    #[test]
    fn both_rules_are_seventy_six_equals_signs() {
        for build_mode in [BuildMode::Clean, BuildMode::Filtered, BuildMode::Xbox] {
            let report = patch_files_report(&project_config(build_mode));
            let rules: Vec<&str> = report
                .lines()
                .filter(|line| line.starts_with('='))
                .collect();

            assert_eq!(rules.len(), 2, "build mode: {build_mode:?}");
            for rule in rules {
                assert_eq!(rule, "=".repeat(76), "build mode: {build_mode:?}");
            }
        }
    }
}
