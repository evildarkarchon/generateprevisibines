use std::path::PathBuf;

/// Build mode matching batch `-clean`, `-filtered`, `-xbox` (default: clean).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BuildMode {
    #[default]
    Clean,
    Filtered,
    Xbox,
}

impl BuildMode {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Clean => "clean",
            Self::Filtered => "filtered",
            Self::Xbox => "xbox",
        }
    }

    /// Steps 4 and 5 (PSG compress, CDX build) run only in clean mode.
    #[must_use]
    pub const fn includes_psg_and_cdx(self) -> bool {
        matches!(self, Self::Clean)
    }
}

impl std::str::FromStr for BuildMode {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "clean" => Ok(Self::Clean),
            "filtered" => Ok(Self::Filtered),
            "xbox" => Ok(Self::Xbox),
            _ => Err(format!("unknown build mode: {s}")),
        }
    }
}

/// Archive backend: Bethesda `Archive2.exe` (default) or community `BSArch.exe`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ArchiveTool {
    #[default]
    Archive2,
    BSArch,
}

impl ArchiveTool {
    #[must_use]
    pub const fn program_name(self) -> &'static str {
        match self {
            Self::Archive2 => "Archive2",
            Self::BSArch => "BSArch",
        }
    }
}

/// Eight-step workflow from batch V2.96 (resume menu steps 1–8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub enum WorkflowStep {
    GeneratePrecombines = 1,
    MergePrecombineObjects = 2,
    CreateBa2FromPrecombines = 3,
    CompressPsg = 4,
    BuildCdx = 5,
    GeneratePrevis = 6,
    MergePrevis = 7,
    AddPrevisToArchive = 8,
}

impl WorkflowStep {
    #[must_use]
    pub const fn number(self) -> u8 {
        self as u8
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::GeneratePrecombines => "Generate Precombines Via CK",
            Self::MergePrecombineObjects => "Merge PrecombineObjects.esp Via xEdit",
            Self::CreateBa2FromPrecombines => "Create BA2 Archive from Precombines",
            Self::CompressPsg => "Compress PSG Via CK",
            Self::BuildCdx => "Build CDX Via CK",
            Self::GeneratePrevis => "Generate Previs Via CK",
            Self::MergePrevis => "Merge Previs.esp Via xEdit",
            Self::AddPrevisToArchive => "Add Previs files to BA2 Archive",
        }
    }

    /// Steps included for a given build mode (batch skips 4–5 unless clean).
    #[must_use]
    pub fn steps_for_mode(mode: BuildMode) -> &'static [WorkflowStep] {
        use WorkflowStep::{
            AddPrevisToArchive, BuildCdx, CompressPsg, CreateBa2FromPrecombines,
            GeneratePrecombines, GeneratePrevis, MergePrecombineObjects, MergePrevis,
        };

        if mode.includes_psg_and_cdx() {
            &[
                GeneratePrecombines,
                MergePrecombineObjects,
                CreateBa2FromPrecombines,
                CompressPsg,
                BuildCdx,
                GeneratePrevis,
                MergePrevis,
                AddPrevisToArchive,
            ]
        } else {
            &[
                GeneratePrecombines,
                MergePrecombineObjects,
                CreateBa2FromPrecombines,
                GeneratePrevis,
                MergePrevis,
                AddPrevisToArchive,
            ]
        }
    }

    #[must_use]
    pub fn from_number(n: u8) -> Option<Self> {
        match n {
            1 => Some(Self::GeneratePrecombines),
            2 => Some(Self::MergePrecombineObjects),
            3 => Some(Self::CreateBa2FromPrecombines),
            4 => Some(Self::CompressPsg),
            5 => Some(Self::BuildCdx),
            6 => Some(Self::GeneratePrevis),
            7 => Some(Self::MergePrevis),
            8 => Some(Self::AddPrevisToArchive),
            _ => None,
        }
    }
}

