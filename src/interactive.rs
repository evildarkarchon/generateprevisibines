//! Interactive plugin flow (`:GetPlugin`, `:TryCopySeed`, `:CheckPluginExists`, `:GetStep`).

use std::path::Path;

use dialoguer::{Confirm, Input};

use crate::config::{BuildMode, PluginIdentity, WorkflowStep};
use crate::error::{Error, Result};
use crate::files::{FileSpace, SystemFileSpace};
use crate::toolchain::PluginReadiness;
use crate::tools::wait::{MO2_DELAY_AFTER_SEED_COPY_SECS, SystemWait, Wait};
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

/// Copy seed plugin `xPrevisPatch.esp` as opaque bytes and wait for MO2 VFS if needed.
///
/// # Errors
///
/// Returns [`Error::SeedPluginMissing`] when the seed is absent and [`Error::SeedCopyFailed`]
/// when the copied plugin remains invisible after the conditional wait. Destination-parent
/// creation and operating-system copy failures propagate as [`Error::Io`].
pub fn copy_seed_plugin(data_dir: &Path, plugin_path: &Path) -> Result<()> {
    let seed = data_dir.join("xPrevisPatch.esp");
    if !seed.is_file() {
        return Err(Error::SeedPluginMissing);
    }

    // ESP payloads are binary; the File Space copy keeps them out of tolerant text reads.
    SystemFileSpace.copy(&seed, plugin_path)?;

    if !plugin_path.is_file() {
        // This legacy compatibility helper retains its direct production wait until its
        // policy-bearing surface is removed; Workflow Request Intake now owns the active seed
        // copy episode through its injected Wait port.
        SystemWait.sync_delay(MO2_DELAY_AFTER_SEED_COPY_SECS);
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

    read_plugin_name(
        build_mode,
        || {
            Ok(Input::new()
                .with_prompt("Plugin name")
                .allow_empty(true)
                .interact_text()?)
        },
        |err| eprintln!("ERROR - {err}"),
    )
}

/// Read plugin-name answers until the terminal input converts to a typed candidate or an exit.
///
/// Returns `None` for a blank answer, which is the batch's deliberate-exit response. Invalid names
/// are passed to `report_invalid` and re-read here so terminal repetition never leaks into Intake;
/// Intake still validates the returned candidate itself because other adapters may supply it.
/// Reader failures propagate unchanged to the caller.
fn read_plugin_name(
    build_mode: BuildMode,
    mut read_answer: impl FnMut() -> Result<String>,
    mut report_invalid: impl FnMut(&Error),
) -> Result<Option<PluginIdentity>> {
    loop {
        let raw = read_answer()?;

        if raw.trim().is_empty() {
            return Ok(None);
        }

        let plugin = PluginIdentity::parse(&raw);
        match validation::validate_plugin(&plugin, build_mode) {
            Ok(()) => return Ok(Some(plugin)),
            Err(err) => report_invalid(&err),
        }
    }
}

/// Report the candidate plugin that Workflow Request Intake found missing.
pub fn report_missing_plugin(plugin_file: &str) {
    println!("Plugin {plugin_file} does not exist.");
}

/// Ask whether the missing plugin should be copied from `xPrevisPatch.esp`.
///
/// This adapter owns only terminal presentation and answer conversion; Intake establishes that
/// the seed exists and performs any accepted copy through its File Space.
///
/// # Errors
///
/// Returns [`Error::Prompt`] when the terminal confirmation cannot be completed.
pub fn prompt_seed_copy_confirmation() -> Result<bool> {
    Ok(Confirm::new()
        .with_prompt("Copy xPrevisPatch.esp as a starting plugin?")
        .default(false)
        .interact()?)
}

/// Report that the seed plugin was copied and became visible to Intake.
pub fn report_seed_copy_success() {
    println!("Seed plugin copied.");
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
    read_existing_plugin_action(
        || {
            Ok(Input::new()
                .with_prompt(format!(
                    "Plugin {plugin_file} already exists. Continue (Y), Exit (N), or Choose resume step (C)?"
                ))
                .interact_text()?)
        },
        || eprintln!("Enter Y, N, or C"),
    )
}

/// Read existing-plugin answers until the terminal input converts to a typed Intake action.
///
/// The injected reader and invalid-answer reporter keep repetition testable without a real
/// terminal. Reader failures propagate unchanged to the caller.
fn read_existing_plugin_action(
    mut read_answer: impl FnMut() -> Result<String>,
    mut report_invalid: impl FnMut(),
) -> Result<ExistingPluginAction> {
    loop {
        let raw = read_answer()?;
        if let Some(action) = parse_existing_plugin_choice(&raw) {
            return Ok(action);
        }
        report_invalid();
    }
}

/// Resume step menu; `None` when user picks `0` (re-prompt plugin).
pub fn prompt_resume_step(build_mode: BuildMode) -> Result<Option<WorkflowStep>> {
    println!();
    println!("Choose which step to resume from:");
    workflow::print_resume_menu(build_mode);
    println!("[0] Re-enter plugin name");

    read_resume_step(
        build_mode,
        || Ok(Input::new().with_prompt("Step number").interact_text()?),
        || eprintln!("Invalid step for this build mode."),
    )
}

/// Read resume answers until the terminal input converts to an allowed Workflow Step or re-entry.
///
/// The injected reader and invalid-answer reporter keep repetition testable without a real
/// terminal. Reader failures propagate unchanged to the caller.
fn read_resume_step(
    build_mode: BuildMode,
    mut read_answer: impl FnMut() -> Result<String>,
    mut report_invalid: impl FnMut(),
) -> Result<Option<WorkflowStep>> {
    loop {
        let raw = read_answer()?;

        match parse_resume_step_choice(&raw, build_mode) {
            Some(ResumeStepChoice::RePromptPlugin) => return Ok(None),
            Some(ResumeStepChoice::Step(step)) => return Ok(Some(step)),
            None => report_invalid(),
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
    use std::cell::{Cell, RefCell};
    use std::collections::VecDeque;
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

    /// Existing-plugin input remains inside the terminal adapter until a typed action is parsed.
    #[test]
    fn existing_plugin_prompt_repeats_invalid_input_inside_the_terminal_adapter() {
        let mut answers = VecDeque::from(["x".to_owned(), "?".to_owned(), "c".to_owned()]);
        let invalid_answers = Cell::new(0);

        let action = read_existing_plugin_action(
            || Ok(answers.pop_front().unwrap()),
            || invalid_answers.set(invalid_answers.get() + 1),
        )
        .unwrap();

        assert_eq!(action, ExistingPluginAction::ChooseResumeStep);
        assert_eq!(invalid_answers.get(), 2);
    }

    /// Resume input remains inside the terminal adapter until the build mode permits the step.
    #[test]
    fn resume_prompt_repeats_build_mode_incompatible_input_inside_the_terminal_adapter() {
        let mut answers = VecDeque::from(["4".to_owned(), "3".to_owned()]);
        let invalid_answers = Cell::new(0);

        let step = read_resume_step(
            BuildMode::Filtered,
            || Ok(answers.pop_front().unwrap()),
            || invalid_answers.set(invalid_answers.get() + 1),
        )
        .unwrap();

        assert_eq!(step, Some(WorkflowStep::CreateBa2FromPrecombines));
        assert_eq!(invalid_answers.get(), 1);
    }

    /// A blank plugin-name line is the terminal's deliberate-exit answer, not invalid input.
    #[test]
    fn plugin_name_prompt_converts_blank_input_to_exit_without_reporting_invalid_input() {
        let mut answers = VecDeque::from(["   ".to_owned()]);
        let invalid_answers = Cell::new(0);

        let plugin = read_plugin_name(
            BuildMode::Clean,
            || Ok(answers.pop_front().unwrap()),
            |_| invalid_answers.set(invalid_answers.get() + 1),
        )
        .unwrap();

        assert_eq!(plugin, None);
        assert_eq!(invalid_answers.get(), 0);
    }

    /// Invalid plugin names are reported and re-read inside the terminal adapter, so Intake only
    /// ever receives a typed candidate or a deliberate exit.
    #[test]
    fn plugin_name_prompt_repeats_invalid_input_inside_the_terminal_adapter() {
        let mut answers =
            VecDeque::from(["previs".to_owned(), "My Mod".to_owned(), "MyMod".to_owned()]);
        let reported = RefCell::new(Vec::new());

        let plugin = read_plugin_name(
            BuildMode::Clean,
            || Ok(answers.pop_front().unwrap()),
            |error| reported.borrow_mut().push(error.to_string()),
        )
        .unwrap();

        assert_eq!(plugin, Some(PluginIdentity::parse("MyMod")));
        assert_eq!(reported.borrow().len(), 2);
        assert!(answers.is_empty());
    }

    /// Terminal reader failures surface unchanged instead of being treated as an exit answer.
    #[test]
    fn plugin_name_prompt_propagates_reader_failures() {
        let err = read_plugin_name(
            BuildMode::Clean,
            || Err(Error::Other("recorded terminal failure".to_owned())),
            |_| panic!("a reader failure is not invalid input"),
        )
        .unwrap_err();

        assert!(matches!(err, Error::Other(message) if message == "recorded terminal failure"));
    }

    #[test]
    fn seed_copy_preserves_binary_bytes_creates_parents_and_replaces_the_plugin() {
        let dir = tempdir().unwrap();
        let data = dir.path();
        let seed = data.join("xPrevisPatch.esp");
        let dest = data.join("nested").join("MyMod.esp");
        let plugin_bytes = [0x00, 0xFF, 0x80, b'E', b'S', b'P'];
        fs::write(&seed, plugin_bytes).unwrap();

        copy_seed_plugin(data, &dest).unwrap();

        assert_eq!(fs::read(&dest).unwrap(), plugin_bytes);

        fs::write(&dest, b"stale").unwrap();
        copy_seed_plugin(data, &dest).unwrap();

        assert_eq!(fs::read(dest).unwrap(), plugin_bytes);
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
