use anyhow::{bail, Context, Result};
use log::{info, warn};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::config::{BuildMode, Config};
use crate::filesystem;
use crate::prompts;
use crate::tools::{ArchiveManager, CreationKitRunner, FO4EditRunner};
use crate::validation;

/// Workflow steps for previs generation
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum WorkflowStep {
    GeneratePrecombined = 1,
    MergeCombinedObjects = 2,
    CreatePrecombinedArchive = 3,
    CompressPSG = 4,
    BuildCDX = 5,
    GeneratePrevis = 6,
    MergePrevis = 7,
    AddPrevisToArchive = 8,
}

impl WorkflowStep {
    /// Get step number (1-8)
    pub fn number(&self) -> u8 {
        *self as u8
    }

    /// Get step name for display
    pub fn name(&self) -> &'static str {
        match self {
            Self::GeneratePrecombined => "Generate Precombines Via CK",
            Self::MergeCombinedObjects => "Merge PrecombineObjects.esp Via xEdit",
            Self::CreatePrecombinedArchive => "Create BA2 Archive from Precombines",
            Self::CompressPSG => "Compress PSG Via CK",
            Self::BuildCDX => "Build CDX Via CK",
            Self::GeneratePrevis => "Generate Previs Via CK",
            Self::MergePrevis => "Merge Previs.esp Via xEdit",
            Self::AddPrevisToArchive => "Add Previs files to BA2 Archive",
        }
    }

    /// Check if step is clean-mode only
    pub fn is_clean_mode_only(&self) -> bool {
        matches!(self, Self::CompressPSG | Self::BuildCDX)
    }

    /// Convert from step number (1-8)
    pub fn from_number(n: u8) -> Option<Self> {
        match n {
            1 => Some(Self::GeneratePrecombined),
            2 => Some(Self::MergeCombinedObjects),
            3 => Some(Self::CreatePrecombinedArchive),
            4 => Some(Self::CompressPSG),
            5 => Some(Self::BuildCDX),
            6 => Some(Self::GeneratePrevis),
            7 => Some(Self::MergePrevis),
            8 => Some(Self::AddPrevisToArchive),
            _ => None,
        }
    }

    /// Get next step
    pub fn next(&self) -> Option<Self> {
        Self::from_number(self.number() + 1)
    }
}

/// Workflow executor for the 8-step previs generation process
pub struct WorkflowExecutor<'a> {
    config: &'a Config,
    plugin_name: String,
    data_dir: PathBuf,
    start_time: Instant,
    interactive: bool,
}

impl<'a> WorkflowExecutor<'a> {
    /// Create a new workflow executor
    pub fn new(config: &'a Config, plugin_name: String, interactive: bool) -> Self {
        let data_dir = config.data_dir();

        Self {
            config,
            plugin_name,
            data_dir,
            start_time: Instant::now(),
            interactive,
        }
    }

    /// Run the complete workflow from step 1 to 8
    pub fn run_all(&self) -> Result<()> {
        self.run_from_step(WorkflowStep::GeneratePrecombined)
    }

    /// Run the workflow starting from a specific step
    pub fn run_from_step(&self, start_step: WorkflowStep) -> Result<()> {
        if start_step == WorkflowStep::GeneratePrecombined {
            info!("=== Beginning previs generation for {} ===", self.plugin_name);
            info!("Build Mode: {:?}", self.config.build_mode);
        } else {
            info!("=== Resuming previs generation for {} ===", self.plugin_name);
            info!("Build Mode: {:?}", self.config.build_mode);
            info!("Starting from: Step {} - {}", start_step.number(), start_step.name());
        }

        let mut current_step = Some(start_step);

        while let Some(step) = current_step {
            // Skip clean-mode-only steps if not in clean mode
            if step.is_clean_mode_only() && self.config.build_mode != BuildMode::Clean {
                info!(
                    "Skipping Step {} - {} (clean mode only)",
                    step.number(),
                    step.name()
                );
                current_step = step.next();
                continue;
            }

            info!("");
            info!("=== Step {} - {} ===", step.number(), step.name());

            // Execute the step
            self.execute_step(step)?;

            info!("Step {} completed successfully", step.number());
            current_step = step.next();
        }

        self.print_summary();
        Ok(())
    }

