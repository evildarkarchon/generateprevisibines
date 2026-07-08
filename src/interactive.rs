//! Interactive plugin flow (`:GetPlugin`, `:TryCopySeed`, `:CheckPluginExists`, `:GetStep`).

use std::path::Path;

use dialoguer::{Confirm, Input};

use crate::config::{BuildMode, PluginIdentity, WorkflowStep};
use crate::error::{Error, Result};
use crate::timing::{self, MO2_DELAY_AFTER_SEED_COPY_SECS};
use crate::toolchain::PluginReadiness;
use crate::validation;
use crate::workflow;

/// Result of prompting when the target plugin already exists (batch `:CheckPluginExists`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExistingPluginAction {
    Continue,
    Exit,
    ChooseResumeStep,
}

/// Result of resume-step menu input (`0` re-prompts plugin flow).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResumeStepChoice {
    RePromptPlugin,
    Step(WorkflowStep),
}

/// Parse Y/N/C from `:CheckPluginExists` (case-insensitive).
#[must_use]
pub fn parse_existing_plugin_choice(input: &str) -> Option<ExistingPluginAction> {
    match input.trim().to_ascii_uppercase().as_str() {
        "Y" | "YES" => Some(ExistingPluginAction::Continue),
        "N" | "NO" => Some(ExistingPluginAction::Exit),
        "C" => Some(ExistingPluginAction::ChooseResumeStep),
        _ => None,
    }
}

/// Parse resume menu choice: `0` re-prompts plugin; `1`–`8` map to workflow steps.
#[must_use]
pub fn parse_resume_step_choice(input: &str, build_mode: BuildMode) -> Option<ResumeStepChoice> {
    let trimmed = input.trim();
    let n: u8 = trimmed.parse().ok()?;
    if n == 0 {
        return Some(ResumeStepChoice::RePromptPlugin);
    }
    let step = WorkflowStep::from_number(n)?;
    let allowed = WorkflowStep::steps_for_mode(build_mode);
    allowed
        .contains(&step)
        .then_some(ResumeStepChoice::Step(step))
}

/// Copy seed plugin `xPrevisPatch.esp` and wait for MO2 VFS if needed.
pub fn copy_seed_plugin(data_dir: &Path, plugin_path: &Path) -> Result<()> {
    let seed = data_dir.join("xPrevisPatch.esp");
    if !seed.is_file() {
        return Err(Error::SeedPluginMissing);
    }

    if let Some(parent) = plugin_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    std::fs::copy(&seed, plugin_path)?;

    if !plugin_path.is_file() {
        timing::mo2_sync_delay(MO2_DELAY_AFTER_SEED_COPY_SECS);
    }

    if plugin_path.is_file() {
        Ok(())
    } else {
        Err(Error::SeedCopyFailed)
    }
}

/// Prompt for plugin name until valid or user aborts (empty line exits 0).
pub fn prompt_plugin_name(build_mode: BuildMode) -> Result<Option<PluginIdentity>> {
    println!();
    println!("Enter the name of the Plugin you wish to build Previsibines for.");
    println!("(Do not include the .esp extension unless you need .esm/.esl)");
    println!();

    loop {
        let raw: String = Input::new()
            .with_prompt("Plugin name")
            .allow_empty(true)
            .interact_text()?;

        if raw.trim().is_empty() {
            return Ok(None);
        }

        let plugin = PluginIdentity::parse(&raw);
        match validation::validate_plugin(&plugin, build_mode) {
            Ok(()) => return Ok(Some(plugin)),
            Err(err) => eprintln!("ERROR - {err}"),
        }
    }
}

/// Y/N seed copy prompt (batch `:TryCopySeed`).
pub fn prompt_seed_copy(data_dir: &Path, plugin_path: &Path) -> Result<bool> {
    let seed = data_dir.join("xPrevisPatch.esp");
    println!(
        "Plugin {} does not exist.",
        plugin_path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
    );
    if !seed.is_file() {
        return Err(Error::SeedPluginMissing);
    }

    let copy = Confirm::new()
        .with_prompt("Copy xPrevisPatch.esp as a starting plugin?")
        .default(false)
        .interact()?;

    if copy {
        copy_seed_plugin(data_dir, plugin_path)?;
        println!("Seed plugin copied.");
    }

    Ok(copy)
}

/// Y/N/C when plugin file already exists.
pub fn prompt_existing_plugin_action(plugin_file: &str) -> Result<ExistingPluginAction> {
    loop {
        let raw: String = Input::new()
            .with_prompt(format!(
                "Plugin {plugin_file} already exists. Continue (Y), Exit (N), or Choose resume step (C)?"
            ))
            .interact_text()?;

        if let Some(action) = parse_existing_plugin_choice(&raw) {
            return Ok(action);
        }
        eprintln!("Enter Y, N, or C");
    }
}

