use anyhow::Result;
use std::path::PathBuf;

/// Build mode for the precombine/previs generation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildMode {
    Clean,
    Filtered,
    Xbox,
}

impl BuildMode {
    pub fn as_str(&self) -> &str {
        match self {
            BuildMode::Clean => "clean",
            BuildMode::Filtered => "filtered",
            BuildMode::Xbox => "xbox",
        }
    }
}

/// Archive tool to use
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveTool {
    Archive2,
    BSArch,
}

/// Configuration for the tool, including paths to external programs
#[derive(Debug)]
pub struct Config {
    /// Build mode (clean/filtered/xbox)
    pub build_mode: BuildMode,

    /// Archive tool to use
    pub archive_tool: ArchiveTool,

    /// Plugin name (e.g., "MyMod.esp")
    pub plugin_name: Option<String>,

    /// Fallout 4 installation directory
    pub fo4_dir: PathBuf,

    /// `FO4Edit` executable path
    pub fo4edit_path: PathBuf,

    /// Creation Kit executable path
    pub creation_kit_path: PathBuf,

    /// Archive2 or `BSArch` executable path
    pub archive_exe_path: PathBuf,

    /// CKPE configuration file path
    pub ckpe_config_path: Option<PathBuf>,

    /// Creation Kit log file path (from CKPE config)
    pub ck_log_path: Option<PathBuf>,

    /// Use Mod Organizer 2 mode (run tools through MO2's VFS)
    pub mo2_mode: bool,

    /// Path to ModOrganizer.exe (only used if `mo2_mode` is true)
    pub mo2_path: Option<PathBuf>,

    /// Path to MO2's VFS staging directory (e.g., overwrite folder)
    /// Required when `mo2_mode` is true for archiving operations
    pub mo2_data_dir: Option<PathBuf>,
}

impl Config {
    /// Create a new configuration with the given build mode and archive tool
    pub fn new(build_mode: BuildMode, archive_tool: ArchiveTool) -> Self {
        Self {
            build_mode,
            archive_tool,
            plugin_name: None,
            fo4_dir: PathBuf::new(),
            fo4edit_path: PathBuf::new(),
            creation_kit_path: PathBuf::new(),
            archive_exe_path: PathBuf::new(),
            ckpe_config_path: None,
            ck_log_path: None,
            mo2_mode: false,
            mo2_path: None,
            mo2_data_dir: None,
        }
    }

    /// Set the plugin name
    #[allow(dead_code)]
    pub fn with_plugin_name(mut self, name: String) -> Self {
        self.plugin_name = Some(name);
        self
    }

    /// Get the Data directory for Fallout 4
    pub fn data_dir(&self) -> PathBuf {
        self.fo4_dir.join("Data")
    }

    /// Get the meshes\\precombined directory
    #[allow(dead_code)]
    pub fn precombined_dir(&self) -> PathBuf {
        self.data_dir().join("meshes").join("precombined")
    }

    /// Get the vis directory
    #[allow(dead_code)]
    pub fn vis_dir(&self) -> PathBuf {
        self.data_dir().join("vis")
    }

    /// Validate that all required paths exist
    pub fn validate(&self) -> Result<()> {
        if !self.fo4_dir.exists() {
            anyhow::bail!(
                "Fallout 4 directory does not exist: {}",
                self.fo4_dir.display()
            );
        }

        if !self.fo4edit_path.exists() {
            anyhow::bail!("FO4Edit not found at: {}", self.fo4edit_path.display());
        }

        if !self.creation_kit_path.exists() {
            anyhow::bail!(
                "Creation Kit not found at: {}",
                self.creation_kit_path.display()
            );
        }

        if !self.archive_exe_path.exists() {
            let tool_name = match self.archive_tool {
                ArchiveTool::Archive2 => "Archive2",
                ArchiveTool::BSArch => "BSArch",
            };
            anyhow::bail!(
                "{} not found at: {}",
                tool_name,
                self.archive_exe_path.display()
            );
        }

        // Validate MO2 configuration if MO2 mode is enabled
        if self.mo2_mode {
            if let Some(ref mo2_path) = self.mo2_path {
                if !mo2_path.exists() {
                    anyhow::bail!("Mod Organizer 2 not found at: {}", mo2_path.display());
                }
            } else {
                anyhow::bail!("MO2 mode is enabled but mo2_path is not set");
            }

            // Validate mo2_data_dir if provided
            if let Some(ref mo2_data_dir) = self.mo2_data_dir
                && !mo2_data_dir.exists() {
                    anyhow::bail!(
                        "MO2 data directory not found at: {}",
                        mo2_data_dir.display()
                    );
                }
        }

        Ok(())
    }
}