    /// Execute a specific workflow step
    fn execute_step(&self, step: WorkflowStep) -> Result<()> {
        match step {
            WorkflowStep::GeneratePrecombined => self.step1_generate_precombined(),
            WorkflowStep::MergeCombinedObjects => self.step2_merge_combined_objects(),
            WorkflowStep::CreatePrecombinedArchive => self.step3_create_precombined_archive(),
            WorkflowStep::CompressPSG => self.step4_compress_psg(),
            WorkflowStep::BuildCDX => self.step5_build_cdx(),
            WorkflowStep::GeneratePrevis => self.step6_generate_previs(),
            WorkflowStep::MergePrevis => self.step7_merge_previs(),
            WorkflowStep::AddPrevisToArchive => self.step8_add_previs_to_archive(),
        }
    }

    /// Check if a directory needs cleaning, prompt user if interactive
    fn check_and_clean_directory(&self, dir: &Path, dir_name: &str) -> Result<()> {
        if !dir.exists() {
            return Ok(());
        }

        if filesystem::is_directory_empty(dir) {
            return Ok(());
        }

        // Directory is not empty
        if self.interactive {
            // Prompt user to clean
            if prompts::prompt_clean_directory(dir_name)? {
                info!("Cleaning directory: {}", dir_name);
                fs::remove_dir_all(dir)?;
                fs::create_dir_all(dir)?;
            } else {
                bail!("Cannot proceed: Directory '{}' is not empty", dir_name);
            }
        } else {
            // Non-interactive mode: fail
            bail!(
                "Directory '{}' is not empty. Clean it or run interactively.",
                dir_name
            );
        }

        Ok(())
    }

    /// Step 1: Generate Precombines Via CK
    fn step1_generate_precombined(&self) -> Result<()> {
        // Pre-check: meshes\precombined and vis must be empty
        let precombined_dir = self.data_dir.join("meshes").join("precombined");
        let vis_dir = self.data_dir.join("vis");

        self.check_and_clean_directory(&precombined_dir, "meshes\\precombined")?;
        self.check_and_clean_directory(&vis_dir, "vis")?;

        // Run CreationKit
        let ck_log = self.config.ck_log_path.as_ref()
            .ok_or_else(|| anyhow::anyhow!("CK log path not configured"))?;

        let ck_runner = CreationKitRunner::new(&self.config.creation_kit_path, &self.config.fo4_dir)
            .with_log_file(ck_log);

        ck_runner.generate_precombined(&self.plugin_name, self.config.build_mode)?;

        // Post-check: .nif files created
        if !precombined_dir.exists() || filesystem::is_directory_empty(&precombined_dir) {
            bail!("No precombined meshes were generated");
        }

        // Post-check: .psg file created (clean mode only)
        if self.config.build_mode == BuildMode::Clean {
            let plugin_base = validation::get_plugin_base_name(&self.plugin_name);
            let psg_file = self.data_dir.join(format!("{} - Geometry.psg", plugin_base));

            if !psg_file.exists() {
                warn!("PSG file not created: {}", psg_file.display());
            }
        }

        Ok(())
    }

    /// Step 2: Merge PrecombineObjects.esp Via xEdit
    fn step2_merge_combined_objects(&self) -> Result<()> {
        // Pre-check: Precombined meshes exist
        let precombined_dir = self.data_dir.join("meshes").join("precombined");

        if !precombined_dir.exists() || filesystem::is_directory_empty(&precombined_dir) {
            bail!("No precombined meshes found. Run Step 1 first.");
        }

        // Run FO4Edit
        let fo4edit_runner = FO4EditRunner::new(&self.config.fo4edit_path, &self.config.fo4_dir);
        fo4edit_runner.merge_combined_objects(&self.plugin_name)?;

        Ok(())
    }