/// Resume step menu; `None` when user picks `0` (re-prompt plugin).
pub fn prompt_resume_step(build_mode: BuildMode) -> Result<Option<WorkflowStep>> {
    println!();
    println!("Choose which step to resume from:");
    workflow::print_resume_menu(build_mode);
    println!("[0] Re-enter plugin name");

    loop {
        let raw: String = Input::new().with_prompt("Step number").interact_text()?;

        match parse_resume_step_choice(&raw, build_mode) {
            Some(ResumeStepChoice::RePromptPlugin) => return Ok(None),
            Some(ResumeStepChoice::Step(step)) => return Ok(Some(step)),
            None => eprintln!("Invalid step for this build mode."),
        }
    }
}

/// Interactive Y/N confirmation for clearing existing precombined meshes (`:RePrecomb`).
pub fn confirm_clear_precombined(precombined_dir: &Path) -> Result<bool> {
    Ok(Confirm::new()
        .with_prompt(format!(
            "Precombined meshes exist in {}. Delete them before regenerating?",
            precombined_dir.display()
        ))
        .default(true)
        .interact()?)
}

/// Orchestrate plugin existence, seed copy, archive guard, and resume prompts.
pub fn ensure_plugin_ready(readiness: &PluginReadiness) -> Result<ExistingPluginAction> {
    let plugin_path = readiness.plugin_path();
    let data_dir = readiness.data_dir();

    if readiness.plugin_archive_path().is_file() {
        return Err(Error::PluginAlreadyHasArchive);
    }

    if !plugin_path.is_file() {
        if readiness.non_interactive() {
            return Err(Error::Other(format!(
                "plugin not found: {}",
                plugin_path.display()
            )));
        }

        if !prompt_seed_copy(data_dir, &plugin_path)? {
            return Ok(ExistingPluginAction::Exit);
        }

        if !plugin_path.is_file() {
            return Err(Error::Other(format!(
                "plugin not found after seed copy: {}",
                plugin_path.display()
            )));
        }

        return Ok(ExistingPluginAction::Continue);
    }

    if readiness.non_interactive() {
        return Ok(ExistingPluginAction::Continue);
    }

    match prompt_existing_plugin_action(readiness.plugin_file_name())? {
        ExistingPluginAction::Continue => Ok(ExistingPluginAction::Continue),
        other => Ok(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::BuildMode;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn parses_existing_plugin_choices() {
        assert_eq!(
            parse_existing_plugin_choice("y"),
            Some(ExistingPluginAction::Continue)
        );
        assert_eq!(
            parse_existing_plugin_choice("N"),
            Some(ExistingPluginAction::Exit)
        );
        assert_eq!(
            parse_existing_plugin_choice("c"),
            Some(ExistingPluginAction::ChooseResumeStep)
        );
        assert!(parse_existing_plugin_choice("x").is_none());
    }

    #[test]
    fn resume_step_zero_reprompts() {
        assert_eq!(
            parse_resume_step_choice("0", BuildMode::Clean),
            Some(ResumeStepChoice::RePromptPlugin)
        );
    }

    #[test]
    fn resume_step_four_clean_is_compress_psg() {
        assert_eq!(
            parse_resume_step_choice("4", BuildMode::Clean),
            Some(ResumeStepChoice::Step(WorkflowStep::CompressPsg))
        );
    }

    #[test]
    fn resume_step_four_filtered_invalid() {
        assert!(parse_resume_step_choice("4", BuildMode::Filtered).is_none());
    }

    #[test]
    fn seed_copy_writes_plugin() {
        let dir = tempdir().unwrap();
        let data = dir.path();
        fs::write(data.join("xPrevisPatch.esp"), b"seed").unwrap();
        let dest = data.join("MyMod.esp");
        copy_seed_plugin(data, &dest).unwrap();
        assert!(dest.is_file());
    }

    #[test]
    fn ensure_plugin_ready_rejects_existing_archive() {
        use crate::config::PluginIdentity;
        use std::fs;
        use tempfile::tempdir;

        let dir = tempdir().unwrap();
        let data = dir.path();
        fs::create_dir_all(data).unwrap();
        let plugin = PluginIdentity::parse("MyMod");
        fs::write(data.join(&plugin.file_name), b"plug").unwrap();
        fs::write(data.join(plugin.archive_name()), b"ba2").unwrap();

        let readiness = PluginReadiness::new(plugin, data.to_path_buf(), true);

        let err = super::ensure_plugin_ready(&readiness).unwrap_err();
        assert!(matches!(err, crate::error::Error::PluginAlreadyHasArchive));
    }
}
