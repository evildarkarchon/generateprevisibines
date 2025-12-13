use anyhow::Result;
use dialoguer::{Confirm, Input, Select};
use std::path::Path;

use crate::validation::validate_plugin_name;

/// Prompt user for plugin name with validation
///
/// Validates:
/// - No reserved names (previs, combinedobjects, xprevispatch)
/// - No spaces in clean mode
/// - Ensures .esp/.esm extension
pub fn prompt_plugin_name(clean_mode: bool) -> Result<String> {
    loop {
        let input: String = Input::new()
            .with_prompt("Enter the name of the plugin to generate previsibines for")
            .interact_text()?;

        let input = input.trim();

        if input.is_empty() {
            println!("Plugin name cannot be empty. Please try again.");
            continue;
        }

        // Ensure extension is present
        let plugin_name =
            if !input.to_lowercase().ends_with(".esp") && !input.to_lowercase().ends_with(".esm") {
                format!("{}.esp", input)
            } else {
                input.to_string()
            };

        match validate_plugin_name(&plugin_name, clean_mode) {
            Ok(()) => return Ok(plugin_name),
            Err(e) => {
                println!("{}", e);
                continue;
            }
        }
    }
}

/// Prompt for using existing plugin or starting fresh
///
/// Returns:
/// - Some(true): Use existing plugin
/// - Some(false): Start fresh
/// - None: User chose to exit
pub fn prompt_use_existing_plugin(plugin_path: &Path) -> Result<Option<bool>> {
    println!("\nPlugin already exists: {}", plugin_path.display());

    let choices = vec![
        "Yes - Use existing plugin and continue",
        "No - Start fresh (will backup existing)",
        "Exit - Cancel operation",
    ];

    let selection = Select::new()
        .with_prompt("What would you like to do?")
        .items(&choices)
        .default(0)
        .interact()?;

    match selection {
        0 => Ok(Some(true)),  // Yes
        1 => Ok(Some(false)), // No
        _ => Ok(None),        // Exit or any other selection
    }
}

/// Prompt for which step to restart from
///
/// Returns:
/// - Some(1..=8): Step number to restart from
/// - None: User chose to exit (0)
pub fn prompt_restart_step() -> Result<Option<u8>> {
    println!("\nWorkflow can resume from any of these steps:");
    println!("  1. Generate Precombines Via CK");
    println!("  2. Merge PrecombineObjects.esp Via xEdit");
    println!("  3. Create BA2 Archive from Precombines");
    println!("  4. Compress PSG Via CK (clean mode only)");
    println!("  5. Build CDX Via CK (clean mode only)");
    println!("  6. Generate Previs Via CK");
    println!("  7. Merge Previs.esp Via xEdit");
    println!("  8. Add Previs files to BA2 Archive");
    println!("  0. Exit");

    let step: u8 = Input::new()
        .with_prompt("Enter step number to restart from (0-8)")
        .validate_with(|input: &u8| -> Result<(), &str> {
            if *input <= 8 {
                Ok(())
            } else {
                Err("Please enter a number between 0 and 8")
            }
        })
        .interact_text()?;

    if step == 0 { Ok(None) } else { Ok(Some(step)) }
}

/// Prompt to confirm cleaning a directory
pub fn prompt_clean_directory(dir_name: &str) -> Result<bool> {
    Confirm::new()
        .with_prompt(format!(
            "Directory '{}' is not empty. Delete existing files?",
            dir_name
        ))
        .default(false)
        .interact()
        .map_err(Into::into)
}

/// Prompt to confirm removing working files
pub fn prompt_remove_working_files() -> Result<bool> {
    println!("\nThe following temporary files can be removed:");
    println!("  - Previs.esp");
    println!("  - PrecombineObjects.esp");
    println!("  - SeventySix*.esp");

    Confirm::new()
        .with_prompt("Remove working files?")
        .default(true)
        .interact()
        .map_err(Into::into)
}

/// Simple yes/no confirmation
pub fn confirm(prompt: &str, default: bool) -> Result<bool> {
    Confirm::new()
        .with_prompt(prompt)
        .default(default)
        .interact()
        .map_err(Into::into)
}

#[cfg(test)]
mod tests {

    // Note: Interactive prompts are difficult to unit test
    // These would require mocking stdin or using a testing framework
    // that supports interactive input simulation

    #[test]
    fn test_module_compiles() {
        // Basic compilation test
        assert!(true);
    }
}
