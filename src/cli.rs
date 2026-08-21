//! CLI parsing compatible with batch V2.98 argument conventions.

use std::ffi::{OsStr, OsString};
use std::path::PathBuf;

use clap::{ArgAction, Args, CommandFactory, FromArgMatches, Parser};

use crate::config::{ArchiveTool, BuildMode, WorkflowStep};

/// Command-line interface for `GeneratePrevisibines`.
///
/// Compatibility-aware constructors accept legacy batch-style flags (`-clean`,
/// `-FO4:path`) as well as the modern Clap syntax.
#[derive(Debug, Clone)]
pub struct Cli {
    build_mode: BuildMode,
    archive_tool: ArchiveTool,
    /// Fallout 4 install directory.
    pub fo4_dir: Option<PathBuf>,
    /// Plugin name (for example, `MyMod` or `MyMod.esp`).
    pub plugin: Option<String>,
    /// Resume workflow from step 1–8 without prompting.
    pub resume_from: Option<WorkflowStep>,
    /// List planned workflow steps without discovering or running external tools.
    pub dry_run: bool,
}

/// Private Clap grammar used only after compatibility normalization.
#[derive(Debug, Parser, Clone)]
#[command(
    name = "generateprevisibines",
    version,
    about = "Automate Fallout 4 precombine and previs generation (Rust port, scaffold)"
)]
struct RawCli {
    #[command(flatten)]
    build_mode: BuildModeFlags,

    #[command(flatten)]
    archive_tool: ArchiveToolFlags,

    /// Fallout 4 install directory (`-FO4:directory` in batch).
    // The single-value action deliberately rejects repetitions, even when both paths match.
    #[arg(long = "FO4", value_name = "DIR", action = ArgAction::Set)]
    fo4_dir: Option<PathBuf>,

    /// Plugin name (e.g. `MyMod` or `MyMod.esp`). Non-interactive when provided.
    #[arg(value_name = "PLUGIN")]
    plugin: Option<String>,

    /// Resume workflow from step 1–8 (non-interactive).
    // Resume intent must be singular; accepting the last occurrence would hide ambiguous input.
    #[arg(
        long,
        value_name = "N",
        value_parser = parse_resume_step,
        action = ArgAction::Set
    )]
    resume_from: Option<WorkflowStep>,

    /// List planned workflow steps and exit (scaffold diagnostic).
    #[arg(long)]
    dry_run: bool,
}

#[derive(Debug, Args, Clone, Copy, Default)]
struct BuildModeFlags {
    // Self-overrides make aliases for one mode idempotent while conflicts still reject mixed modes.
    /// Build mode: clean (default).
    #[arg(
        short = 'c',
        long = "clean",
        overrides_with = "clean",
        conflicts_with_all = ["filtered", "xbox"]
    )]
    clean: bool,

    /// Build mode: filtered (skips PSG and CDX).
    #[arg(
        short = 'f',
        long = "filtered",
        overrides_with = "filtered",
        conflicts_with_all = ["clean", "xbox"]
    )]
    filtered: bool,

    /// Build mode: Xbox compression (also skips PSG and CDX).
    #[arg(
        short = 'x',
        long = "xbox",
        overrides_with = "xbox",
        conflicts_with_all = ["clean", "filtered"]
    )]
    xbox: bool,
}

impl BuildModeFlags {
    #[must_use]
    const fn selected(self) -> BuildMode {
        if self.filtered {
            BuildMode::Filtered
        } else if self.xbox {
            BuildMode::Xbox
        } else {
            BuildMode::Clean
        }
    }
}

#[derive(Debug, Args, Clone, Copy, Default)]
struct ArchiveToolFlags {
    /// Use `BSArch` instead of Archive2.
    // Unlike same-mode aliases, duplicate archive selectors are ambiguous and must remain errors.
    #[arg(long = "bsarch", action = ArgAction::SetTrue)]
    bsarch: bool,
}

impl ArchiveToolFlags {
    #[must_use]
    const fn selected(self) -> ArchiveTool {
        if self.bsarch {
            ArchiveTool::BSArch
        } else {
            ArchiveTool::Archive2
        }
    }
}

fn parse_resume_step(s: &str) -> std::result::Result<WorkflowStep, String> {
    let n: u8 = s
        .parse()
        .map_err(|_| "resume step must be a number 1–8".to_string())?;
    WorkflowStep::from_number(n).ok_or_else(|| "resume step must be between 1 and 8".to_string())
}

