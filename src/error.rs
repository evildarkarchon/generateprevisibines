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

    // The batch's own wording (`GeneratePrevisibines.bat:271`), which never shows the user the
    // findstr needle it matched on. `main` prints the `ERROR - ` prefix, so this renders as the
    // batch's `ERROR - GeneratePrecombined ran out of Reference Handles`.
    #[error("GeneratePrecombined ran out of Reference Handles")]
    HandleArrayLogError,

    #[error("CombinedObjects.esp was not created by Creation Kit")]
    MissingCombinedObjects,

    #[error("{0} - Geometry.psg was not created (clean build mode)")]
    MissingGeometryPsg(String),

    #[error("no precombined .nif meshes were generated under meshes\\precombined")]
    NoPrecombinedMeshes,

    #[error("workflow step {0} is not implemented yet")]
    StepNotImplemented(u8),

    // A preparation bug rather than a user state: a Workflow Run only resolves Creation Kit
    // paths when a runnable Workflow Operation asked for Creation Kit readiness, so reaching an
    // episode without them means planning and readiness disagreed. A variant rather than two
    // copies of one string, because both the toolchain and the run have to report it.
    #[error("Creation Kit was not prepared for this Workflow Run")]
    CreationKitNotPrepared,

    #[error("{0}")]
    Other(String),
}
