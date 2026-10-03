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

    // The batch's own wording (`GeneratePrevisibines.bat:323`), from Step 7's entry check: no
    // `.uvd` under `Data\vis` means Step 6 left nothing to merge. A stop rather than a warning,
    // because absent outputs stop; only a partial shortfall is Step 7's warning to raise.
    #[error("No Visibility files Generated")]
    NoVisibilityFiles,

    // The batch's own wording (`GeneratePrevisibines.bat:324`), Step 7's second entry check. Not
    // `MissingCreationKitOutput`: Step 7 has not run Creation Kit, it has only found nothing to
    // merge, as a resume at 7 over an empty `Data` would.
    #[error("No Previs.esp Generated")]
    NoPrevisPlugin,

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

    // The archive-tool twin of `CreationKitNotPrepared`, and a preparation bug for the same
    // reason: a Workflow Run resolves the archive tool only when a runnable Workflow Operation
    // asked for archive readiness.
    #[error("The archive tool was not prepared for this Workflow Run")]
    ArchiveNotPrepared,

    // The batch's own wording (`GeneratePrevisibines.bat:406`), without the literal quotes its
    // `echo "ERROR - …"` printed. Stops every run, interactive or not: the batch's `Call` let a
    // non-interactive run fall through it.
    #[error("Archive2 failed with error {}", exit_code_text(*.code))]
    Archive2Failed { code: Option<i32> },

    // The batch's own wording (`GeneratePrevisibines.bat:418`), quotes dropped as above.
    #[error("Archive2 Extract failed with error {}", exit_code_text(*.code))]
    Archive2ExtractFailed { code: Option<i32> },

    // The batch's own wording (`GeneratePrevisibines.bat:399, 432`).
    #[error("BSArch failed with error {}", exit_code_text(*.code))]
    BsarchFailed { code: Option<i32> },

    // New: the batch never unpacks. Worded as the Archive2 extract failure is.
    #[error("BSArch Unpack failed with error {}", exit_code_text(*.code))]
    BsarchUnpackFailed { code: Option<i32> },

    // The batch's own wording (`GeneratePrevisibines.bat:408`). Load-bearing: `Archive2 -c` with
    // no sources exits 0 and creates nothing.
    #[error("No plugin archive Created")]
    NoPluginArchiveCreated,

    // The new archive was built and checked, but the old one could not be replaced by it. The
    // work folder is kept: it may hold the only copy of the new archive, and for BSArch of the
    // staged precombines or `vis`, which the next run's restore moves back.
    #[error(
        "The new plugin archive is at {} but could not be moved to {}. The next run moves back \
         what {} holds the only copy of, or sets {} aside if it cannot.",
        .built.display(),
        .target.display(),
        .work.display(),
        .work.display()
    )]
    ArchiveSwapFailed {
        built: std::path::PathBuf,
        target: std::path::PathBuf,
        work: std::path::PathBuf,
    },

    // A BSArch pack failed and the staged folder could not be moved back into `Data`. Replaces
    // the pack's own error, because where the files are now is what the operator needs.
    #[error(
        "Could not move {} back to {}. The next run moves it back, or sets {} aside if it cannot.",
        .from.display(),
        .to.display(),
        .work.display()
    )]
    ArchiveMoveBackFailed {
        from: std::path::PathBuf,
        to: std::path::PathBuf,
        work: std::path::PathBuf,
    },

    // A stop even though the swap succeeded: a restore list left behind would make the next run
    // move files the new archive already holds back into `Data`.
    #[error(
        "The new plugin archive is in place, but {} could not be removed. Delete it before \
         rerunning, or the next run will move files the archive already holds back into Data.",
        .path.display()
    )]
    ArchiveRestoreListNotRemoved { path: std::path::PathBuf },

    // A divergence decided on #29: with no loose meshes the batch silently skips to Step 4 (289).
    // The port completes only when the archive already holds them (an earlier attempt packed
    // it), and otherwise stops, because Step 8 would have no precombines to add previs to.
    #[error("No Precombined meshes found to archive, and {name} does not exist")]
    NoPrecombinesToArchive { name: String },

    // A divergence: the batch rebuilds from `vis` alone when the extract yields no precombines
    // (441 → 445), which silently drops them. The old archive is untouched when this is raised.
    #[error("{name} holds no precombined meshes, so previs cannot be added to it")]
    PluginArchiveHasNoPrecombines { name: String },

    // Raised before intake when another process holds `<fo4>\GeneratePrevisibines.lock`
    // (ADR-0005): two runs against one installation would each read the other's live work as
    // crash leftovers. A crashed run's lock is released by Windows, so a retry soon succeeds.
    #[error(
        "Another GeneratePrevisibines run is using {} (it holds {}). Wait for it to finish, then rerun.",
        .fallout4_dir.display(),
        .lock_path.display()
    )]
    InstallationInUse {
        fallout4_dir: std::path::PathBuf,
        lock_path: std::path::PathBuf,
    },

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

    /// Batch lines 323 and 324, minus the `ERROR - ` prefix.
    #[test]
    fn step_seven_entry_stops_read_as_the_batch_wording() {
        assert_eq!(
            Error::NoVisibilityFiles.to_string(),
            "No Visibility files Generated"
        );
        assert_eq!(Error::NoPrevisPlugin.to_string(), "No Previs.esp Generated");
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

    /// Batch lines 399, 406, 418 and 432, without the quotes and the `ERROR - ` prefix, plus
    /// the new BSArch unpack failure worded alongside them.
    #[test]
    fn the_archive_tool_failures_read_as_the_batch_wording() {
        let code = Some(-1);

        assert_eq!(
            Error::Archive2Failed { code }.to_string(),
            "Archive2 failed with error -1"
        );
        assert_eq!(
            Error::Archive2ExtractFailed { code }.to_string(),
            "Archive2 Extract failed with error -1"
        );
        assert_eq!(
            Error::BsarchFailed { code }.to_string(),
            "BSArch failed with error -1"
        );
        assert_eq!(
            Error::BsarchUnpackFailed { code }.to_string(),
            "BSArch Unpack failed with error -1"
        );
        assert_eq!(
            Error::NoPluginArchiveCreated.to_string(),
            "No plugin archive Created"
        );
    }

    /// A code-less exit status cannot be built on Windows, so the rendering is pinned here.
    #[test]
    fn an_archive_tool_failure_with_no_exit_code_renders_it_as_unknown() {
        assert_eq!(
            Error::Archive2Failed { code: None }.to_string(),
            "Archive2 failed with error unknown"
        );
        assert_eq!(
            Error::BsarchUnpackFailed { code: None }.to_string(),
            "BSArch Unpack failed with error unknown"
        );
    }

    /// The swap and move-back failures say the next run restores the files, not that it
    /// clears them.
    #[test]
    fn the_archive_recovery_stops_name_every_path_and_the_next_run_restore() {
        let work = std::path::PathBuf::from(r"C:\Fallout4\ArchiveWork");

        assert_eq!(
            Error::ArchiveSwapFailed {
                built: work.join("MyMod - Main.ba2"),
                target: std::path::PathBuf::from(r"C:\Fallout4\Data\MyMod - Main.ba2"),
                work: work.clone(),
            }
            .to_string(),
            format!(
                "The new plugin archive is at {} but could not be moved to \
                 C:\\Fallout4\\Data\\MyMod - Main.ba2. The next run moves back what {} holds \
                 the only copy of, or sets {} aside if it cannot.",
                work.join("MyMod - Main.ba2").display(),
                work.display(),
                work.display()
            )
        );
        assert_eq!(
            Error::ArchiveMoveBackFailed {
                from: work.join("staging").join("vis"),
                to: std::path::PathBuf::from(r"C:\Fallout4\Data\vis"),
                work: work.clone(),
            }
            .to_string(),
            format!(
                "Could not move {} back to C:\\Fallout4\\Data\\vis. The next run moves it back, \
                 or sets {} aside if it cannot.",
                work.join("staging").join("vis").display(),
                work.display()
            )
        );
        assert_eq!(
            Error::ArchiveRestoreListNotRemoved {
                path: work.join("restore.txt"),
            }
            .to_string(),
            format!(
                "The new plugin archive is in place, but {} could not be removed. Delete it \
                 before rerunning, or the next run will move files the archive already holds \
                 back into Data.",
                work.join("restore.txt").display()
            )
        );
    }

    /// Step 3 with nothing to pack and no archive from an earlier attempt names the archive it
    /// looked for.
    #[test]
    fn nothing_to_archive_at_step_three_names_the_missing_archive() {
        assert_eq!(
            Error::NoPrecombinesToArchive {
                name: "MyMod - Main.ba2".to_string()
            }
            .to_string(),
            "No Precombined meshes found to archive, and MyMod - Main.ba2 does not exist"
        );
    }

    #[test]
    fn an_archive_without_precombines_is_named() {
        assert_eq!(
            Error::PluginArchiveHasNoPrecombines {
                name: "MyMod - Main.ba2".to_string()
            }
            .to_string(),
            "MyMod - Main.ba2 holds no precombined meshes, so previs cannot be added to it"
        );
        assert_eq!(
            Error::ArchiveNotPrepared.to_string(),
            "The archive tool was not prepared for this Workflow Run"
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