impl From<RawCli> for Cli {
    fn from(raw: RawCli) -> Self {
        Self {
            build_mode: raw.build_mode.selected(),
            archive_tool: raw.archive_tool.selected(),
            fo4_dir: raw.fo4_dir,
            plugin: raw.plugin,
            resume_from: raw.resume_from,
            dry_run: raw.dry_run,
        }
    }
}

impl Cli {
    /// Parse the process argument vector after normalizing legacy batch tokens.
    ///
    /// Clap retains ownership of help, version, usage, and parse-error exits.
    #[must_use]
    pub fn parse() -> Self {
        Self::parse_from(std::env::args_os())
    }

    /// Parse a supplied native argument vector after normalizing legacy batch tokens.
    ///
    /// Like Clap's `Parser::parse_from`, this exits the process for help, version,
    /// or invalid arguments.
    #[must_use]
    pub fn parse_from<I, T>(args: I) -> Self
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString>,
    {
        Self::try_parse_from(args).unwrap_or_else(|error| error.exit())
    }

    /// Try to parse a native argument vector after normalizing legacy batch tokens.
    ///
    /// Parse failures are returned as Clap errors with Clap-generated usage text.
    pub fn try_parse_from<I, T>(args: I) -> std::result::Result<Self, clap::Error>
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString>,
    {
        try_parse_normalized_args(normalize_compatible_args(args)).map(Into::into)
    }

    /// Return the selected build mode, defaulting to clean.
    #[must_use]
    pub const fn build_mode(&self) -> BuildMode {
        self.build_mode
    }

    /// Return the selected archive backend, defaulting to Archive2.
    #[must_use]
    pub const fn archive_tool(&self) -> ArchiveTool {
        self.archive_tool
    }
}

/// Parse normalized arguments while retaining the built command for complete Clap diagnostics.
fn try_parse_normalized_args(args: Vec<OsString>) -> std::result::Result<RawCli, clap::Error> {
    let mut command = RawCli::command();
    let mut matches = match command.try_get_matches_from_mut(args) {
        Ok(matches) => matches,
        Err(error) => return Err(with_usage_context(error, &mut command)),
    };

    RawCli::from_arg_matches_mut(&mut matches).map_err(|error| {
        let formatted_error = error.format(&mut command);
        with_usage_context(formatted_error, &mut command)
    })
}

/// Add generated usage to a failing Clap diagnostic when its error kind omits that context.
fn with_usage_context(mut error: clap::Error, command: &mut clap::Command) -> clap::Error {
    use clap::error::{ContextKind, ContextValue};

    // Clap omits usage for value-validation and empty-value errors. Filling only absent context
    // keeps its original error kind and message while making every command-line failure actionable.
    if error.use_stderr() && error.get(ContextKind::Usage).is_none() {
        error.insert(
            ContextKind::Usage,
            ContextValue::StyledStr(command.render_usage()),
        );
    }

    error
}

/// Normalize legacy option spellings before the private Clap grammar sees them.
fn normalize_compatible_args<I, T>(args: I) -> Vec<OsString>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString>,
{
    let mut args = args.into_iter().map(Into::into);
    let mut normalized = Vec::new();

    if let Some(program) = args.next() {
        normalized.push(program);
    }

    for arg in args {
        if os_str_eq_ignore_ascii_case(&arg, "-clean") {
            normalized.push(OsString::from("--clean"));
        } else if os_str_eq_ignore_ascii_case(&arg, "-filtered") {
            normalized.push(OsString::from("--filtered"));
        } else if os_str_eq_ignore_ascii_case(&arg, "-xbox") {
            normalized.push(OsString::from("--xbox"));
        } else if os_str_eq_ignore_ascii_case(&arg, "-bsarch") {
            normalized.push(OsString::from("--bsarch"));
        } else if let Some(fo4_dir) = strip_ascii_prefix_ignore_case(&arg, "-FO4:") {
            // Keep the value attached so Clap cannot reinterpret a dash-prefixed path as an option.
            let mut normalized_fo4 = OsString::from("--FO4=");
            normalized_fo4.push(fo4_dir);
            normalized.push(normalized_fo4);
        } else {
            normalized.push(arg);
        }
    }

    normalized
}

fn os_str_eq_ignore_ascii_case(value: &OsStr, expected: &str) -> bool {
    strip_ascii_prefix_ignore_case(value, expected).is_some_and(|suffix| suffix.is_empty())
}

