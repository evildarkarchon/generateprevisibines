//! Workflow Toolchain preparation.
//!
//! This module owns external-tool readiness before Workflow Operations run.

use std::path::{Path, PathBuf};

use crate::config::{ArchiveTool, CkpeConfigKind, PluginIdentity};
use crate::discovery::{self, ToolPaths};
use crate::error::{Error, Result};
use crate::tools::{CreationKitPaths, Fo4EditPaths};
use crate::validation;

/// Pre-intake toolchain facts needed before a Workflow Request becomes a Workflow Run.
#[derive(Debug, Clone)]
pub struct WorkflowToolchainProbe {
    tools: ToolPaths,
    fallout4_dir: PathBuf,
    data_dir: PathBuf,
    /// Whether the Fallout 4 directory came from `-FO4:` rather than the registry.
    ///
    /// Recorded because FO4Edit is pointed at `Data` with `-D:` only then: the batch sets
    /// `%ModDir_%` only in `:SetFO4` (line 498), and otherwise FO4Edit finds the install itself.
    fallout4_dir_overridden: bool,
    diagnostics: Vec<ToolchainDiagnostic>,
}

impl WorkflowToolchainProbe {
    /// Discover the Fallout 4 data root and best-effort tool paths for later preparation.
    ///
    /// This is intentionally light: it must resolve the data root used for plugin readiness,
    /// but it does not require Creation Kit, CKPE, xEdit scripts, or archive tools yet.
    pub fn discover(exe_dir: &Path, fallout4_override: Option<PathBuf>) -> Result<Self> {
        let overridden = fallout4_override.is_some();
        let mut probe =
            Self::from_tool_paths(discovery::discover_tools(exe_dir, fallout4_override))?;
        probe.fallout4_dir_overridden = overridden;
        Ok(probe)
    }

    /// Build a probe from already-discovered tool paths.
    ///
    /// Tests use this constructor to avoid registry discovery while still exercising the
    /// same readiness logic as production.
    pub fn from_tool_paths(tools: ToolPaths) -> Result<Self> {
        // Reached whenever the batch's `locCreationKit_` would still be empty: the registry had
        // no answer and no `-FO4:<dir>` was given. Both of batch line 63's remedies must survive
        // the port, because a registry-less host (Wine) and a never-launched install both land
        // here and neither is fixable from a bare registry error. `main` adds `ERROR - `.
        //
        // The wording says "directory", not line 63's "Fallout4.exe", on purpose: this is only a
        // did-we-resolve-a-directory test. Line 63's actual `Exist` check on `Fallout4.exe` has
        // no counterpart here, so claiming it would be a lie for a stale or typo'd directory,
        // which reaches the `CreationKit.exe` check below instead.
        let fallout4_dir = tools.fallout4_dir.clone().ok_or_else(|| {
            Error::Other(
                "Fallout 4 directory could not be determined. To fix, run Fallout4Launcher.exe \
                 once, or use --FO4 <DIR> to specify the location of Fallout4.exe."
                    .into(),
            )
        })?;
        let data_dir = fallout4_dir.join("Data");

        let mut diagnostics = Vec::new();
        if let Some(ref fo4edit) = tools.fo4edit {
            diagnostics.push(ToolchainDiagnostic::Fo4EditDiscovered(fo4edit.clone()));
        }
        diagnostics.push(ToolchainDiagnostic::Fallout4Directory(fallout4_dir.clone()));

        Ok(Self {
            tools,
            fallout4_dir,
            data_dir,
            // Tool paths alone cannot say where the directory came from; `discover`, which was
            // handed the override, sets this afterwards.
            fallout4_dir_overridden: false,
            diagnostics,
        })
    }

    /// Create the narrow plugin-readiness view used by Workflow Request Intake prompts.
    #[must_use]
    pub fn plugin_readiness(
        &self,
        plugin: &PluginIdentity,
        non_interactive: bool,
    ) -> PluginReadiness {
        PluginReadiness::new(plugin.clone(), self.data_dir.clone(), non_interactive)
    }

