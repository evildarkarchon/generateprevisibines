//! Wrappers for external tools.
//!
//! See `docs/workarounds.md` — do not remove MO2 delays, DLL renaming, `FO4Edit`
//! keystroke automation, or Archive2 extract-repack behavior when implementing these.

mod archive;
mod creation_kit;
// `DllGuard` is deliberately not re-exported below. `disable` takes the crate-private
// `FileSpace`, so the guard cannot be constructed from outside the crate, and its only
// caller — `creation_kit` — reaches it through this module directly.
mod dll;
mod fo4edit;

// Internal seams: crate-visible so the tests in `src/workflow/` can reach the recording
// adapters, but never part of what a Workflow Operation is handed.
pub(crate) mod process;
pub(crate) mod wait;

pub use archive::ArchiveOps;
pub use fo4edit::Fo4EditOps;

// Crate-visible, not public: `CreationKitOps` borrows the crate-private `ProcessRunner`,
// `Wait` and `FileSpace` ports, so it cannot be constructed from outside the crate anyway.
// `CkOperation` is not re-exported at all — Creation Kit's command grammar stays inside
// `creation_kit`, behind the four domain methods. `CkRun` is not re-exported either: the
// Generate Precombines Operation reads its log content without ever naming the type.
pub(crate) use creation_kit::{CreationKitOps, CreationKitPaths};