    /// Step 3: Create BA2 Archive from Precombines
    fn step3_create_precombined_archive(&self) -> Result<()> {
        let precombined_dir = self.data_dir.join("meshes").join("precombined");

        if !precombined_dir.exists() {
            bail!("Precombined directory not found");
        }

        let plugin_base = validation::get_plugin_base_name(&self.plugin_name);
        let archive_name = format!("{} - Main.ba2", plugin_base);

        let (archive2_path, bsarch_path) = match self.config.archive_tool {
            crate::config::ArchiveTool::Archive2 => (Some(self.config.archive_exe_path.clone()), None),
            crate::config::ArchiveTool::BSArch => (None, Some(self.config.archive_exe_path.clone())),
        };

        let archive_manager = ArchiveManager::new(
            self.config.archive_tool,
            archive2_path,
            bsarch_path,
            &self.config.fo4_dir,
        )?;

        let is_xbox = self.config.build_mode == BuildMode::Xbox;
        archive_manager.create_archive(&precombined_dir, &archive_name, is_xbox)?;

        info!("Created archive: {}", archive_name);
        Ok(())
    }

    /// Step 4: Compress PSG Via CK (clean mode only)
    fn step4_compress_psg(&self) -> Result<()> {
        let plugin_base = validation::get_plugin_base_name(&self.plugin_name);
        let psg_file = self.data_dir.join(format!("{} - Geometry.psg", plugin_base));

        // Pre-check: .psg file exists
        if !psg_file.exists() {
            bail!("PSG file not found: {}", psg_file.display());
        }

        // Run CreationKit
        let ck_log = self.config.ck_log_path.as_ref()
            .ok_or_else(|| anyhow::anyhow!("CK log path not configured"))?;

        let ck_runner = CreationKitRunner::new(&self.config.creation_kit_path, &self.config.fo4_dir)
            .with_log_file(ck_log);

        ck_runner.compress_psg(&self.plugin_name)?;

        // Delete .psg file
        fs::remove_file(&psg_file)
            .with_context(|| format!("Failed to delete PSG file: {}", psg_file.display()))?;

        info!("Deleted PSG file");
        Ok(())
    }

    /// Step 5: Build CDX Via CK (clean mode only)
    fn step5_build_cdx(&self) -> Result<()> {
        let ck_log = self.config.ck_log_path.as_ref()
            .ok_or_else(|| anyhow::anyhow!("CK log path not configured"))?;

        let ck_runner = CreationKitRunner::new(&self.config.creation_kit_path, &self.config.fo4_dir)
            .with_log_file(ck_log);

        ck_runner.build_cdx(&self.plugin_name)?;

        Ok(())
    }

    /// Step 6: Generate Previs Via CK
    fn step6_generate_previs(&self) -> Result<()> {
        // Pre-check: vis directory empty
        let vis_dir = self.data_dir.join("vis");

        self.check_and_clean_directory(&vis_dir, "vis")?;

        // Run CreationKit
        let ck_log = self.config.ck_log_path.as_ref()
            .ok_or_else(|| anyhow::anyhow!("CK log path not configured"))?;

        let ck_runner = CreationKitRunner::new(&self.config.creation_kit_path, &self.config.fo4_dir)
            .with_log_file(ck_log);

        ck_runner.generate_previs(&self.plugin_name)?;

        // Post-check: .uvd files created
        if !vis_dir.exists() || filesystem::is_directory_empty(&vis_dir) {
            bail!("No previs data was generated");
        }

        Ok(())
    }

