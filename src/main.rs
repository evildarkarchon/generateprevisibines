//! GeneratePrevisibines — Rust port (batch V2.96 reference).

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use generateprevisibines::{
    cli::Cli,
    discovery, interactive, logging,
    tools::{
        assert_resume_step_implemented, filter_runnable_steps, ProductionRunner, ScaffoldRunner,
        ToolContext,
    },
    validation, workflow,
};
use generateprevisibines::{interactive::ExistingPluginAction, workflow::WorkflowEngine};
use tracing_subscriber::EnvFilter;

fn main() -> ExitCode {
    if let Err(err) = run() {
        eprintln!("ERROR - {err}");
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn run() -> generateprevisibines::Result<()> {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("generateprevisibines=info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();

    print_banner();

    let cli = Cli::parse();
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."));

    let tools = discovery::discover_tools(&exe_dir, cli.fo4_dir.clone())?;

    if let Some(ref fo4edit) = tools.fo4edit {
        tracing::info!(
            "{}",
            discovery::format_version_line("FO4Edit", fo4edit, None)
        );
    }
    if let Some(ref fo4) = tools.fallout4_dir {
        tracing::info!("Fallout 4 directory: {}", fo4.display());
    }

    let Some(fallout4_dir) = tools.fallout4_dir.clone() else {
        return Err(generateprevisibines::Error::Other(
            "Fallout 4 directory could not be determined. Use --FO4 <DIR>.".into(),
        ));
    };

    if cli.dry_run {
        run_dry_run(&cli, fallout4_dir);
        return Ok(());
    }

    let Some((mut config, ck_log_path)) = resolve_config(&cli, &tools, fallout4_dir)? else {
        return Ok(());
    };

    validate_ckpe_and_scripts(&config, &exe_dir, &tools)?;

    config.ck_log_path = Some(ck_log_path);

    assert_resume_step_implemented(&config)?;

    let planned = WorkflowEngine::<ProductionRunner>::planned_steps(&config);
    let runnable = filter_runnable_steps(&planned);
    if runnable.is_empty() {
        return Err(generateprevisibines::Error::StepNotImplemented(
            planned
                .first()
                .map_or(1, |s| s.number()),
        ));
    }
    if runnable.len() < planned.len() {
        tracing::warn!(
            skipped = planned.len() - runnable.len(),
            "Later workflow steps are not implemented yet; running Step 1 only."
        );
    }

    let log_path = logging::session_log_path(&config.plugin);
    logging::init_session_log(
        &log_path,
        config.build_mode.as_str(),
        &config.plugin.file_name,
    )?;

    let creation_kit = tools.creation_kit.ok_or_else(|| {
        generateprevisibines::Error::Other(format!(
            "CreationKit.exe not found in {}",
            config.fallout4_dir.display()
        ))
    })?;

    let ctx = ToolContext {
        session_log: Some(log_path.clone()),
        unattended_log: Some(logging::unattended_log_path()),
        fallout4_dir: config.fallout4_dir.clone(),
        creation_kit,
        ck_log_path: config.ck_log_path.clone(),
    };

    let engine = WorkflowEngine::new(ProductionRunner::new());
    engine.run_steps(&runnable, &config, &ctx)?;

    println!("Build step(s) complete. Log: {}", log_path.display());
    Ok(())
}

/// Resolve plugin + config. `Ok(None)` when the user exits from interactive prompts.
fn resolve_config(
    cli: &Cli,
    tools: &discovery::ToolPaths,
    fallout4_dir: PathBuf,
) -> generateprevisibines::Result<Option<(generateprevisibines::ProjectConfig, PathBuf)>> {
    let build_mode = cli.build_mode();
    let archive_tool = cli.archive_tool();
    let xedit_data_dir = cli.fo4_dir.as_ref().map(|d| d.join("Data"));
    let fo4edit_path = tools.fo4edit.clone();

    if cli.plugin.is_some() {
        let mut config = cli.clone().into_project_config(fallout4_dir)?;
        config.fo4edit_path = fo4edit_path;

        match interactive::ensure_plugin_ready(&config)? {
            ExistingPluginAction::Exit => return Ok(None),
            ExistingPluginAction::ChooseResumeStep => {
                return Err(generateprevisibines::Error::Other(
                    "resume step selection requires interactive mode".into(),
                ));
            }
            ExistingPluginAction::Continue => {}
        }

        let ck_log = resolve_ck_log_from_install(&config)?;
        return Ok(Some((config, ck_log)));
    }

    let mut resume_from = cli.resume_from;

    loop {
        let Some(plugin) = interactive::prompt_plugin_name(build_mode)? else {
            return Ok(None);
        };

        let mut config = Cli::project_config(
            fallout4_dir.clone(),
            build_mode,
            archive_tool,
            plugin,
            false,
            resume_from,
            xedit_data_dir.clone(),
            fo4edit_path.clone(),
            None,
        )?;

        match interactive::ensure_plugin_ready(&config)? {
            ExistingPluginAction::Exit => return Ok(None),
            ExistingPluginAction::ChooseResumeStep => {
                if let Some(step) = interactive::prompt_resume_step(build_mode)? {
                    config.resume_from = Some(step);
                } else {
                    resume_from = None;
                    continue;
                }
            }
            ExistingPluginAction::Continue => {}
        }

        let ck_log = resolve_ck_log_from_install(&config)?;
        return Ok(Some((config, ck_log)));
    }
}

fn resolve_ck_log_from_install(
    config: &generateprevisibines::ProjectConfig,
) -> generateprevisibines::Result<PathBuf> {
    let ckpe_path = config
        .fallout4_dir
        .join(validation::detect_ckpe_config_kind(&config.fallout4_dir).file_name());
    let contents = std::fs::read_to_string(&ckpe_path)?;
    let (_kind, log_file) = validation::validate_ckpe_config(&config.fallout4_dir, &contents)?;
    Ok(interactive::resolve_ck_log_path(
        &config.fallout4_dir,
        &log_file,
    ))
}

fn validate_ckpe_and_scripts(
    config: &generateprevisibines::ProjectConfig,
    exe_dir: &std::path::Path,
    tools: &discovery::ToolPaths,
) -> generateprevisibines::Result<()> {
    let ckpe_path = config
        .fallout4_dir
        .join(validation::detect_ckpe_config_kind(&config.fallout4_dir).file_name());
    if ckpe_path.is_file() {
        let contents = std::fs::read_to_string(&ckpe_path)?;
        let (kind, log_file) = validation::validate_ckpe_config(&config.fallout4_dir, &contents)?;
        tracing::info!("Using CKPE config: {} (log: {log_file})", kind.file_name());
    } else {
        return Err(generateprevisibines::Error::CkpeConfig(format!(
            "CKPE not configured. File {} missing",
            validation::detect_ckpe_config_kind(&config.fallout4_dir).file_name()
        )));
    }

    if let Some(ref fo4edit) = tools.fo4edit {
        let scripts_dir = fo4edit.parent().unwrap_or(exe_dir).join("Edit Scripts");
        validation::validate_required_xedit_scripts(&scripts_dir)?;
    }

    Ok(())
}

fn run_dry_run(cli: &Cli, fallout4_dir: PathBuf) {
    let mode = cli.build_mode();
    println!("Build mode: {}", mode.as_str());
    println!("Archiver: {}", cli.archive_tool().program_name());
    workflow::WorkflowEngine::<ScaffoldRunner>::print_resume_menu(mode);

    let config = dry_run_config(cli, fallout4_dir);
    let steps = workflow::WorkflowEngine::<ScaffoldRunner>::planned_steps(&config);
    println!("\nPlanned steps:");
    for step in &steps {
        println!("  {} - {}", step.number(), step.label());
    }

    let runnable = filter_runnable_steps(&steps);
    if runnable.len() < steps.len() {
        println!(
            "\nNote: only Step 1 is implemented; a full run would execute {} of {} planned steps.",
            runnable.len(),
            steps.len()
        );
    }
    if let Err(err) = assert_resume_step_implemented(&config) {
        println!("\nNote: {err}");
    }

    println!("\nDry run — external tools are not invoked.");
}

fn dry_run_config(cli: &Cli, fallout4_dir: PathBuf) -> generateprevisibines::ProjectConfig {
    use generateprevisibines::config::PluginIdentity;

    let plugin_name = cli.plugin.as_deref().unwrap_or("ExampleMod");
    let plugin = PluginIdentity::parse(plugin_name);
    let xedit_data_dir = cli.fo4_dir.as_ref().map(|d| d.join("Data"));

    generateprevisibines::ProjectConfig {
        build_mode: cli.build_mode(),
        archive_tool: cli.archive_tool(),
        fallout4_dir,
        plugin,
        non_interactive: cli.plugin.is_some(),
        resume_from: cli.resume_from,
        fo4edit_path: None,
        xedit_data_dir,
        ck_log_path: None,
    }
}

fn print_banner() {
    println!("============================================================================");
    println!("Automatic Previsbine Builder");
    println!("Reference: generateprevisibines.bat V2.96");
    println!("If you use MO2 then tools must be run from within MO2.");
    println!("============================================================================");
    println!();
}