    /// Validate external-tool readiness for the runnable Workflow Operations.
    pub fn prepare(
        &self,
        exe_dir: &Path,
        archive_tool: ArchiveTool,
        requirements: ToolchainRequirements,
    ) -> Result<WorkflowToolchain> {
        let mut diagnostics = self.diagnostics.clone();

        let creation_kit = if requirements.needs_creation_kit() {
            let creation_kit = self.tools.creation_kit.clone().ok_or_else(|| {
                Error::Other(format!(
                    "CreationKit.exe not found in {}",
                    self.fallout4_dir.display()
                ))
            })?;
            let ckpe = load_ckpe_installation(&self.fallout4_dir)?;
            diagnostics.push(ToolchainDiagnostic::CkpeConfig {
                file_name: ckpe.file_name.clone(),
                log_file: ckpe.log_file.clone(),
            });
            if !ckpe.handle_limit_enabled {
                diagnostics.push(ToolchainDiagnostic::CkpeHandleLimitDisabled {
                    file_name: ckpe.file_name.clone(),
                    setting_key: ckpe.handle_setting_key.clone(),
                });
            }
            Some(CreationKitToolchain {
                executable: creation_kit,
                ck_log_path: ckpe.log_path,
            })
        } else {
            None
        };

        let fo4edit = if requirements.needs_fo4edit() {
            let fo4edit = self.tools.fo4edit.clone().ok_or_else(|| {
                Error::Other(
                    "FO4Edit/xEdit directory not found. Run this program from its directory or install xEdit."
                        .to_string(),
                )
            })?;
            validate_xedit_scripts(exe_dir, &fo4edit)?;
            Some(Fo4EditToolchain {
                executable: fo4edit,
                data_dir_override: self.fallout4_dir_overridden.then(|| self.data_dir.clone()),
            })
        } else {
            None
        };

        let archive = if requirements.needs_archive() {
            Some(required_archive_tool(&self.tools, archive_tool)?)
        } else {
            None
        };

        Ok(WorkflowToolchain {
            fallout4_dir: self.fallout4_dir.clone(),
            data_dir: self.data_dir.clone(),
            creation_kit,
            discovered_fo4edit: self.tools.fo4edit.clone(),
            fo4edit,
            archive,
            diagnostics,
        })
    }

    /// Resolved Fallout 4 install directory used by this probe.
    #[must_use]
    pub fn fallout4_dir(&self) -> &Path {
        &self.fallout4_dir
    }

    /// Resolved Fallout 4 data directory used for plugin readiness and artifacts.
    #[must_use]
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// Best-effort diagnostics available before full toolchain preparation.
    #[must_use]
    pub fn diagnostics(&self) -> &[ToolchainDiagnostic] {
        &self.diagnostics
    }
}

/// Narrow plugin-readiness view used before a Workflow Run exists.
#[derive(Debug, Clone)]
pub struct PluginReadiness {
    plugin: PluginIdentity,
    data_dir: PathBuf,
    non_interactive: bool,
}

impl PluginReadiness {
    /// Create a plugin-readiness view from a plugin identity and data root.
    #[must_use]
    pub fn new(plugin: PluginIdentity, data_dir: PathBuf, non_interactive: bool) -> Self {
        Self {
            plugin,
            data_dir,
            non_interactive,
        }
    }

    /// Plugin file name that will be checked or copied during intake.
    #[must_use]
    pub fn plugin_file_name(&self) -> &str {
        &self.plugin.file_name
    }

    /// Whether intake is running without interactive prompts.
    #[must_use]
    pub const fn non_interactive(&self) -> bool {
        self.non_interactive
    }

    /// Data directory that contains plugins and seed files.
    #[must_use]
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// Full path to the target plugin file.
    #[must_use]
    pub fn plugin_path(&self) -> PathBuf {
        self.data_dir.join(&self.plugin.file_name)
    }

    /// Full path to the plugin's main BA2 archive.
    #[must_use]
    pub fn plugin_archive_path(&self) -> PathBuf {
        self.data_dir.join(self.plugin.archive_name())
    }
}