/// Parsed plugin identity (base name + file name with extension).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginIdentity {
    pub base_name: String,
    pub file_name: String,
}

impl PluginIdentity {
    /// Parse user input the same way the batch script does (default `.esp` extension).
    #[must_use]
    pub fn parse(input: &str) -> Self {
        let trimmed = input.trim();
        let path = std::path::Path::new(trimmed);
        let has_esp_ext = path.extension().is_some_and(|ext| {
            ext.eq_ignore_ascii_case("esp")
                || ext.eq_ignore_ascii_case("esm")
                || ext.eq_ignore_ascii_case("esl")
        });

        if has_esp_ext {
            let base = trimmed
                .rsplit_once('.')
                .map_or(trimmed, |(base, _)| base)
                .to_string();
            Self {
                base_name: base,
                file_name: trimmed.to_string(),
            }
        } else {
            Self {
                base_name: trimmed.to_string(),
                file_name: format!("{trimmed}.esp"),
            }
        }
    }

    #[must_use]
    pub fn archive_name(&self) -> String {
        format!("{} - Main.ba2", self.base_name)
    }
}

/// CKPE configuration variant detected from Fallout 4 install layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CkpeConfigKind {
    /// `CreationKitPlatformExtended.toml` (modern CKPE.Fallout4.dll)
    Toml,
    /// `CreationKitPlatformExtended.ini`
    Ini,
    /// Legacy `fallout4_test.ini` with different setting keys
    LegacyTestIni,
}

impl CkpeConfigKind {
    #[must_use]
    pub const fn file_name(self) -> &'static str {
        match self {
            Self::Toml => "CreationKitPlatformExtended.toml",
            Self::Ini => "CreationKitPlatformExtended.ini",
            Self::LegacyTestIni => "fallout4_test.ini",
        }
    }

    #[must_use]
    pub const fn handle_setting_key(self) -> &'static str {
        match self {
            Self::Toml | Self::Ini => "bBSPointerHandleExtremly",
            Self::LegacyTestIni => "BSHandleRefObjectPatch",
        }
    }

    #[must_use]
    pub const fn log_setting_key(self) -> &'static str {
        match self {
            Self::Toml | Self::Ini => "sOutputFile",
            Self::LegacyTestIni => "OutputFile",
        }
    }
}

/// Resolved paths and options for a workflow run.
#[derive(Debug, Clone)]
pub struct ProjectConfig {
    pub build_mode: BuildMode,
    pub archive_tool: ArchiveTool,
    pub fallout4_dir: PathBuf,
    pub plugin: PluginIdentity,
    pub non_interactive: bool,
    pub resume_from: Option<WorkflowStep>,
    pub fo4edit_path: Option<PathBuf>,
    pub xedit_data_dir: Option<PathBuf>,
    /// Resolved CK log path (absolute) from CKPE validation.
    pub ck_log_path: Option<PathBuf>,
}

impl ProjectConfig {
    #[must_use]
    pub fn data_dir(&self) -> PathBuf {
        self.fo4edit_data_dir()
    }

    #[must_use]
    pub fn fo4edit_data_dir(&self) -> PathBuf {
        self.xedit_data_dir
            .clone()
            .unwrap_or_else(|| self.fallout4_data())
    }

    #[must_use]
    pub fn fallout4_data(&self) -> PathBuf {
        self.fallout4_dir.join("Data")
    }

    #[must_use]
    pub fn plugin_path(&self) -> PathBuf {
        self.fo4edit_data_dir().join(&self.plugin.file_name)
    }

    #[must_use]
    pub fn plugin_archive_path(&self) -> PathBuf {
        self.fo4edit_data_dir().join(self.plugin.archive_name())
    }

    #[must_use]
    pub fn precombined_dir(&self) -> PathBuf {
        self.fo4edit_data_dir().join("meshes").join("precombined")
    }

    #[must_use]
    pub fn vis_dir(&self) -> PathBuf {
        self.fo4edit_data_dir().join("vis")
    }
}