// The batch form embeds the path in one token. Splitting platform-native units avoids rejecting
// or corrupting attached paths that cannot round-trip through UTF-8.
/// Strip an ASCII option prefix without converting the native argument suffix to UTF-8.
#[cfg(unix)]
fn strip_ascii_prefix_ignore_case(value: &OsStr, prefix: &str) -> Option<OsString> {
    use std::os::unix::ffi::{OsStrExt, OsStringExt};

    let bytes = value.as_bytes();
    bytes
        .get(..prefix.len())
        .filter(|candidate| candidate.eq_ignore_ascii_case(prefix.as_bytes()))
        .map(|_| OsString::from_vec(bytes[prefix.len()..].to_vec()))
}

/// Strip an ASCII option prefix without converting the native argument suffix to UTF-8.
#[cfg(windows)]
fn strip_ascii_prefix_ignore_case(value: &OsStr, prefix: &str) -> Option<OsString> {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};

    let units = value.encode_wide().collect::<Vec<_>>();
    let prefix_units = prefix.encode_utf16().collect::<Vec<_>>();
    let matches = units
        .get(..prefix_units.len())?
        .iter()
        .zip(&prefix_units)
        .all(
            |(&candidate, &expected)| match (u8::try_from(candidate), u8::try_from(expected)) {
                (Ok(candidate), Ok(expected)) => candidate.eq_ignore_ascii_case(&expected),
                _ => false,
            },
        );

    matches.then(|| OsString::from_wide(&units[prefix_units.len()..]))
}

#[cfg(test)]
mod tests {
    use clap::error::ErrorKind;

    use super::*;

    const BUILD_MODE_ALIASES_AND_LEGACY_CASE_VARIANTS: &[(BuildMode, &[&str])] = &[
        (BuildMode::Clean, &["--clean", "-c", "-clean", "-ClEaN"]),
        (
            BuildMode::Filtered,
            &["--filtered", "-f", "-filtered", "-FiLtErEd"],
        ),
        (BuildMode::Xbox, &["--xbox", "-x", "-xbox", "-XbOx"]),
    ];

    const BSARCH_ALIASES_AND_LEGACY_CASE_VARIANTS: &[&str] = &["--bsarch", "-bsarch", "-BsArCh"];

    /// Return the Clap error for an invalid public-parser invocation and verify usage context.
    fn clap_error_with_usage(args: &[&str]) -> clap::Error {
        let error = Cli::try_parse_from(args.iter().copied()).unwrap_err();
        let diagnostic = error.to_string();
        assert!(
            diagnostic.contains("Usage:"),
            "missing usage context for {args:?}: {diagnostic}"
        );
        error
    }

    #[test]
    fn every_build_mode_alias_and_legacy_case_variant_parses() {
        for &(expected, aliases) in BUILD_MODE_ALIASES_AND_LEGACY_CASE_VARIANTS {
            for &alias in aliases {
                let cli = Cli::try_parse_from(["generateprevisibines", alias]).unwrap();
                assert_eq!(cli.build_mode(), expected, "alias: {alias}");
            }
        }
    }

    #[test]
    fn legacy_choices_are_case_insensitive() {
        for &alias in BSARCH_ALIASES_AND_LEGACY_CASE_VARIANTS {
            let cli = Cli::try_parse_from(["generateprevisibines", alias]).unwrap();
            assert_eq!(cli.archive_tool(), ArchiveTool::BSArch, "alias: {alias}");
        }

        for args in [
            ["generateprevisibines", "-FO4:D:\\Games\\Fallout 4"],
            ["generateprevisibines", "-fO4:D:\\Games\\Fallout 4"],
        ] {
            let cli = Cli::try_parse_from(args).unwrap();
            assert_eq!(
                cli.fo4_dir.as_deref(),
                Some(std::path::Path::new(r"D:\Games\Fallout 4")),
                "arguments: {args:?}"
            );
        }
    }

    #[test]
    fn plugin_can_be_supplied_or_omitted() {
        let omitted = Cli::try_parse_from(["generateprevisibines"]).unwrap();
        assert_eq!(omitted.plugin, None);

        let supplied = Cli::try_parse_from(["generateprevisibines", "MyMod.esp"]).unwrap();
        assert_eq!(supplied.plugin.as_deref(), Some("MyMod.esp"));
    }

