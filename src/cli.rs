//! CLI parsing compatible with batch V2.96 argument conventions.

use std::path::PathBuf;

use clap::Parser;

use crate::config::{ArchiveTool, BuildMode, PluginIdentity, ProjectConfig, WorkflowStep};
use crate::error::{Error, Result};
use crate::validation;

/// Command-line interface for GeneratePrevisibines.
///
/// Supports legacy batch-style flags (`-clean`, `-FO4:path`) as well as long options.
#[derive(Debug, Parser, Clone)]
#[command(
    name = "generateprevisibines",
    version,
    about = "Automate Fallout 4 precombine and previs generation (Rust port, scaffold)"
)]
pub struct Cli {
    /// Build mode: clean (default), filtered, or xbox.
    #[arg(long = "clean", conflicts_with_all = ["filtered", "xbox"])]
    pub clean: bool,

    #[arg(short = 'f', long = "filtered", conflicts_with_all = ["clean", "xbox"])]
    pub filtered: bool,

    #[arg(short = 'x', long = "xbox", conflicts_with_all = ["clean", "filtered"])]
    pub xbox: bool,

    /// Use BSArch instead of Archive2.
    #[arg(long = "bsarch")]
    pub bsarch: bool,

    /// Fallout 4 install directory (`-FO4:directory` in batch).
    #[arg(long = "FO4", value_name = "DIR")]
    pub fo4_dir: Option<PathBuf>,

    /// Plugin name (e.g. `MyMod` or `MyMod.esp`). Non-interactive when provided.
    #[arg(value_name = "PLUGIN")]
    pub plugin: Option<String>,

    /// Resume workflow from step 1–8 (non-interactive).
    #[arg(long, value_name = "N", value_parser = parse_resume_step)]
    pub resume_from: Option<WorkflowStep>,

    /// List planned workflow steps and exit (scaffold diagnostic).
    #[arg(long)]
    pub dry_run: bool,
}

fn parse_resume_step(s: &str) -> std::result::Result<WorkflowStep, String> {
    let n: u8 = s
        .parse()
        .map_err(|_| "resume step must be a number 1–8".to_string())?;
    WorkflowStep::from_number(n).ok_or_else(|| "resume step must be between 1 and 8".to_string())
}

impl Cli {
    #[must_use]
    pub fn build_mode(&self) -> BuildMode {
        if self.filtered {
            BuildMode::Filtered
        } else if self.xbox {
            BuildMode::Xbox
        } else {
            BuildMode::Clean
        }
    }

    #[must_use]
    pub fn archive_tool(&self) -> ArchiveTool {
        if self.bsarch {
            ArchiveTool::BSArch
        } else {
            ArchiveTool::Archive2
        }
    }
}

/// Parse raw argv the way the batch script does (up to four option tokens, any order).
///
/// Batch accepts `-clean`, `-filtered`, `-xbox`, `-bsarch`, `-FO4:dir`, and a bare plugin name.
pub fn parse_batch_style_args(args: &[&str]) -> Result<BatchParsedArgs> {
    let mut build_mode = BuildMode::Clean;
    let mut archive_tool = ArchiveTool::Archive2;
    let mut fo4_dir = None;
    let mut plugin = None;

    for arg in args {
        let stripped = arg.trim_matches('"');
        if stripped.is_empty() {
            continue;
        }

        if let Some(dir) = stripped
            .strip_prefix("-fo4:")
            .or_else(|| stripped.strip_prefix("-FO4:"))
        {
            fo4_dir = Some(PathBuf::from(dir));
            continue;
        }

        let flag = stripped.trim_start_matches('-').to_ascii_lowercase();
        match flag.as_str() {
            "clean" => build_mode = BuildMode::Clean,
            "filtered" => build_mode = BuildMode::Filtered,
            "xbox" => build_mode = BuildMode::Xbox,
            "bsarch" => archive_tool = ArchiveTool::BSArch,
            _ if stripped.starts_with('-') => {
                return Err(Error::InvalidParameter(stripped.to_string()));
            }
            _ => {
                validation::validate_plugin_name_token(stripped)?;
                plugin = Some(stripped.to_string());
            }
        }
    }

    Ok(BatchParsedArgs {
        build_mode,
        archive_tool,
        fo4_dir,
        plugin,
    })
}

#[derive(Debug, Clone)]
pub struct BatchParsedArgs {
    pub build_mode: BuildMode,
    pub archive_tool: ArchiveTool,
    pub fo4_dir: Option<PathBuf>,
    pub plugin: Option<String>,
}

impl Cli {
    /// Build a [`ProjectConfig`] after Fallout 4 directory resolution (CLI plugin required).
    pub fn into_project_config(self, fallout4_dir: PathBuf) -> Result<ProjectConfig> {
        let plugin_name = self
            .plugin
            .clone()
            .ok_or_else(|| Error::Other("plugin name required in non-interactive mode".into()))?;
        Self::project_config(
            fallout4_dir,
            self.build_mode(),
            self.archive_tool(),
            PluginIdentity::parse(&plugin_name),
            true,
            self.resume_from,
            self.fo4_dir.as_ref().map(|d| d.join("Data")),
            None,
            None,
        )
    }

    /// Shared config builder for CLI and interactive flows.
    #[allow(clippy::too_many_arguments)]
    pub fn project_config(
        fallout4_dir: PathBuf,
        build_mode: BuildMode,
        archive_tool: ArchiveTool,
        plugin: PluginIdentity,
        non_interactive: bool,
        resume_from: Option<WorkflowStep>,
        xedit_data_dir: Option<PathBuf>,
        fo4edit_path: Option<PathBuf>,
        ck_log_path: Option<PathBuf>,
    ) -> Result<ProjectConfig> {
        validation::validate_plugin(&plugin, build_mode)?;

        Ok(ProjectConfig {
            build_mode,
            archive_tool,
            fallout4_dir,
            plugin,
            non_interactive,
            resume_from,
            fo4edit_path,
            xedit_data_dir,
            ck_log_path,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batch_style_parses_flags_and_plugin() {
        let parsed =
            parse_batch_style_args(&["-filtered", "-bsarch", "-FO4:D:\\Games\\FO4", "MyMod"])
                .unwrap();
        assert_eq!(parsed.build_mode, BuildMode::Filtered);
        assert_eq!(parsed.archive_tool, ArchiveTool::BSArch);
        assert_eq!(
            parsed.fo4_dir.as_deref(),
            Some(std::path::Path::new("D:\\Games\\FO4"))
        );
        assert_eq!(parsed.plugin.as_deref(), Some("MyMod"));
    }

    #[test]
    fn batch_style_rejects_unknown_flag() {
        let err = parse_batch_style_args(&["-notreal"]).unwrap_err();
        assert!(matches!(err, Error::InvalidParameter(_)));
    }

    #[test]
    fn clap_build_mode_defaults_clean() {
        let cli = Cli::try_parse_from(["generateprevisibines"]).unwrap();
        assert_eq!(cli.build_mode(), BuildMode::Clean);
    }
}
