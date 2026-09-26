//! `GeneratePrevisibines` — Rust port (batch V2.98 reference).

use std::path::PathBuf;
use std::process::ExitCode;

use generateprevisibines::{
    cli::Cli,
    discovery,
    intake::{WorkflowIntakeOutcome, WorkflowRequestIntake},
    run::RunDiagnostic,
    toolchain::{ToolchainDiagnostic, WorkflowToolchainProbe},
    workflow::{self, WorkflowPlan, operations::production_workflow_plan},
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

    if cli.dry_run {
        run_dry_run(&cli);
        return Ok(());
    }

    let probe = WorkflowToolchainProbe::discover(&exe_dir, cli.fo4_dir.clone())?;
    let workflow_run = match WorkflowRequestIntake::production().resolve(&cli, &exe_dir, &probe)? {
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

    let plan = production_workflow_plan(mode, cli.resume_from);
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
    println!("Reference: generateprevisibines.bat V2.98");
    println!("If you use MO2 then tools must be run from within MO2.");
    println!("============================================================================");
    println!();
}

fn emit_run_diagnostics(diagnostics: &[RunDiagnostic]) {
    for diagnostic in diagnostics {
        match diagnostic {
            RunDiagnostic::Toolchain(toolchain) => {
                emit_toolchain_diagnostic(toolchain);
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

fn emit_toolchain_diagnostic(diagnostic: &ToolchainDiagnostic) {
    match diagnostic {
        ToolchainDiagnostic::Fo4EditDiscovered(path) => {
            tracing::info!("{}", discovery::format_version_line("FO4Edit", path, None));
        }
        ToolchainDiagnostic::Fallout4Directory(path) => {
            tracing::info!("Fallout 4 directory: {}", path.display());
        }
        ToolchainDiagnostic::CkpeConfig {
            file_name,
            log_file,
        } => {
            tracing::info!("Using CKPE config: {file_name} (log: {log_file})");
        }
        ToolchainDiagnostic::CkpeHandleLimitDisabled {
            file_name,
            setting_key,
        } => {
            tracing::warn!(
                "Increased Reference Limit not enabled, Precombine Step may fail. To fix, set {setting_key}=true in {file_name}."
            );
        }
    }
}
