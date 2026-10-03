//! Wrappers for external tools.
//!
//! See `docs/workarounds.md` — do not remove the MO2 delays, the DLL renaming around Creation
//! Kit, or FO4Edit's Module Selection dismissal and close sequence. The Creation Kit and FO4Edit
//! episodes are here; the archive adapters are not yet, and the invocation detail they must
//! reproduce lives with slices 4 and 8 in `docs/future-features.md`.

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