    #[test]
    fn modern_choices_parse_in_an_invocation_longer_than_four_tokens() {
        let cli = Cli::try_parse_from([
            "generateprevisibines",
            "--clean",
            "--bsarch",
            "--FO4",
            r"D:\Games\Fallout 4",
            "--resume-from",
            "6",
            "--dry-run",
            "MyMod.esp",
        ])
        .unwrap();

        assert_eq!(cli.build_mode(), BuildMode::Clean);
        assert_eq!(cli.archive_tool(), ArchiveTool::BSArch);
        assert_eq!(
            cli.fo4_dir.as_deref(),
            Some(std::path::Path::new(r"D:\Games\Fallout 4"))
        );
        assert_eq!(cli.plugin.as_deref(), Some("MyMod.esp"));
        assert_eq!(cli.resume_from, Some(WorkflowStep::GeneratePrevis));
        assert!(cli.dry_run);
    }

    #[test]
    fn legacy_and_modern_choices_can_be_mixed() {
        let cli = Cli::try_parse_from([
            "generateprevisibines",
            "-filtered",
            "--bsarch",
            r"-FO4:D:\Games\Fallout 4",
            "--resume-from",
            "6",
            "MyMod.esp",
            "--dry-run",
        ])
        .unwrap();

        assert_eq!(cli.build_mode(), BuildMode::Filtered);
        assert_eq!(cli.archive_tool(), ArchiveTool::BSArch);
        assert_eq!(
            cli.fo4_dir.as_deref(),
            Some(std::path::Path::new(r"D:\Games\Fallout 4"))
        );
        assert_eq!(cli.plugin.as_deref(), Some("MyMod.esp"));
        assert_eq!(cli.resume_from, Some(WorkflowStep::GeneratePrevis));
        assert!(cli.dry_run);
    }

    #[test]
    fn repeated_aliases_for_the_same_build_mode_are_idempotent() {
        for &(expected, aliases) in BUILD_MODE_ALIASES_AND_LEGACY_CASE_VARIANTS {
            for &first in aliases {
                for &second in aliases {
                    let cli = Cli::try_parse_from(["generateprevisibines", first, second]).unwrap();
                    assert_eq!(
                        cli.build_mode(),
                        expected,
                        "same-mode aliases: {first}, {second}"
                    );
                }
            }
        }
    }

