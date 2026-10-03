//! Wrappers for external tools.
//!
//! See `docs/workarounds.md` — do not remove the MO2 delays, the DLL renaming around Creation
//! Kit, FO4Edit's Module Selection dismissal and close sequence, or the Archive2 extract-repack.
//! The Creation Kit, FO4Edit and Archive episodes are all here; `docs/episodes.md` holds the
//! invocation detail each one reproduces.

mod archive;
mod creation_kit;
// `DllGuard` is deliberately not re-exported below. `disable` takes the crate-private
// `FileSpace`, so the guard cannot be constructed from outside the crate, and its only
// caller — `creation_kit` — reaches it through this module directly.
mod dll;
mod fo4edit;

// Internal seams: crate-visible so the tests in `src/workflow/` can reach the recording
// adapters, but never part of what a Workflow Operation is handed.
pub(crate) mod clock;
pub(crate) mod desktop;
pub(crate) mod process;
pub(crate) mod wait;

// Crate-visible, not public: `CreationKitOps` borrows the crate-private `ProcessRunner`,
// `Wait` and `FileSpace` ports, so it cannot be constructed from outside the crate anyway.
// `CkOperation` is not re-exported at all — Creation Kit's command grammar stays inside
// `creation_kit`, behind the four domain methods. `CkRun` is not re-exported either: the
// Generate Precombines Operation reads its log content without ever naming the type.
pub(crate) use creation_kit::{CkPorts, CreationKitOps, CreationKitPaths};
// The same for FO4Edit: `XeditScript` and the source plugins stay inside `fo4edit`, behind
// the two merge methods. The script file names are re-exported only because plugin validation
// checks that the scripts are installed, and they should have one spelling.
pub(crate) use fo4edit::{
    Fo4EditOps, Fo4EditPaths, Fo4EditPorts, MERGE_COMBINED_OBJECTS_SCRIPT, MERGE_PREVIS_SCRIPT,
};
// The same for the archive tools: their command lines, the work folder and the restore list
// stay inside `archive`, behind the two domain verbs. The run-start restore is re-exported
// because the Workflow Run calls it before any step, without an episode.
pub(crate) use archive::{ArchiveOps, ArchivePaths, ArchivePorts, restore_archive_work_folders};

// Test-only seeding of leftover work folders, in the format that stays inside `archive`.
#[cfg(test)]
pub(crate) use archive::leftovers;
