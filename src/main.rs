//! GeneratePrevisibines — Rust port (batch V2.96 reference).

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use generateprevisibines::interactive::ExistingPluginAction;
use generateprevisibines::{
    cli::Cli,
    discovery, interactive,
    run::{RunDiagnostic, WorkflowRequest, WorkflowRun},
    workflow::operations::{ProductionOperationAdapters, WorkflowOperationExecutor},
    workflow::{self, WorkflowPlan},
};
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

    let tools = WorkflowRun::discover_tools(&exe_dir, cli.fo4_dir.clone())?;
    let fallout4_dir = WorkflowRun::fallout4_dir(&tools)?;

    if cli.dry_run {
        emit_run_diagnostics(&WorkflowRun::tool_diagnostics(&tools));
        run_dry_run(&cli, fallout4_dir);
        return Ok(());
    }

    let Some(request) = resolve_request(&cli, &tools, fallout4_dir)? else {
        return Ok(());
    };

    let workflow_run = WorkflowRun::prepare(&request, &exe_dir, tools)?;
    emit_run_diagnostics(workflow_run.diagnostics());
    workflow_run.execute()?;

    println!(
        "Build step(s) complete. Log: {}",
        workflow_run.log_path().display()
    );
    Ok(())
}

/// Resolve plugin + request. `Ok(None)` when the user exits from interactive prompts.
fn resolve_request(
    cli: &Cli,
    tools: &discovery::ToolPaths,
    fallout4_dir: PathBuf,
) -> generateprevisibines::Result<Option<WorkflowRequest>> {
    let build_mode = cli.build_mode();
    let archive_tool = cli.archive_tool();
    let fo4edit_path = tools.fo4edit.clone();

    if let Some(plugin_name) = &cli.plugin {
        let request = WorkflowRequest::new(
            build_mode,
            archive_tool,
            generateprevisibines::config::PluginIdentity::parse(plugin_name),
            true,
            cli.resume_from,
            cli.fo4_dir.clone(),
        );
        let config = request.to_project_config(fallout4_dir, fo4edit_path, None)?;

        match interactive::ensure_plugin_ready(&config)? {
            ExistingPluginAction::Exit => return Ok(None),
            ExistingPluginAction::ChooseResumeStep => {
                return Err(generateprevisibines::Error::Other(
                    "resume step selection requires interactive mode".into(),
                ));
            }
            ExistingPluginAction::Continue => {}
        }

        return Ok(Some(request));
    }

    let mut resume_from = cli.resume_from;

    loop {
        let Some(plugin) = interactive::prompt_plugin_name(build_mode)? else {
            return Ok(None);
        };

        let mut request = WorkflowRequest::new(
            build_mode,
            archive_tool,
            plugin,
            false,
            resume_from,
            cli.fo4_dir.clone(),
        );
        let config = request.to_project_config(fallout4_dir.clone(), fo4edit_path.clone(), None)?;

        match interactive::ensure_plugin_ready(&config)? {
            ExistingPluginAction::Exit => return Ok(None),
            ExistingPluginAction::ChooseResumeStep => {
                if let Some(step) = interactive::prompt_resume_step(build_mode)? {
                    request.resume_from = Some(step);
                } else {
                    resume_from = None;
                    continue;
                }
            }
            ExistingPluginAction::Continue => {}
        }

        return Ok(Some(request));
    }
}

fn run_dry_run(cli: &Cli, fallout4_dir: PathBuf) {
    let mode = cli.build_mode();
    println!("Build mode: {}", mode.as_str());
    println!("Archiver: {}", cli.archive_tool().program_name());
    workflow::print_resume_menu(mode);

    let config = dry_run_config(cli, fallout4_dir);
    let capability = WorkflowOperationExecutor::<ProductionOperationAdapters>::capability();
    let steps = WorkflowPlan::planned_steps_for_config(&config);
    let runnable = capability.filter_steps(&steps);

    println!("\nPlanned steps:");
    for step in &steps {
        println!("  {} - {}", step.number(), step.label());
    }

    if runnable.len() < steps.len() {
        let runnable_summary = if runnable.is_empty() {
            "no planned steps".to_string()
        } else {
            runnable
                .iter()
                .map(|step| format!("Step {}", step.number()))
                .collect::<Vec<_>>()
                .join(", ")
        };
        println!(
            "\nNote: current production capability ({runnable_summary}) would execute {} of {} planned steps.",
            runnable.len(),
            steps.len()
        );
    }
    if let Err(err) = WorkflowPlan::new(&config, capability) {
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

fn emit_run_diagnostics(diagnostics: &[RunDiagnostic]) {
    for diagnostic in diagnostics {
        match diagnostic {
            RunDiagnostic::Fo4EditDiscovered(path) => {
                tracing::info!("{}", discovery::format_version_line("FO4Edit", path, None));
            }
            RunDiagnostic::Fallout4Directory(path) => {
                tracing::info!("Fallout 4 directory: {}", path.display());
            }
            RunDiagnostic::CkpeConfig {
                file_name,
                log_file,
            } => {
                tracing::info!("Using CKPE config: {file_name} (log: {log_file})");
            }
            RunDiagnostic::LaterStepsNotImplemented { skipped, .. } => {
                tracing::warn!(
                    skipped,
                    "Later workflow steps are not implemented yet; running Step 1 only."
                );
            }
        }
    }
}