    #[test]
    fn different_build_modes_conflict_in_any_order_or_spelling() {
        for (left_index, &(_, left_aliases)) in BUILD_MODE_ALIASES_AND_LEGACY_CASE_VARIANTS
            .iter()
            .enumerate()
        {
            for &(_, right_aliases) in
                &BUILD_MODE_ALIASES_AND_LEGACY_CASE_VARIANTS[left_index + 1..]
            {
                for &left in left_aliases {
                    for &right in right_aliases {
                        for (first, second) in [(left, right), (right, left)] {
                            let error =
                                clap_error_with_usage(&["generateprevisibines", first, second]);
                            assert_eq!(
                                error.kind(),
                                ErrorKind::ArgumentConflict,
                                "conflicting aliases: {first}, {second}"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn repeated_bsarch_aliases_are_rejected() {
        for &first in BSARCH_ALIASES_AND_LEGACY_CASE_VARIANTS {
            for &second in BSARCH_ALIASES_AND_LEGACY_CASE_VARIANTS {
                let error = clap_error_with_usage(&["generateprevisibines", first, second]);
                assert_eq!(
                    error.kind(),
                    ErrorKind::ArgumentConflict,
                    "repeated BSArch aliases: {first}, {second}"
                );
            }
        }
    }

    #[test]
    fn repeated_fallout4_overrides_are_rejected() {
        for args in [
            &[
                "generateprevisibines",
                r"-FO4:D:\Games\Fallout 4",
                r"-fO4:D:\Games\Fallout 4",
            ][..],
            &[
                "generateprevisibines",
                r"-FO4:D:\Games\Fallout 4",
                r"-FO4:E:\Fallout 4",
            ],
            &[
                "generateprevisibines",
                "--FO4",
                r"D:\Games\Fallout 4",
                "--FO4",
                r"D:\Games\Fallout 4",
            ],
            &[
                "generateprevisibines",
                "--FO4",
                r"D:\Games\Fallout 4",
                r"-FO4:E:\Fallout 4",
            ],
            &[
                "generateprevisibines",
                r"-FO4:D:\Games\Fallout 4",
                "--FO4",
                r"E:\Fallout 4",
            ],
        ] {
            let error = clap_error_with_usage(args);
            assert_eq!(error.kind(), ErrorKind::ArgumentConflict);
        }
    }

    #[test]
    fn repeated_resume_steps_are_rejected() {
        for args in [
            [
                "generateprevisibines",
                "--resume-from",
                "6",
                "--resume-from",
                "6",
            ],
            [
                "generateprevisibines",
                "--resume-from",
                "3",
                "--resume-from",
                "7",
            ],
        ] {
            let error = clap_error_with_usage(&args);
            assert_eq!(error.kind(), ErrorKind::ArgumentConflict);
        }
    }

    #[test]
    fn empty_fallout4_overrides_are_rejected_with_usage() {
        for args in [
            &["generateprevisibines", "-FO4:"][..],
            &["generateprevisibines", "-fO4:"],
            &["generateprevisibines", "--FO4", ""],
            &["generateprevisibines", "--FO4="],
        ] {
            clap_error_with_usage(args);
        }
    }

    #[test]
    fn attached_legacy_fo4_accepts_a_dash_prefixed_path() {
        let cli = Cli::try_parse_from(["generateprevisibines", "-FO4:-relative"]).unwrap();

        assert_eq!(
            cli.fo4_dir.as_deref(),
            Some(std::path::Path::new("-relative"))
        );
    }

    #[test]
    fn build_mode_defaults_to_clean() {
        let cli = Cli::try_parse_from(["generateprevisibines"]).unwrap();
        assert_eq!(cli.build_mode(), BuildMode::Clean);
    }

    #[test]
    fn unknown_options_and_multiple_plugins_use_clap_errors() {
        for args in [
            ["generateprevisibines", "-notreal"],
            ["generateprevisibines", "--not-real"],
        ] {
            let unknown = clap_error_with_usage(&args);
            assert_eq!(unknown.kind(), ErrorKind::UnknownArgument);
        }

        let multiple_plugins =
            clap_error_with_usage(&["generateprevisibines", "FirstMod", "--dry-run", "SecondMod"]);
        assert!(matches!(
            multiple_plugins.kind(),
            ErrorKind::UnknownArgument | ErrorKind::TooManyValues
        ));
    }

    #[test]
    fn invalid_resume_steps_use_clap_errors_with_usage() {
        for value in ["0", "9", "not-a-number"] {
            let error = clap_error_with_usage(&["generateprevisibines", "--resume-from", value]);
            assert_eq!(error.kind(), ErrorKind::ValueValidation, "value: {value}");
        }
    }

    #[test]
    fn help_and_version_remain_clap_owned() {
        let help = Cli::try_parse_from(["generateprevisibines", "--help"]).unwrap_err();
        assert_eq!(help.kind(), ErrorKind::DisplayHelp);
        assert!(help.to_string().contains("Usage:"));

        let version = Cli::try_parse_from(["generateprevisibines", "--version"]).unwrap_err();
        assert_eq!(version.kind(), ErrorKind::DisplayVersion);
    }

    #[cfg(windows)]
    #[test]
    fn attached_legacy_fo4_preserves_native_windows_path_units() {
        use std::os::windows::ffi::OsStringExt;

        let native_path_units = vec![
            u16::from(b'D'),
            u16::from(b':'),
            u16::from(b'\\'),
            0xD800,
            u16::from(b'\\'),
            u16::from(b'F'),
            u16::from(b'O'),
            u16::from(b'4'),
        ];
        let native_path = OsString::from_wide(&native_path_units);
        let mut attached_units = "-fO4:".encode_utf16().collect::<Vec<_>>();
        attached_units.extend_from_slice(&native_path_units);

        let cli = Cli::try_parse_from(vec![
            OsString::from("generateprevisibines"),
            OsString::from_wide(&attached_units),
        ])
        .unwrap();

        assert_eq!(cli.fo4_dir, Some(PathBuf::from(native_path)));
    }

    #[cfg(unix)]
    #[test]
    fn attached_legacy_fo4_preserves_native_unix_path_bytes() {
        use std::os::unix::ffi::OsStringExt;

        let native_path_bytes = vec![b'/', b'f', b'o', 0x80, b'4'];
        let native_path = OsString::from_vec(native_path_bytes.clone());
        let mut attached_bytes = b"-fO4:".to_vec();
        attached_bytes.extend_from_slice(&native_path_bytes);

        let cli = Cli::try_parse_from(vec![
            OsString::from("generateprevisibines"),
            OsString::from_vec(attached_bytes),
        ])
        .unwrap();

        assert_eq!(cli.fo4_dir, Some(PathBuf::from(native_path)));
    }
}
