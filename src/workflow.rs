use anyhow::{Context, Result, bail};
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
        // Check for xPrevisPatch plugins before starting workflow from step 1
        if start_step == WorkflowStep::GeneratePrecombined && self.interactive {
            self.check_xprevis_patches()?;
        }

        if start_step == WorkflowStep::GeneratePrecombined {
            info!(
                "=== Beginning previs generation for {} ===",
                self.plugin_name
            );
            info!("Build Mode: {:?}", self.config.build_mode);
        } else {
            info!(
                "=== Resuming previs generation for {} ===",
                self.plugin_name
            );
            info!("Build Mode: {:?}", self.config.build_mode);
            info!(
                "Starting from: Step {} - {}",
                start_step.number(),
                start_step.name()
            );
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
    ///
    /// Validates that a directory is empty before proceeding with a workflow step.
    /// This is critical for previs generation because leftover files from previous
    /// builds can cause conflicts or incorrect results.
    ///
    /// # Behavior
    ///
    /// - **Directory doesn't exist:** Returns `Ok(())` without creating it
    /// - **Directory is empty:** Returns `Ok(())` without prompting
    /// - **Directory is not empty:**
    ///   - **Interactive mode:** Prompts user "Clean directory?" (Y/N)
    ///     - User selects Yes → Deletes all contents and returns `Ok(())`
    ///     - User selects No → Returns error, workflow stops
    ///   - **Non-interactive mode:** Returns error immediately with helpful message
    ///
    /// # Arguments
    ///
    /// * `dir` - Directory to check (e.g., `Data\meshes\precombined`)
    /// * `dir_name` - Human-readable directory name for prompts and error messages (e.g., `"meshes\\precombined"`)
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` if the directory is empty or successfully cleaned
    ///
    /// # Errors
    ///
    /// This function will return an error if:
    /// - **Interactive mode:** User declines to clean the directory (workflow cannot continue)
    /// - Directory exists and is not empty, but cannot be deleted (permission denied, files in use)
    /// - Directory cannot be recreated after deletion (permission denied, disk full)
    /// - **Non-interactive mode:** Directory is not empty (includes helpful message to clean manually or run interactively)
    ///
    /// # Examples
    ///
    /// ## Interactive Mode
    ///
    /// ```no_run
    /// # use std::path::Path;
    /// # use anyhow::Result;
    /// # struct Executor { interactive: bool }
    /// # impl Executor {
    /// # fn check_and_clean_directory(&self, dir: &Path, dir_name: &str) -> Result<()> { Ok(()) }
    /// # fn example(&self) -> Result<()> {
    /// let precombined_dir = Path::new("C:\\Games\\Fallout4\\Data\\meshes\\precombined");
    /// self.check_and_clean_directory(&precombined_dir, "meshes\\precombined")?;
    /// // Directory is now empty and ready for new precombined meshes
    /// # Ok(()) } }
    /// # Ok::<(), anyhow::Error>(())
    /// ```
    ///
    /// ## Non-Interactive Mode
    ///
    /// ```no_run
    /// # use std::path::Path;
    /// # use anyhow::Result;
    /// # struct Executor { interactive: bool }
    /// # impl Executor {
    /// # fn check_and_clean_directory(&self, dir: &Path, dir_name: &str) -> Result<()> {
    /// #   if !self.interactive {
    /// #     anyhow::bail!("Directory is not empty")
    /// #   }
    /// #   Ok(())
    /// # }
    /// # fn example(&self) -> Result<()> {
    /// // Non-interactive mode with non-empty directory
    /// let vis_dir = Path::new("C:\\Games\\Fallout4\\Data\\vis");
    /// match self.check_and_clean_directory(&vis_dir, "vis") {
    ///     Ok(_) => println!("Directory ready"),
    ///     Err(e) => {
    ///         // Error message: "Directory 'vis' is not empty. Clean it or run interactively."
    ///         eprintln!("{}", e);
    ///     }
    /// }
    /// # Ok(()) } }
    /// # Ok::<(), anyhow::Error>(())
    /// ```
    ///
    /// # Interactive vs. Non-Interactive Behavior
    ///
    /// | Scenario | Interactive Mode | Non-Interactive Mode |
    /// |----------|------------------|---------------------|
    /// | Directory doesn't exist | `Ok(())` | `Ok(())` |
    /// | Directory is empty | `Ok(())` | `Ok(())` |
    /// | Directory has files | Prompt user → Clean or Error | Immediate error |
    ///
    /// # Safety Considerations
    ///
    /// **WARNING: This is a destructive operation.**
    ///
    /// - Deletion is permanent and cannot be undone
    /// - All files and subdirectories in `dir` are deleted recursively
    /// - Always verify `dir_name` matches the actual directory before calling
    /// - In interactive mode, the user is prompted before deletion
    /// - In non-interactive mode, the function fails rather than auto-deleting
    ///
    /// # Notes
    ///
    /// - Uses `prompts::prompt_clean_directory()` for interactive confirmation
    /// - The directory is recreated after deletion (even if it was previously empty)
    /// - This function is called at the start of Steps 1 and 6 to ensure clean working directories
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
    ///
    /// Runs CreationKit to generate precombined meshes (.nif files) for the plugin.
    /// Precombined meshes combine multiple static objects into single meshes for
    /// better performance.
    ///
    /// # Pre-Checks
    ///
    /// - Ensures `meshes/precombined` directory is empty (prompts user if not)
    /// - Ensures `vis` directory is empty (prompts user if not)
    ///
    /// # Process
    ///
    /// 1. Cleans working directories if needed
    /// 2. Runs CreationKit with precombine generation flags
    /// 3. Validates that .nif files were created
    /// 4. In clean mode, validates that .psg file was created
    ///
    /// # Post-Checks
    ///
    /// - Verifies precombined meshes exist in `meshes/precombined`
    /// - In clean mode, checks for PSG file (warns if missing)
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - User declines to clean non-empty directories (interactive mode)
    /// - Directories are not empty (non-interactive mode)
    /// - CreationKit fails to run or crashes
    /// - No precombined meshes were generated
    fn step1_generate_precombined(&self) -> Result<()> {
        // Pre-check: meshes\precombined and vis must be empty
        let precombined_dir = self.data_dir.join("meshes").join("precombined");
        let vis_dir = self.data_dir.join("vis");

        self.check_and_clean_directory(&precombined_dir, "meshes\\precombined")?;
        self.check_and_clean_directory(&vis_dir, "vis")?;

        // Run CreationKit
        let ck_log = self
            .config
            .ck_log_path
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("CK log path not configured"))?;

        let mut ck_runner =
            CreationKitRunner::new(&self.config.creation_kit_path, &self.config.fo4_dir)
                .with_log_file(ck_log);

        if let Some(ref mo2_path) = self.config.mo2_path {
            ck_runner = ck_runner.with_mo2(mo2_path);
        }

        ck_runner.generate_precombined(&self.plugin_name, self.config.build_mode)?;

        // Post-check: .nif files created
        if !precombined_dir.exists() || filesystem::is_directory_empty(&precombined_dir) {
            bail!("No precombined meshes were generated");
        }

        // Post-check: .psg file created (clean mode only)
        if self.config.build_mode == BuildMode::Clean {
            let plugin_base = validation::get_plugin_base_name(&self.plugin_name);
            let psg_file = self
                .data_dir
                .join(format!("{} - Geometry.psg", plugin_base));

            if !psg_file.exists() {
                warn!("PSG file not created: {}", psg_file.display());
            }
        }

        Ok(())
    }

    /// Step 2: Merge PrecombineObjects.esp Via xEdit
    ///
    /// Runs FO4Edit to merge the temporary PrecombineObjects.esp (created by CreationKit)
    /// into the main plugin. This consolidates precombine data into the plugin itself.
    ///
    /// # Pre-Checks
    ///
    /// - Verifies precombined meshes exist from Step 1
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - No precombined meshes found (Step 1 not completed)
    /// - FO4Edit fails to run or merge operation fails
    fn step2_merge_combined_objects(&self) -> Result<()> {
        // Pre-check: Precombined meshes exist
        let precombined_dir = self.data_dir.join("meshes").join("precombined");

        if !precombined_dir.exists() || filesystem::is_directory_empty(&precombined_dir) {
            bail!("No precombined meshes found. Run Step 1 first.");
        }

        // Run FO4Edit
        let mut fo4edit_runner =
            FO4EditRunner::new(&self.config.fo4edit_path, &self.config.fo4_dir);

        if let Some(ref mo2_path) = self.config.mo2_path {
            fo4edit_runner = fo4edit_runner.with_mo2(mo2_path);
        }

        fo4edit_runner.merge_combined_objects(&self.plugin_name)?;

        Ok(())
    }

    /// Step 3: Create BA2 Archive from Precombines
    ///
    /// Creates a BA2 archive containing all precombined meshes. The archive is named
    /// `<PluginName> - Main.ba2` and uses either PC or Xbox compression format.
    ///
    /// # Process
    ///
    /// - Uses Archive2 or BSArch (depending on configuration)
    /// - Archives all .nif files from `meshes/precombined`
    /// - MO2-aware: Collects files from MO2 staging directory if configured
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Archive tool fails to create the BA2 file
    /// - No precombined meshes found to archive
    fn step3_create_precombined_archive(&self) -> Result<()> {
        let plugin_base = validation::get_plugin_base_name(&self.plugin_name);
        let archive_name = format!("{} - Main.ba2", plugin_base);

        let (archive2_path, bsarch_path) = match self.config.archive_tool {
            crate::config::ArchiveTool::Archive2 => {
                (Some(self.config.archive_exe_path.clone()), None)
            }
            crate::config::ArchiveTool::BSArch => {
                (None, Some(self.config.archive_exe_path.clone()))
            }
        };

        let archive_manager = ArchiveManager::new(
            self.config.archive_tool,
            archive2_path,
            bsarch_path,
            &self.config.fo4_dir,
        )?;

        let is_xbox = self.config.build_mode == BuildMode::Xbox;
        let mo2_data_dir = self.config.mo2_data_dir.as_deref();

        archive_manager.create_archive_from_precombines(&archive_name, is_xbox, mo2_data_dir)?;

        info!("Created archive: {}", archive_name);
        Ok(())
    }

    /// Step 4: Compress PSG Via CK (clean mode only)
    ///
    /// Runs CreationKit to compress the PSG (PreSceneGraph) file created in Step 1.
    /// This step is only performed in clean mode, not in filtered mode.
    ///
    /// # Pre-Checks
    ///
    /// - Verifies the PSG file exists: `<PluginName> - Geometry.psg`
    ///
    /// # Process
    ///
    /// 1. Runs CreationKit to compress the PSG file
    /// 2. Deletes the original PSG file after compression
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - PSG file not found (Step 1 may have failed)
    /// - CreationKit fails to compress the PSG
    /// - PSG file cannot be deleted after compression
    fn step4_compress_psg(&self) -> Result<()> {
        let plugin_base = validation::get_plugin_base_name(&self.plugin_name);
        let psg_file = self
            .data_dir
            .join(format!("{} - Geometry.psg", plugin_base));

        // Pre-check: .psg file exists
        if !psg_file.exists() {
            bail!("PSG file not found: {}", psg_file.display());
        }

        // Run CreationKit
        let ck_log = self
            .config
            .ck_log_path
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("CK log path not configured"))?;

        let mut ck_runner =
            CreationKitRunner::new(&self.config.creation_kit_path, &self.config.fo4_dir)
                .with_log_file(ck_log);

        if let Some(ref mo2_path) = self.config.mo2_path {
            ck_runner = ck_runner.with_mo2(mo2_path);
        }

        ck_runner.compress_psg(&self.plugin_name)?;

        // Delete .psg file
        fs::remove_file(&psg_file)
            .with_context(|| format!("Failed to delete PSG file: {}", psg_file.display()))?;

        info!("Deleted PSG file");
        Ok(())
    }

    /// Step 5: Build CDX Via CK (clean mode only)
    ///
    /// Runs CreationKit to build CDX (Combined Data Index) files. This step is only
    /// performed in clean mode, not in filtered mode.
    ///
    /// # Errors
    ///
    /// Returns an error if CreationKit fails to build the CDX files
    fn step5_build_cdx(&self) -> Result<()> {
        let ck_log = self
            .config
            .ck_log_path
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("CK log path not configured"))?;

        let mut ck_runner =
            CreationKitRunner::new(&self.config.creation_kit_path, &self.config.fo4_dir)
                .with_log_file(ck_log);

        if let Some(ref mo2_path) = self.config.mo2_path {
            ck_runner = ck_runner.with_mo2(mo2_path);
        }

        ck_runner.build_cdx(&self.plugin_name)?;

        Ok(())
    }

    /// Step 6: Generate Previs Via CK
    ///
    /// Runs CreationKit to generate previs (precomputed visibility) data. Previs data
    /// tells the engine which objects are visible from different locations, improving
    /// performance by culling invisible objects.
    ///
    /// # Pre-Checks
    ///
    /// - Ensures `vis` directory is empty (prompts user if not)
    ///
    /// # Process
    ///
    /// 1. Cleans `vis` directory if needed
    /// 2. Runs CreationKit to generate previs data
    /// 3. Validates that .uvd files were created
    ///
    /// # Post-Checks
    ///
    /// - Verifies previs data exists in `vis` directory
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - User declines to clean non-empty `vis` directory (interactive mode)
    /// - `vis` directory is not empty (non-interactive mode)
    /// - CreationKit fails to run or crashes
    /// - No previs data was generated
    fn step6_generate_previs(&self) -> Result<()> {
        // Pre-check: vis directory empty
        let vis_dir = self.data_dir.join("vis");

        self.check_and_clean_directory(&vis_dir, "vis")?;

        // Run CreationKit
        let ck_log = self
            .config
            .ck_log_path
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("CK log path not configured"))?;

        let mut ck_runner =
            CreationKitRunner::new(&self.config.creation_kit_path, &self.config.fo4_dir)
                .with_log_file(ck_log);

        if let Some(ref mo2_path) = self.config.mo2_path {
            ck_runner = ck_runner.with_mo2(mo2_path);
        }

        ck_runner.generate_previs(&self.plugin_name)?;

        // Post-check: .uvd files created
        if !vis_dir.exists() || filesystem::is_directory_empty(&vis_dir) {
            bail!("No previs data was generated");
        }

        Ok(())
    }

    /// Step 7: Merge Previs.esp Via xEdit
    ///
    /// Runs FO4Edit to merge the temporary Previs.esp (created by CreationKit)
    /// into the main plugin. This consolidates previs data into the plugin itself.
    ///
    /// # Pre-Checks
    ///
    /// - Verifies previs data (.uvd files) exist from Step 6
    /// - Verifies Previs.esp was created by CreationKit
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - No previs data found (Step 6 not completed)
    /// - Previs.esp not found (CreationKit failed to create it)
    /// - FO4Edit fails to run or merge operation fails
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
        let mut fo4edit_runner =
            FO4EditRunner::new(&self.config.fo4edit_path, &self.config.fo4_dir);

        if let Some(ref mo2_path) = self.config.mo2_path {
            fo4edit_runner = fo4edit_runner.with_mo2(mo2_path);
        }

        fo4edit_runner.merge_previs(&self.plugin_name)?;

        Ok(())
    }

    /// Step 8: Add Previs files to BA2 Archive
    ///
    /// Adds previs data (.uvd files) to the existing BA2 archive created in Step 3.
    /// This completes the previs generation workflow.
    ///
    /// # Process
    ///
    /// - Uses Archive2 or BSArch (depending on configuration)
    /// - For Archive2: Extract → Add files → Re-archive (no append support)
    /// - For BSArch: Appends files directly to existing archive
    /// - MO2-aware: Collects files from MO2 staging directory if configured
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Archive tool fails to add files to the BA2
    /// - No previs data found to add
    /// - For Archive2: Extraction or re-archiving fails
    fn step8_add_previs_to_archive(&self) -> Result<()> {
        let plugin_base = validation::get_plugin_base_name(&self.plugin_name);
        let archive_name = format!("{} - Main.ba2", plugin_base);

        let (archive2_path, bsarch_path) = match self.config.archive_tool {
            crate::config::ArchiveTool::Archive2 => {
                (Some(self.config.archive_exe_path.clone()), None)
            }
            crate::config::ArchiveTool::BSArch => {
                (None, Some(self.config.archive_exe_path.clone()))
            }
        };

        let archive_manager = ArchiveManager::new(
            self.config.archive_tool,
            archive2_path,
            bsarch_path,
            &self.config.fo4_dir,
        )?;

        let is_xbox = self.config.build_mode == BuildMode::Xbox;
        let mo2_data_dir = self.config.mo2_data_dir.as_deref();

        archive_manager.add_previs_to_archive(&archive_name, is_xbox, mo2_data_dir)?;

        info!("Added previs data to archive: {}", archive_name);
        Ok(())
    }

    /// Check for xPrevisPatch plugins and prompt to rename
    fn check_xprevis_patches(&self) -> Result<()> {
        let xprevis_plugins = filesystem::find_xprevis_patch_plugins(&self.data_dir)?;

        if !xprevis_plugins.is_empty() {
            println!();
            for plugin in &xprevis_plugins {
                println!("  Found: {}", plugin);
            }

            if prompts::prompt_rename_xprevis_patch()? {
                println!(
                    "\nPlease rename the xPrevisPatch plugin(s) manually before continuing."
                );
                println!("You can add a suffix like '_old' or '_backup' to the filename.");
                anyhow::bail!("xPrevisPatch plugin(s) detected - please rename and restart");
            } else {
                println!("\nContinuing anyway - but be aware this may cause conflicts.");
            }
        }

        Ok(())
    }

    /// Clean up working files if user confirms
    fn cleanup_working_files(&self) -> Result<()> {
        let working_files = filesystem::find_working_files(&self.data_dir)?;

        if !working_files.is_empty() && prompts::prompt_remove_working_files()? {
            for file_name in &working_files {
                let file_path = self.data_dir.join(file_name);
                if file_path.exists() {
                    fs::remove_file(&file_path).with_context(|| {
                        format!("Failed to delete working file: {}", file_path.display())
                    })?;
                    info!("Deleted: {}", file_name);
                }
            }
            println!("\nWorking files cleaned up successfully");
        }

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
        info!(
            "Previsibines generated successfully for {}!",
            self.plugin_name
        );
        info!("");
        info!("What's next:");
        info!("  • Test your mod in-game to verify everything works");

        // In interactive mode, prompt to clean up working files
        if self.interactive {
            println!();
            if let Err(e) = self.cleanup_working_files() {
                warn!("Failed to clean up working files: {}", e);
            }
        } else {
            info!("  • Clean up temp files if needed (Previs.esp, PrecombineObjects.esp)");
        }
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
