//! `GeneratePrevisibines` — Rust port (batch V2.96 reference).

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use generateprevisibines::{
    cli::Cli,
    discovery,
    intake::{WorkflowIntakeOutcome, WorkflowRequestIntake},
    run::{RunDiagnostic, WorkflowRun},
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
    if cli.dry_run {
        emit_run_diagnostics(&WorkflowRun::tool_diagnostics(&tools));
        run_dry_run(&cli);
        return Ok(());
    }

    let workflow_run = match WorkflowRequestIntake::interactive().resolve(&cli, &exe_dir, tools)? {
        WorkflowIntakeOutcome::Ready(run) => run,
        WorkflowIntakeOutcome::Exited => return Ok(()),
    };
    emit_run_diagnostics(workflow_run.diagnostics());
    workflow_run.execute()?;

    println!(
        "Build step(s) complete. Log: {}",
        workflow_run.log_path().display()
    );
    Ok(())
}

fn run_dry_run(cli: &Cli) {
    let mode = cli.build_mode();
    println!("Build mode: {}", mode.as_str());
    println!("Archiver: {}", cli.archive_tool().program_name());
    workflow::print_resume_menu(mode);

    let capability =
        WorkflowOperationExecutor::<ProductionOperationAdapters>::production_capability();
    let plan = WorkflowPlan::new(mode, cli.resume_from, capability);
    let steps = plan.as_ref().map_or_else(
        |_| WorkflowPlan::steps_for(mode, cli.resume_from),
        |plan| plan.planned_steps().to_vec(),
    );
    let runnable = plan
        .as_ref()
        .map_or_else(|_| Vec::new(), |plan| plan.runnable_steps().to_vec());

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
    if let Err(err) = plan {
        println!("\nNote: {err}");
    }

    println!("\nDry run — external tools are not invoked.");
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