/// Prepared external-tool facts for a Workflow Run.
#[derive(Debug, Clone)]
pub struct WorkflowToolchain {
    fallout4_dir: PathBuf,
    data_dir: PathBuf,
    creation_kit: Option<CreationKitToolchain>,
    /// The FO4Edit discovery found, whether or not any runnable operation required it.
    discovered_fo4edit: Option<PathBuf>,
    /// FO4Edit as prepared for the runnable operations; `None` when none required it.
    fo4edit: Option<Fo4EditToolchain>,
    archive: Option<PathBuf>,
    diagnostics: Vec<ToolchainDiagnostic>,
}

impl WorkflowToolchain {
    /// Resolved Fallout 4 install directory.
    #[must_use]
    pub fn fallout4_dir(&self) -> &Path {
        &self.fallout4_dir
    }

    /// Resolved data directory used by workflow artifacts.
    #[must_use]
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// Diagnostics collected while preparing the toolchain.
    #[must_use]
    pub fn diagnostics(&self) -> &[ToolchainDiagnostic] {
        &self.diagnostics
    }

    /// The FO4Edit/xEdit executable: the prepared one when xEdit-backed operations require it,
    /// otherwise whatever discovery found.
    #[must_use]
    pub fn fo4edit_path(&self) -> Option<&Path> {
        self.fo4edit
            .as_ref()
            .map(|fo4edit| fo4edit.executable.as_path())
            .or(self.discovered_fo4edit.as_deref())
    }

    /// Prepared archive executable path when archive-backed operations require it.
    #[must_use]
    pub fn archive_path(&self) -> Option<&Path> {
        self.archive.as_deref()
    }

    /// Resolve the paths a CK-backed Workflow Operation's Creation Kit episodes run against.
    ///
    /// `session_log` is the Workflow Run's own log, which each episode folds its Creation Kit
    /// log into. Returns [`Error::CreationKitNotPrepared`] when Creation Kit readiness was never
    /// required, and so never prepared, for this Workflow Run.
    pub(crate) fn creation_kit_paths(&self, session_log: PathBuf) -> Result<CreationKitPaths> {
        let creation_kit = self
            .creation_kit
            .as_ref()
            .ok_or(Error::CreationKitNotPrepared)?;

        Ok(CreationKitPaths {
            exe: creation_kit.executable.clone(),
            fallout4_dir: self.fallout4_dir.clone(),
            ck_log_path: creation_kit.ck_log_path.clone(),
            session_log,
        })
    }

    /// Resolve the paths an xEdit-backed Workflow Operation's FO4Edit episodes run against.
    ///
    /// `session_log` is the Workflow Run's own log, which each episode folds the script's log
    /// into. Returns [`Error::Fo4EditNotPrepared`] when FO4Edit readiness was never required,
    /// and so never prepared, for this Workflow Run.
    pub(crate) fn fo4edit_paths(&self, session_log: PathBuf) -> Result<Fo4EditPaths> {
        let fo4edit = self.fo4edit.as_ref().ok_or(Error::Fo4EditNotPrepared)?;

        Ok(Fo4EditPaths {
            exe: fo4edit.executable.clone(),
            data_dir_override: fo4edit.data_dir_override.clone(),
            session_log,
        })
    }
}

/// Toolchain facts and warnings surfaced to presentation code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolchainDiagnostic {
    Fo4EditDiscovered(PathBuf),
    Fallout4Directory(PathBuf),
    CkpeConfig {
        file_name: String,
        log_file: String,
    },
    CkpeHandleLimitDisabled {
        file_name: String,
        setting_key: String,
    },
}

/// External-tool requirements for a set of runnable Workflow Operations.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ToolchainRequirements {
    creation_kit: bool,
    fo4edit: bool,
    archive: bool,
}

impl ToolchainRequirements {
    /// Create an empty requirement set.
    #[must_use]
    pub const fn none() -> Self {
        Self {
            creation_kit: false,
            fo4edit: false,
            archive: false,
        }
    }

