use anyhow::{Context, Result};
use clap::Parser;
use log::info;
use std::path::PathBuf;

mod ckpe_config;
mod config;
mod filesystem;
mod mo2_helper;
mod prompts;
mod registry;
mod tools;
mod utils;
mod validation;
mod workflow;

use config::{ArchiveTool, BuildMode, Config};

#[derive(Parser, Debug)]
#[command(name = "generateprevisibines")]
#[command(about = "Automate Fallout 4 precombine and previs generation", long_about = None)]
#[allow(clippy::struct_excessive_bools)]
struct Args {
    /// Plugin name (e.g., MyMod.esp)
    #[arg(value_name = "PLUGIN")]
    plugin: Option<String>,

    /// Build mode: clean (default if not specified)
    #[arg(short = 'c', long = "clean", conflicts_with_all = ["filtered", "xbox"])]
    clean: bool,

    /// Build mode: filtered
    #[arg(short = 'f', long = "filtered", conflicts_with_all = ["clean", "xbox"])]
    filtered: bool,

    /// Build mode: xbox
    #[arg(short = 'x', long = "xbox", conflicts_with_all = ["clean", "filtered"])]
    xbox: bool,

    /// Use `BSArch` instead of Archive2
    #[arg(long = "bsarch")]
    bsarch: bool,

    /// Override Fallout 4 directory
    #[arg(long = "FO4", value_name = "PATH")]
    fo4_dir: Option<PathBuf>,

    /// Use Mod Organizer 2 mode (runs tools through MO2's VFS)
    /// Requires --mo2-path to be specified
    #[arg(long = "mo2", requires = "mo2_path")]
    mo2_mode: bool,

    /// Path to ModOrganizer.exe (required when using --mo2)
    #[arg(long = "mo2-path", value_name = "PATH")]
    mo2_path: Option<PathBuf>,

    /// Path to MO2's VFS staging directory (e.g., overwrite folder)
    /// Required when using --mo2 for archiving operations
    #[arg(long = "mo2-data-dir", value_name = "PATH")]
    mo2_data_dir: Option<PathBuf>,
}

impl Args {
    /// Get the build mode
    fn get_build_mode(&self) -> BuildMode {
        if self.filtered {
            BuildMode::Filtered
        } else if self.xbox {
            BuildMode::Xbox
        } else {
            BuildMode::Clean // default
        }
    }

    /// Get the archive tool
    fn get_archive_tool(&self) -> ArchiveTool {
        if self.bsarch {
            ArchiveTool::BSArch
        } else {
            ArchiveTool::Archive2
        }
    }
}

