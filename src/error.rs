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

    // A divergence: the batch accepts any length. The PJM merge scripts cut `-mod:` at 60
    // characters, so a longer name sends FO4Edit looking for the wrong plugin, and the run fails
    // at Step 2 with the misleading "missing files, Probably due to MO2" fatal after Step 1's
    // hour in Creation Kit. `name` is the file name, extension included, as it was counted.
    #[error(
        "plugin file name {name} is longer than 60 characters, which the FO4Edit merge scripts cannot read"
    )]
    PluginNameTooLong { name: String },

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

    // The batch's own wording, printed both where Step 1 (259) and Step 6 (312) find `.uvd`
    // files left in `Data\vis`, so it reads correctly from either step.
    #[error("Previs directory (Data\\vis) not empty")]
    VisUvdFilesExist,

    // The batch's own wording (`GeneratePrevisibines.bat:277`), which never shows the user the
    // findstr needle it matched on. The stopped Workflow Run prints the `ERROR - ` prefix, so this
    // renders as the batch's `ERROR - GeneratePrecombined ran out of Reference Handles`.
    #[error("GeneratePrecombined ran out of Reference Handles")]
    HandleArrayLogError,

    #[error("CombinedObjects.esp was not created by Creation Kit")]
    MissingCombinedObjects,

    #[error("{0} - Geometry.psg was not created (clean build mode)")]
    MissingGeometryPsg(String),

    // The batch's single `:RunCK` output check (`GeneratePrevisibines.bat:471`), shared by every
    // Creation Kit step whose output is checked there: one variant because the batch has one
    // line. `operation` is the batch verb and `file` the bare name relative to `Data`, both as
    // `%1` and `%~2` print them. Step 1 keeps its own `MissingCombinedObjects`.
    #[error(
        "{operation} failed to create file {file} with exit status {}",
        exit_code_text(*.code)
    )]
    MissingCreationKitOutput {
        operation: &'static str,
        file: String,
        code: Option<i32>,
    },

    #[error("no precombined .nif meshes were generated under meshes\\precombined")]
    NoPrecombinedMeshes,

    // The batch's own wording (`GeneratePrevisibines.bat:281`), from Step 2's entry check. Kept
    // apart from `NoPrecombinedMeshes`, which is Step 1's postcondition ("were generated"): a
    // resume at 2 has generated nothing, it has only found nothing to merge.
    #[error("No Precombined meshes found")]
    NoPrecombinedMeshesFound,

    #[error("workflow step {0} is not implemented yet")]
    StepNotImplemented(u8),

    // A preparation bug rather than a user state: a Workflow Run only resolves Creation Kit
    // paths when a runnable Workflow Operation asked for Creation Kit readiness, so reaching an
    // episode without them means planning and readiness disagreed. A variant rather than two
    // copies of one string, because both the toolchain and the run have to report it.
    #[error("Creation Kit was not prepared for this Workflow Run")]
    CreationKitNotPrepared,

    // The FO4Edit twin of `CreationKitNotPrepared`, and a preparation bug for the same reason: a
    // Workflow Run resolves FO4Edit paths only when a runnable Workflow Operation asked for
    // FO4Edit readiness.
    #[error("FO4Edit was not prepared for this Workflow Run")]
    Fo4EditNotPrepared,

    // A divergence: the batch polls for the log forever (548–550), so a FO4Edit that crashed or
    // was closed before its script finished left the run hanging. `script` is the bare `.pas`
    // name, as the batch's `%2` prints it.
    #[error(
        "FO4Edit exited before script {script} wrote its log (exit code {})",
        exit_code_text(*.code)
    )]
    Fo4EditExitedEarly {
        script: &'static str,
        code: Option<i32>,
    },

    // FO4Edit is never killed, because xEdit saves the merged plugin on its own close path. A
    // FO4Edit that ignored both close requests is left for the operator to close.
    #[error(
        "FO4Edit (PID {pid}) is still running after two close requests. Close it, then rerun this step."
    )]
    Fo4EditStillRunning { pid: u32 },

    // The log was there when the poll ended and gone when the episode came to read it, after
    // about 35 seconds of close delays. Something other than FO4Edit removed it, so there is
    // nothing left to judge the script run by.
    #[error("FO4Edit's log {} was gone when the run came to read it", .path.display())]
    UnattendedLogMissing { path: std::path::PathBuf },

    // The batch's own wording (`GeneratePrevisibines.bat:561`).
    #[error(
        "FO4Edit script {script} failed [missing files, Probably due to MO2]. Rerun this phase to fix"
    )]
    Fo4EditScriptMissingModules { script: &'static str },

    // The batch's own wording (`GeneratePrevisibines.bat:563`).
    #[error("FO4Edit script {script} failed")]
    Fo4EditScriptFailed { script: &'static str },

    #[error("{0}")]
    Other(String),
}

/// Render an external tool's exit code as the batch's `%Err_%` would print it.
///
/// The batch always has an `%ERRORLEVEL%` to print. A missing code is only reachable off
/// Windows (a signal-terminated process), and "unknown" says so rather than inventing a number.
pub(crate) fn exit_code_text(code: Option<i32>) -> String {
    code.map_or_else(|| "unknown".to_string(), |code| code.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Batch line 471, minus the `ERROR - ` prefix that is added where it is printed.
    #[test]
    fn a_missing_creation_kit_output_reads_as_the_batch_wording() {
        let error = Error::MissingCreationKitOutput {
            operation: "CompressPSG",
            file: "MyMod - Geometry.csg".to_string(),
            code: Some(0),
        };

        assert_eq!(
            error.to_string(),
            "CompressPSG failed to create file MyMod - Geometry.csg with exit status 0"
        );
    }

    /// Batch lines 259 and 312, minus the `ERROR - ` prefix.
    #[test]
    fn a_non_empty_vis_directory_reads_as_the_batch_wording() {
        assert_eq!(
            Error::VisUvdFilesExist.to_string(),
            "Previs directory (Data\\vis) not empty"
        );
    }

    /// Batch lines 561 and 563, minus the `ERROR - ` prefix.
    #[test]
    fn the_shared_fo4edit_fatals_read_as_the_batch_wording() {
        let script = "Batch_FO4MergePrevisandCleanRefr.pas";

        assert_eq!(
            Error::Fo4EditScriptMissingModules { script }.to_string(),
            "FO4Edit script Batch_FO4MergePrevisandCleanRefr.pas failed [missing files, \
             Probably due to MO2]. Rerun this phase to fix"
        );
        assert_eq!(
            Error::Fo4EditScriptFailed { script }.to_string(),
            "FO4Edit script Batch_FO4MergePrevisandCleanRefr.pas failed"
        );
    }

    /// Batch line 281, minus the `ERROR - ` prefix.
    #[test]
    fn no_precombined_meshes_at_step_two_reads_as_the_batch_wording() {
        assert_eq!(
            Error::NoPrecombinedMeshesFound.to_string(),
            "No Precombined meshes found"
        );
    }

    #[test]
    fn an_early_fo4edit_exit_names_the_script_and_renders_an_unknown_code() {
        let script = "Batch_FO4MergeCombinedObjectsAndCheck.pas";

        assert_eq!(
            Error::Fo4EditExitedEarly {
                script,
                code: Some(3)
            }
            .to_string(),
            "FO4Edit exited before script Batch_FO4MergeCombinedObjectsAndCheck.pas wrote its \
             log (exit code 3)"
        );
        assert_eq!(
            Error::Fo4EditExitedEarly { script, code: None }.to_string(),
            "FO4Edit exited before script Batch_FO4MergeCombinedObjectsAndCheck.pas wrote its \
             log (exit code unknown)"
        );
    }

    #[test]
    fn a_fo4edit_still_running_names_its_pid() {
        assert_eq!(
            Error::Fo4EditStillRunning { pid: 4242 }.to_string(),
            "FO4Edit (PID 4242) is still running after two close requests. Close it, then \
             rerun this step."
        );
    }

    #[test]
    fn a_missing_creation_kit_output_with_no_exit_code_renders_it_as_unknown() {
        let error = Error::MissingCreationKitOutput {
            operation: "BuildCDX",
            file: "MyMod.cdx".to_string(),
            code: None,
        };

        assert_eq!(
            error.to_string(),
            "BuildCDX failed to create file MyMod.cdx with exit status unknown"
        );
    }
}