    /// Create the static readiness requirement for a Creation Kit-backed operation.
    #[must_use]
    pub const fn creation_kit() -> Self {
        Self {
            creation_kit: true,
            fo4edit: false,
            archive: false,
        }
    }

    /// Create the static readiness requirement for an FO4Edit-backed operation.
    #[must_use]
    pub const fn fo4edit() -> Self {
        Self {
            creation_kit: false,
            fo4edit: true,
            archive: false,
        }
    }

    /// Combine readiness categories required by multiple Workflow Operations.
    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self {
            creation_kit: self.creation_kit || other.creation_kit,
            fo4edit: self.fo4edit || other.fo4edit,
            archive: self.archive || other.archive,
        }
    }

    /// Mark Creation Kit and CKPE as required.
    pub const fn require_creation_kit(&mut self) {
        self.creation_kit = true;
    }

    /// Mark FO4Edit/xEdit and required scripts as required.
    pub const fn require_fo4edit(&mut self) {
        self.fo4edit = true;
    }

    /// Mark the configured archive tool as required.
    pub const fn require_archive(&mut self) {
        self.archive = true;
    }

    /// Whether Creation Kit and CKPE readiness is required.
    #[must_use]
    pub const fn needs_creation_kit(self) -> bool {
        self.creation_kit
    }

    /// Whether FO4Edit/xEdit readiness is required.
    #[must_use]
    pub const fn needs_fo4edit(self) -> bool {
        self.fo4edit
    }

    /// Whether archive tool readiness is required.
    #[must_use]
    pub const fn needs_archive(self) -> bool {
        self.archive
    }
}

#[derive(Debug, Clone)]
struct CreationKitToolchain {
    executable: PathBuf,
    ck_log_path: PathBuf,
}

/// FO4Edit as prepared for a Workflow Run whose runnable operations require it.
#[derive(Debug, Clone)]
struct Fo4EditToolchain {
    executable: PathBuf,
    /// The run's `Data` directory when `-FO4:` was given; see [`Fo4EditPaths`].
    data_dir_override: Option<PathBuf>,
}

#[derive(Debug, Clone)]
struct CkpeInstallation {
    file_name: String,
    log_file: String,
    log_path: PathBuf,
    handle_setting_key: String,
    handle_limit_enabled: bool,
}

fn load_ckpe_installation(fallout4_dir: &Path) -> Result<CkpeInstallation> {
    let kind = validation::detect_ckpe_config_kind(fallout4_dir);
    let file_name = kind.file_name().to_string();
    let ckpe_path = fallout4_dir.join(&file_name);

    if !ckpe_path.is_file() {
        return Err(Error::CkpeConfig(format!(
            "CKPE not configured. File {file_name} missing"
        )));
    }

    let contents = crate::text::read_lossy(&ckpe_path)?;
    let log_file = parse_ckpe_log_file(kind, &contents)?;
    let handle_setting_key = kind.handle_setting_key().to_string();
    let handle_limit_enabled =
        validation::ckpe_handle_limit_enabled(&contents, &handle_setting_key);

    Ok(CkpeInstallation {
        file_name,
        log_path: resolve_ck_log_path(fallout4_dir, &log_file),
        log_file,
        handle_setting_key,
        handle_limit_enabled,
    })
}

fn parse_ckpe_log_file(kind: CkpeConfigKind, contents: &str) -> Result<String> {
    let log_key = kind.log_setting_key();
    validation::parse_ck_log_setting(contents, log_key).ok_or_else(|| {
        Error::CkpeConfig(format!(
            "CK logging not set in this ini. To fix, set {log_key}=CK.log in it."
        ))
    })
}

fn resolve_ck_log_path(fallout4_dir: &Path, log_setting: &str) -> PathBuf {
    let path = Path::new(log_setting);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        fallout4_dir.join(path)
    }
}