#[allow(clippy::too_many_lines)]
fn main() -> Result<()> {
    // Initialize logging to %TEMP%
    let log_path = utils::init_logging().context("Failed to initialize logging")?;
    info!("GeneratePrevisibines started");
    info!("Log file: {}", log_path.display());

    let args = Args::parse();

    println!("======================================");
    println!("  GeneratePrevisibines - Rust Edition");
    println!("======================================");
    println!();

    // Determine FO4 directory
    let fo4_dir = if let Some(ref dir) = args.fo4_dir {
        println!("Using FO4 directory from command line: {}", dir.display());
        dir.clone()
    } else {
        println!("Finding Fallout 4 installation...");
        let dir = registry::find_fo4_directory()
            .context("Failed to find Fallout 4 installation. Use --FO4 to specify manually.")?;
        println!("Found Fallout 4 at: {}", dir.display());
        dir
    };

    // Find FO4Edit
    println!();
    println!("Finding FO4Edit...");
    let fo4edit_path = registry::find_fo4edit_path().context(
        "Failed to find FO4Edit. Make sure it's in the current directory or properly installed.",
    )?;
    println!("Found FO4Edit at: {}", fo4edit_path.display());

    // Find Creation Kit
    println!();
    println!("Finding Creation Kit...");
    let ck_path = registry::find_creation_kit(&fo4_dir)
        .context("Failed to find Creation Kit in FO4 directory")?;
    println!("Found Creation Kit at: {}", ck_path.display());

    // Find Archive tool
    println!();
    let archive_tool = args.get_archive_tool();
    let archive_path = match archive_tool {
        ArchiveTool::Archive2 => {
            println!("Finding Archive2...");
            registry::find_archive2(&fo4_dir)
                .context("Failed to find Archive2.exe in FO4 Tools directory")?
        }
        ArchiveTool::BSArch => {
            println!("Finding BSArch...");
            registry::find_bsarch(&fo4_dir).context("Failed to find BSArch.exe in FO4 directory")?
        }
    };
    println!(
        "Found {} at: {}",
        match archive_tool {
            ArchiveTool::Archive2 => "Archive2",
            ArchiveTool::BSArch => "BSArch",
        },
        archive_path.display()
    );

    // Validate FO4 directories
    println!();
    println!("Validating Fallout 4 installation...");
    filesystem::validate_fo4_directories(&fo4_dir).context("Invalid Fallout 4 installation")?;
    println!("Fallout 4 installation validated successfully.");

    // Find and parse CKPE config
    println!();
    println!("Checking for CKPE configuration...");
    let ckpe_config_result = registry::find_ckpe_config(&fo4_dir);
    let (ckpe_config_path, ck_log_path) = if let Some(ref config_path) = ckpe_config_result {
        println!("Found CKPE config at: {}", config_path.display());

        // Parse and validate CKPE config
        let ckpe_cfg = ckpe_config::CKPEConfig::parse(config_path)
            .context("Failed to parse CKPE configuration")?;

        println!("CKPE config type: {:?}", ckpe_cfg.config_type);

        // Validate required settings
        ckpe_cfg
            .validate()
            .context("CKPE configuration validation failed")?;
        println!("✓ bBSPointerHandleExtremly is enabled");

        let log_path = if let Some(ref log_path) = ckpe_cfg.log_file_path {
            println!("CK log file: {}", log_path.display());
            Some(log_path.clone())
        } else {
            println!("Warning: CK log file path not found in CKPE config");
            None
        };

        (Some(config_path.clone()), log_path)
    } else {
        println!("Warning: No CKPE configuration file found.");
        println!("The workflow may fail if CKPE is not properly configured.");
        (None, None)
    };

    // Display versions
    println!();
    println!("======================================");
    println!("  Tool Versions");
    println!("======================================");

    let fo4_exe = fo4_dir.join("Fallout4.exe");
    if fo4_exe.exists() {
        let version = utils::get_simple_version(&fo4_exe);
        println!("Fallout 4:      {version}");
    }

    let fo4edit_version = utils::get_simple_version(&fo4edit_path);
    println!("FO4Edit:        {fo4edit_version}");

    let ck_version = utils::get_simple_version(&ck_path);
    println!("Creation Kit:   {ck_version}");

    let archive_version = utils::get_simple_version(&archive_path);
    println!(
        "{}: {}",
        match archive_tool {
            ArchiveTool::Archive2 => "Archive2   ",
            ArchiveTool::BSArch => "BSArch     ",
        },
        archive_version
    );

    // Configure MO2 if enabled
    let (mo2_config, mo2_data_dir_config) = if args.mo2_mode {
        if let Some(ref mo2_path) = args.mo2_path {
            if !mo2_path.exists() {
                anyhow::bail!("Mod Organizer 2 not found at: {}", mo2_path.display());
            }
            println!();
            let mo2_version = utils::get_simple_version(mo2_path);
            println!("Mod Organizer 2: {mo2_version}");

            // Validate mo2_data_dir if provided
            let mo2_data_dir = if let Some(ref data_dir) = args.mo2_data_dir {
                if !data_dir.exists() {
                    anyhow::bail!("MO2 data directory not found at: {}", data_dir.display());
                }
                println!("MO2 data dir:    {}", data_dir.display());
                Some(data_dir.clone())
            } else {
                println!(
                    "Warning: --mo2-data-dir not specified. Archiving may not work correctly in MO2 mode."
                );
                None
            };

            (Some(mo2_path.clone()), mo2_data_dir)
        } else {
            anyhow::bail!("--mo2 flag requires --mo2-path to be specified");
        }
    } else {
        (None, None)
    };

    println!();
    println!("======================================");
    println!("  Configuration");
    println!("======================================");
    println!("Build mode:     {}", args.get_build_mode().as_str());
    println!(
        "Archive tool:   {}",
        match archive_tool {
            ArchiveTool::Archive2 => "Archive2",
            ArchiveTool::BSArch => "BSArch",
        }
    );
    if args.mo2_mode {
        println!("MO2 mode:       Enabled");
        if let Some(ref mo2_path) = mo2_config {
            println!("MO2 path:       {}", mo2_path.display());
        }
    } else {
        println!("MO2 mode:       Disabled");
    }
    if let Some(ref plugin) = args.plugin {
        println!("Plugin:         {plugin}");
    }
    println!();

    // Create configuration
    let mut config = Config::new(args.get_build_mode(), archive_tool);
    config.fo4_dir.clone_from(&fo4_dir);
    config.fo4edit_path = fo4edit_path;
    config.creation_kit_path = ck_path;
    config.archive_exe_path = archive_path;
    config.ckpe_config_path = ckpe_config_path;
    config.ck_log_path = ck_log_path;
    config.plugin_name.clone_from(&args.plugin);
    config.mo2_mode = args.mo2_mode;
    config.mo2_path = mo2_config;
    config.mo2_data_dir = mo2_data_dir_config;

    // Validate configuration
    config
        .validate()
        .context("Configuration validation failed")?;

    // Validate plugin name if provided
    if let Some(ref plugin_name) = args.plugin {
        println!();
        println!("======================================");
        println!("  Plugin Validation");
        println!("======================================");

        let is_clean_mode = matches!(args.get_build_mode(), BuildMode::Clean);
        validation::validate_plugin_name(plugin_name, is_clean_mode)
            .context("Plugin name validation failed")?;
        println!("✓ Plugin name is valid");

        // Check if plugin exists
        let data_dir = fo4_dir.join("Data");
        if validation::plugin_exists(&data_dir, plugin_name) {
            println!(
                "✓ Plugin file exists: {}",
                data_dir.join(plugin_name).display()
            );
        } else {
            println!(
                "Warning: Plugin file not found at: {}",
                data_dir.join(plugin_name).display()
            );
            println!("Make sure the plugin is in the Data directory before running the workflow.");
        }
    }

    // Ensure output directories exist
    println!();
    println!("======================================");
    println!("  Directory Setup");
    println!("======================================");
    let data_dir = fo4_dir.join("Data");
    let (precombined_dir, vis_dir) = filesystem::ensure_output_directories(&data_dir)
        .context("Failed to create output directories")?;

    println!("✓ Created/verified output directories:");
    println!("  Precombined: {}", precombined_dir.display());
    println!("  Vis:         {}", vis_dir.display());

    // Count existing files in output directories
    let nif_count = filesystem::count_files(&precombined_dir, "nif");
    let uvd_count = filesystem::count_files(&vis_dir, "uvd");

    if nif_count > 0 || uvd_count > 0 {
        println!();
        println!("Existing previs/precombine files found:");
        if nif_count > 0 {
            println!("  {nif_count} .nif files in precombined directory");
        }
        if uvd_count > 0 {
            println!("  {uvd_count} .uvd files in vis directory");
        }
        println!("These will be managed during the workflow steps.");
    }

    println!();
    println!("======================================");
    println!("  Summary");
    println!("======================================");
    println!("✓ All tools found and validated successfully!");
    println!("✓ CKPE configuration validated");
    println!("✓ Output directories ready");
    println!();

    info!("Configuration validated successfully");

    // Get plugin name (prompt if not provided)
    let interactive = args.plugin.is_none();
    let plugin_name = if let Some(plugin) = args.plugin {
        plugin
    } else {
        println!("======================================");
        println!("  Plugin Selection");
        println!("======================================");
        let is_clean_mode = matches!(args.get_build_mode(), BuildMode::Clean);
        prompts::prompt_plugin_name(is_clean_mode)?
    };

    info!("Plugin name: {plugin_name}");

    // Check if plugin exists
    let data_dir = fo4_dir.join("Data");
    let plugin_path = data_dir.join(&plugin_name);

    if validation::plugin_exists(&data_dir, &plugin_name) {
        println!("✓ Plugin file found: {}", plugin_path.display());

        // In interactive mode, ask if user wants to use existing or restart
        if interactive {
            match prompts::prompt_use_existing_plugin(&plugin_path)? {
                Some(true) => {
                    println!("Using existing plugin");
                    // Ask which step to resume from
                    if let Some(step_number) = prompts::prompt_restart_step()? {
                        let start_step = workflow::WorkflowStep::from_number(step_number)
                            .ok_or_else(|| anyhow::anyhow!("Invalid step number"))?;

                        println!();
                        println!(
                            "Starting from: Step {} - {}",
                            start_step.number(),
                            start_step.name()
                        );
                        println!();
                        let executor =
                            workflow::WorkflowExecutor::new(&config, plugin_name, interactive);
                        executor.run_from_step(start_step)?;
                    } else {
                        println!("Workflow cancelled by user");
                        return Ok(());
                    }
                }
                Some(false) => {
                    println!("Starting fresh workflow from step 1");
                    println!();
                    let executor =
                        workflow::WorkflowExecutor::new(&config, plugin_name, interactive);
                    executor.run_all()?;
                }
                None => {
                    println!("Workflow cancelled by user");
                    return Ok(());
                }
            }
        } else {
            // Non-interactive: just run from step 1
            println!();
            let executor = workflow::WorkflowExecutor::new(&config, plugin_name, interactive);
            executor.run_all()?;
        }
    } else {
        println!(
            "Warning: Plugin file not found at: {}",
            plugin_path.display()
        );

        if interactive {
            if prompts::confirm(
                "Continue anyway? (plugin will be created by CreationKit)",
                false,
            )? {
                println!();
                let executor = workflow::WorkflowExecutor::new(&config, plugin_name, interactive);
                executor.run_all()?;
            } else {
                println!("Workflow cancelled by user");
                return Ok(());
            }
        } else {
            anyhow::bail!(
                "Plugin file not found: {}\n\
                Make sure the plugin exists in the Data directory or run interactively.",
                plugin_path.display()
            );
        }
    }

    println!();
    println!("Log file: {}", log_path.display());
    info!("Workflow completed successfully");

    Ok(())
}
