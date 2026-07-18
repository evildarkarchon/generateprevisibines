use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("plugin name {name} has illegal characters")]
    InvalidPluginName { name: String },

    #[error("plugin name {name} is reserved, please choose another")]
    ReservedPluginName { name: String },

    #[error("plugin name cannot contain spaces in clean build mode")]
    PluginNameContainsSpaces,

    #[error("CKPE configuration error: {0}")]
    CkpeConfig(String),

    #[error("required xEdit script missing: {0}")]
    MissingXeditScript(String),

    #[error("xEdit script version too old: {script} (requires {required})")]
    XeditScriptVersion { script: String, required: String },

    #[error("{0}")]
    Io(#[from] std::io::Error),

    #[error("prompt error: {0}")]
    Prompt(#[from] dialoguer::Error),

    #[error("registry error: {0}")]
    Registry(String),

    #[error("ERROR - This Plugin already has an Archive")]
    PluginAlreadyHasArchive,

    #[error("xPrevisPatch.esp not found in Data folder")]
    SeedPluginMissing,

    #[error(
        "ERROR - Copy of xPrevisPatch.esp failed. If using MO2, run this tool from within MO2."
    )]
    SeedCopyFailed,

    #[error(
        "precombined meshes already exist — delete meshes\\precombined or resume from a later step"
    )]
    PrecombinedMeshesExist,

    #[error("vis folder contains .uvd files — remove them before generating precombines")]
    VisUvdFilesExist,

    #[error("DEFAULT: OUT OF HANDLE ARRAY ENTRIES found in Creation Kit log")]
    HandleArrayLogError,

    #[error("CombinedObjects.esp was not created by Creation Kit")]
    MissingCombinedObjects,

    #[error("{0} - Geometry.psg was not created (clean build mode)")]
    MissingGeometryPsg(String),

    #[error("no precombined .nif meshes were generated under meshes\\precombined")]
    NoPrecombinedMeshes,

    #[error("workflow step {0} is not implemented yet")]
    StepNotImplemented(u8),

    #[error(
        "workflow operation capability mismatch: run planned for steps {planned:?}, executor supports {executor:?}"
    )]
    OperationCapabilityMismatch { planned: Vec<u8>, executor: Vec<u8> },

    #[error("{0}")]
    Other(String),
}