fn validate_xedit_scripts(exe_dir: &Path, fo4edit: &Path) -> Result<()> {
    let scripts_dir = fo4edit.parent().unwrap_or(exe_dir).join("Edit Scripts");
    validation::validate_required_xedit_scripts(&scripts_dir)
}

/// Write an FO4Edit install into `dir` that FO4Edit readiness accepts, and return its exe.
///
/// `FO4Edit.exe` beside an `Edit Scripts` folder holding both merge scripts at their required
/// versions. Crate-visible for every test module that prepares a Workflow Run whose runnable
/// steps include a merge step — since Step 2 was registered, that is any fresh Clean run —
/// because readiness is probed on the real disk, not through a seam.
#[cfg(test)]
pub(crate) fn write_fo4edit_install(dir: &Path) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    let fo4edit = dir.join("FO4Edit.exe");
    std::fs::write(&fo4edit, b"").unwrap();
    let scripts = dir.join("Edit Scripts");
    std::fs::create_dir_all(&scripts).unwrap();
    std::fs::write(
        scripts.join("Batch_FO4MergeCombinedObjectsAndCheck.pas"),
        "{ V1.5 }",
    )
    .unwrap();
    std::fs::write(
        scripts.join("Batch_FO4MergePrevisandCleanRefr.pas"),
        "{ V2.3 }",
    )
    .unwrap();
    fo4edit
}