    /// Step 7: Merge Previs.esp Via xEdit
    fn step7_merge_previs(&self) -> Result<()> {
        // Pre-check: .uvd files exist
        let vis_dir = self.data_dir.join("vis");

        if !vis_dir.exists() || filesystem::is_directory_empty(&vis_dir) {
            bail!("No previs data found. Run Step 6 first.");
        }

        // Pre-check: Previs.esp exists
        let previs_esp = self.data_dir.join("Previs.esp");
        if !previs_esp.exists() {
            bail!("Previs.esp not found. CreationKit should have created it.");
        }

        // Run FO4Edit
        let fo4edit_runner = FO4EditRunner::new(&self.config.fo4edit_path, &self.config.fo4_dir);
        fo4edit_runner.merge_previs(&self.plugin_name)?;

        Ok(())
    }

    /// Step 8: Add Previs files to BA2 Archive
    fn step8_add_previs_to_archive(&self) -> Result<()> {
        let vis_dir = self.data_dir.join("vis");

        if !vis_dir.exists() {
            bail!("Vis directory not found");
        }

        let plugin_base = validation::get_plugin_base_name(&self.plugin_name);
        let archive_name = format!("{} - Main.ba2", plugin_base);

        let (archive2_path, bsarch_path) = match self.config.archive_tool {
            crate::config::ArchiveTool::Archive2 => (Some(self.config.archive_exe_path.clone()), None),
            crate::config::ArchiveTool::BSArch => (None, Some(self.config.archive_exe_path.clone())),
        };

        let archive_manager = ArchiveManager::new(
            self.config.archive_tool,
            archive2_path,
            bsarch_path,
            &self.config.fo4_dir,
        )?;

        let is_xbox = self.config.build_mode == BuildMode::Xbox;
        archive_manager.add_to_archive(&vis_dir, &archive_name, is_xbox)?;

        info!("Added previs data to archive: {}", archive_name);
        Ok(())
    }

    /// Print final summary
    fn print_summary(&self) {
        let elapsed = self.start_time.elapsed();
        let minutes = elapsed.as_secs() / 60;
        let seconds = elapsed.as_secs() % 60;

        info!("");
        info!("=== All done! ===");
        info!("Plugin: {}", self.plugin_name);
        info!("Build Mode: {:?}", self.config.build_mode);
        info!("Completed in: {}m {}s", minutes, seconds);
        info!("");
        info!("Previsibines generated successfully for {}!", self.plugin_name);
        info!("");
        info!("What's next:");
        info!("  • Test your mod in-game to verify everything works");
        info!("  • Clean up temp files if needed (Previs.esp, PrecombineObjects.esp)");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_workflow_step_numbers() {
        assert_eq!(WorkflowStep::GeneratePrecombined.number(), 1);
        assert_eq!(WorkflowStep::AddPrevisToArchive.number(), 8);
    }

    #[test]
    fn test_workflow_step_from_number() {
        assert_eq!(
            WorkflowStep::from_number(1),
            Some(WorkflowStep::GeneratePrecombined)
        );
        assert_eq!(
            WorkflowStep::from_number(8),
            Some(WorkflowStep::AddPrevisToArchive)
        );
        assert_eq!(WorkflowStep::from_number(0), None);
        assert_eq!(WorkflowStep::from_number(9), None);
    }

    #[test]
    fn test_clean_mode_only_steps() {
        assert!(!WorkflowStep::GeneratePrecombined.is_clean_mode_only());
        assert!(WorkflowStep::CompressPSG.is_clean_mode_only());
        assert!(WorkflowStep::BuildCDX.is_clean_mode_only());
        assert!(!WorkflowStep::GeneratePrevis.is_clean_mode_only());
    }

    #[test]
    fn test_step_next() {
        assert_eq!(
            WorkflowStep::GeneratePrecombined.next(),
            Some(WorkflowStep::MergeCombinedObjects)
        );
        assert_eq!(WorkflowStep::AddPrevisToArchive.next(), None);
    }
}
