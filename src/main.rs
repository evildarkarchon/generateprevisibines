use anyhow::{Context, Result};
use clap::Parser;
use log::info;
use std::path::PathBuf;

mod ckpe_config;
mod config;
mod filesystem;
mod registry;
mod utils;
mod validation;

use config::{ArchiveTool, BuildMode, Config};

#[derive(Parser, Debug)]
#[command(name = "generateprevisibines")]
#[command(about = "Automate Fallout 4 precombine and previs generation", long_about = None)]
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

    /// Use BSArch instead of Archive2
    #[arg(long = "bsarch")]
    bsarch: bool,

    /// Override Fallout 4 directory
    #[arg(long = "FO4", value_name = "PATH")]
    fo4_dir: Option<PathBuf>,
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
    let fo4edit_path = registry::find_fo4edit_path()
        .context("Failed to find FO4Edit. Make sure it's in the current directory or properly installed.")?;
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
            registry::find_bsarch(&fo4_dir)
                .context("Failed to find BSArch.exe in FO4 directory")?
        }
    };
    println!("Found {} at: {}",
        match archive_tool { ArchiveTool::Archive2 => "Archive2", ArchiveTool::BSArch => "BSArch" },
        archive_path.display()
    );

    // Validate FO4 directories
    println!();
    println!("Validating Fallout 4 installation...");
    filesystem::validate_fo4_directories(&fo4_dir)
        .context("Invalid Fallout 4 installation")?;
    println!("Fallout 4 installation validated successfully.");

    // Find and parse CKPE config
    println!();
    println!("Checking for CKPE configuration...");
    let ckpe_config_result = registry::find_ckpe_config(&fo4_dir);
    let ckpe_config_path = if let Some(ref config_path) = ckpe_config_result {
        println!("Found CKPE config at: {}", config_path.display());

        // Parse and validate CKPE config
        let ckpe_cfg = ckpe_config::CKPEConfig::parse(config_path)
            .context("Failed to parse CKPE configuration")?;

        println!(
            "CKPE config type: {:?}",
            ckpe_cfg.config_type
        );

        // Validate required settings
        ckpe_cfg.validate().context("CKPE configuration validation failed")?;
        println!("✓ bBSPointerHandleExtremly is enabled");

        if let Some(ref log_path) = ckpe_cfg.log_file_path {
            println!("CK log file: {}", log_path.display());
        }

        Some(config_path.clone())
    } else {
        println!("Warning: No CKPE configuration file found.");
        println!("The workflow may fail if CKPE is not properly configured.");
        None
    };

    // Display versions
    println!();
    println!("======================================");
    println!("  Tool Versions");
    println!("======================================");

    let fo4_exe = fo4_dir.join("Fallout4.exe");
    if fo4_exe.exists() {
        let version = utils::get_simple_version(&fo4_exe);
        println!("Fallout 4:      {}", version);
    }

    let fo4edit_version = utils::get_simple_version(&fo4edit_path);
    println!("FO4Edit:        {}", fo4edit_version);

    let ck_version = utils::get_simple_version(&ck_path);
    println!("Creation Kit:   {}", ck_version);

    let archive_version = utils::get_simple_version(&archive_path);
    println!("{}: {}",
        match archive_tool { ArchiveTool::Archive2 => "Archive2   ", ArchiveTool::BSArch => "BSArch     " },
        archive_version
    );

    println!();
    println!("======================================");
    println!("  Configuration");
    println!("======================================");
    println!("Build mode:     {}", args.get_build_mode().as_str());
    println!("Archive tool:   {}", match archive_tool { ArchiveTool::Archive2 => "Archive2", ArchiveTool::BSArch => "BSArch" });
    if let Some(ref plugin) = args.plugin {
        println!("Plugin:         {}", plugin);
    }
    println!();

    // Create configuration
    let mut config = Config::new(args.get_build_mode(), archive_tool);
    config.fo4_dir = fo4_dir.clone();
    config.fo4edit_path = fo4edit_path;
    config.creation_kit_path = ck_path;
    config.archive_exe_path = archive_path;
    config.ckpe_config_path = ckpe_config_path;
    config.plugin_name = args.plugin.clone();

    // Validate configuration
    config.validate().context("Configuration validation failed")?;

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
            println!("✓ Plugin file exists: {}", data_dir.join(plugin_name).display());
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
            println!("  {} .nif files in precombined directory", nif_count);
        }
        if uvd_count > 0 {
            println!("  {} .uvd files in vis directory", uvd_count);
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
    println!("Log file: {}", log_path.display());

    info!("Configuration validated successfully");
    info!("Ready to proceed with workflow");

    Ok(())
}