fn required_archive_tool(tools: &ToolPaths, archive_tool: ArchiveTool) -> Result<PathBuf> {
    match archive_tool {
        ArchiveTool::Archive2 => tools.archive2.clone().ok_or_else(|| {
            Error::Other("Archive2.exe not found in Fallout 4 Tools\\archive2".into())
        }),
        ArchiveTool::BSArch => tools
            .bsarch
            .clone()
            .ok_or_else(|| Error::Other("BSArch.exe not found next to FO4Edit/xEdit".into())),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;

    fn probe_with_fo4(fallout4_dir: PathBuf) -> WorkflowToolchainProbe {
        WorkflowToolchainProbe::from_tool_paths(ToolPaths {
            fallout4_dir: Some(fallout4_dir),
            ..ToolPaths::default()
        })
        .unwrap()
    }

    #[test]
    fn probe_uses_fallout4_data_root_for_plugin_readiness() {
        let fo4 = PathBuf::from(r"C:\Fallout4");
        let probe = probe_with_fo4(fo4.clone());
        let readiness = probe.plugin_readiness(&PluginIdentity::parse("MyMod"), true);

        assert_eq!(readiness.data_dir(), fo4.join("Data"));
        assert_eq!(readiness.plugin_path(), fo4.join("Data").join("MyMod.esp"));
        assert!(readiness.non_interactive());
    }

    /// An undetermined Fallout 4 directory — the state a registry-less host (Wine) or a
    /// never-launched install leaves behind — must carry batch line 63's two concrete remedies
    /// rather than a raw registry error. `main` supplies the `ERROR - ` prefix.
    #[test]
    fn probe_reports_missing_fallout4_dir() {
        let err = WorkflowToolchainProbe::from_tool_paths(ToolPaths::default()).unwrap_err();
        let message = err.to_string();

        assert!(
            matches!(err, Error::Other(_)),
            "unexpected variant: {err:?}"
        );
        assert!(
            message.contains("Fallout 4 directory"),
            "missing the condition actually tested: {message}"
        );
        assert!(
            message.contains("Fallout4Launcher.exe"),
            "missing launcher remedy: {message}"
        );
        assert!(
            message.contains("--FO4"),
            "missing override remedy: {message}"
        );
    }

    #[test]
    fn prepare_requires_creation_kit_when_requested() {
        let dir = tempdir().unwrap();
        let fo4 = dir.path().join("Fallout4");
        fs::create_dir_all(&fo4).unwrap();
        fs::write(
            fo4.join("fallout4_test.ini"),
            "[CreationKit]\nBSHandleRefObjectPatch=true\n[CreationKit_Log]\nOutputFile=CK.log\n",
        )
        .unwrap();
        let probe = probe_with_fo4(fo4.clone());
        let mut requirements = ToolchainRequirements::none();
        requirements.require_creation_kit();

        let err = probe
            .prepare(dir.path(), ArchiveTool::Archive2, requirements)
            .unwrap_err();

        assert!(matches!(err, Error::Other(message) if message.contains("CreationKit.exe")));
    }

    #[test]
    fn prepare_loads_ckpe_and_log_path_for_creation_kit() {
        let dir = tempdir().unwrap();
        let fo4 = dir.path().join("Fallout4");
        fs::create_dir_all(&fo4).unwrap();
        let ck = fo4.join("CreationKit.exe");
        fs::write(&ck, b"").unwrap();
        fs::write(
            fo4.join("fallout4_test.ini"),
            "[CreationKit]\nBSHandleRefObjectPatch=true\n[CreationKit_Log]\nOutputFile=CK.log\n",
        )
        .unwrap();
        let probe = WorkflowToolchainProbe::from_tool_paths(ToolPaths {
            fallout4_dir: Some(fo4.clone()),
            creation_kit: Some(ck.clone()),
            ..ToolPaths::default()
        })
        .unwrap();
        let mut requirements = ToolchainRequirements::none();
        requirements.require_creation_kit();

        let toolchain = probe
            .prepare(dir.path(), ArchiveTool::Archive2, requirements)
            .unwrap();
        let session_log = dir.path().join("session.log");
        let paths = toolchain.creation_kit_paths(session_log.clone()).unwrap();

        assert_eq!(paths.exe, ck);
        assert_eq!(paths.fallout4_dir, fo4);
        assert_eq!(paths.ck_log_path, fo4.join("CK.log"));
        assert_eq!(paths.session_log, session_log);
        assert!(toolchain.diagnostics().iter().any(|diagnostic| matches!(
            diagnostic,
            ToolchainDiagnostic::CkpeConfig { log_file, .. } if log_file == "CK.log"
        )));
    }

    #[test]
    fn prepare_tolerates_non_utf8_ckpe_config_bytes() {
        let dir = tempdir().unwrap();
        let fo4 = dir.path().join("Fallout4");
        fs::create_dir_all(&fo4).unwrap();
        let ck = fo4.join("CreationKit.exe");
        fs::write(&ck, b"").unwrap();
        fs::write(
            fo4.join("fallout4_test.ini"),
            b"[CreationKit]\nBSHandleRefObjectPatch=true\n[CreationKit_Log]\nOutputFile=CK.log\n;\xFF\n",
        )
        .unwrap();
        let probe = WorkflowToolchainProbe::from_tool_paths(ToolPaths {
            fallout4_dir: Some(fo4),
            creation_kit: Some(ck),
            ..ToolPaths::default()
        })
        .unwrap();
        let mut requirements = ToolchainRequirements::none();
        requirements.require_creation_kit();

        probe
            .prepare(dir.path(), ArchiveTool::Archive2, requirements)
            .unwrap();
    }

    #[test]
    fn prepare_surfaces_ckpe_handle_limit_warning_as_diagnostic() {
        let dir = tempdir().unwrap();
        let fo4 = dir.path().join("Fallout4");
        fs::create_dir_all(&fo4).unwrap();
        fs::write(fo4.join("CreationKit.exe"), b"").unwrap();
        fs::write(
            fo4.join("fallout4_test.ini"),
            "[CreationKit]\nBSHandleRefObjectPatch=false\n[CreationKit_Log]\nOutputFile=CK.log\n",
        )
        .unwrap();
        let probe = WorkflowToolchainProbe::from_tool_paths(ToolPaths {
            fallout4_dir: Some(fo4),
            creation_kit: Some(dir.path().join("Fallout4").join("CreationKit.exe")),
            ..ToolPaths::default()
        })
        .unwrap();
        let mut requirements = ToolchainRequirements::none();
        requirements.require_creation_kit();

        let toolchain = probe
            .prepare(dir.path(), ArchiveTool::Archive2, requirements)
            .unwrap();

        assert!(toolchain.diagnostics().iter().any(|diagnostic| matches!(
            diagnostic,
            ToolchainDiagnostic::CkpeHandleLimitDisabled { setting_key, .. }
                if setting_key == "BSHandleRefObjectPatch"
        )));
    }

    #[test]
    fn the_fo4edit_requirement_asks_for_fo4edit_alone() {
        let requirements = ToolchainRequirements::fo4edit();

        assert!(requirements.needs_fo4edit());
        assert!(!requirements.needs_creation_kit());
        assert!(!requirements.needs_archive());
    }

    /// Discovered through `discover` with `-FO4:`, as `main` does: FO4Edit is then pointed at
    /// that install's `Data` (batch 498 sets `%ModDir_%` only in `:SetFO4`).
    #[test]
    fn a_fallout4_override_points_fo4edit_at_its_data_directory() {
        let dir = tempdir().unwrap();
        let fo4edit = write_fo4edit_install(dir.path());
        let fo4 = dir.path().join("Fallout4");
        fs::create_dir_all(&fo4).unwrap();
        let probe = WorkflowToolchainProbe::discover(dir.path(), Some(fo4.clone())).unwrap();

        let toolchain = probe
            .prepare(
                dir.path(),
                ArchiveTool::Archive2,
                ToolchainRequirements::fo4edit(),
            )
            .unwrap();
        let session_log = dir.path().join("session.log");
        let paths = toolchain.fo4edit_paths(session_log.clone()).unwrap();

        assert_eq!(paths.exe, fo4edit);
        assert_eq!(paths.data_dir_override, Some(fo4.join("Data")));
        assert_eq!(paths.session_log, session_log);
    }

    /// A Fallout 4 directory that did not come from `-FO4:` leaves FO4Edit to find `Data`.
    #[test]
    fn without_an_override_fo4edit_finds_its_own_data_directory() {
        let dir = tempdir().unwrap();
        let fo4edit = write_fo4edit_install(dir.path());
        let probe = WorkflowToolchainProbe::from_tool_paths(ToolPaths {
            fo4edit: Some(fo4edit),
            fallout4_dir: Some(dir.path().join("Fallout4")),
            ..ToolPaths::default()
        })
        .unwrap();

        let toolchain = probe
            .prepare(
                dir.path(),
                ArchiveTool::Archive2,
                ToolchainRequirements::fo4edit(),
            )
            .unwrap();
        let paths = toolchain
            .fo4edit_paths(dir.path().join("session.log"))
            .unwrap();

        assert_eq!(paths.data_dir_override, None);
    }

    /// FO4Edit discovered but never required is not prepared, so no episode can reach it.
    #[test]
    fn fo4edit_paths_need_fo4edit_readiness() {
        let dir = tempdir().unwrap();
        let fo4edit = write_fo4edit_install(dir.path());
        let probe = WorkflowToolchainProbe::from_tool_paths(ToolPaths {
            fo4edit: Some(fo4edit.clone()),
            fallout4_dir: Some(dir.path().join("Fallout4")),
            ..ToolPaths::default()
        })
        .unwrap();

        let toolchain = probe
            .prepare(
                dir.path(),
                ArchiveTool::Archive2,
                ToolchainRequirements::none(),
            )
            .unwrap();

        assert!(matches!(
            toolchain.fo4edit_paths(dir.path().join("session.log")),
            Err(Error::Fo4EditNotPrepared)
        ));
        // Discovery's answer is still reported.
        assert_eq!(toolchain.fo4edit_path(), Some(fo4edit.as_path()));
    }

    #[test]
    fn resolve_ck_log_path_handles_relative_and_absolute_settings() {
        let fo4 = PathBuf::from(r"C:\Fallout4");
        let absolute = std::env::current_dir().unwrap().join("CK.log");

        assert_eq!(resolve_ck_log_path(&fo4, "CK.log"), fo4.join("CK.log"));
        assert_eq!(
            resolve_ck_log_path(&fo4, &absolute.to_string_lossy()),
            absolute
        );
    }
}
