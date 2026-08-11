//! Wrappers for external tools.
//!
//! See `docs/workarounds.md` — do not remove the MO2 delays or the DLL renaming around
//! Creation Kit. The FO4Edit and archive adapters are not here yet: ADR-0002 defers them
//! to Phase B so they can be designed against real callers, and the invocation detail they
//! must reproduce lives with slices 3, 4 and 8 in `docs/future-features.md`.

mod creation_kit;
// `DllGuard` is deliberately not re-exported below. `disable` takes the crate-private
// `FileSpace`, so the guard cannot be constructed from outside the crate, and its only
// caller — `creation_kit` — reaches it through this module directly.
mod dll;

// Internal seams: crate-visible so the tests in `src/workflow/` can reach the recording
// adapters, but never part of what a Workflow Operation is handed.
pub(crate) mod clock;
pub(crate) mod process;
pub(crate) mod wait;

// Crate-visible, not public: `CreationKitOps` borrows the crate-private `ProcessRunner`,
// `Wait` and `FileSpace` ports, so it cannot be constructed from outside the crate anyway.
// `CkOperation` is not re-exported at all — Creation Kit's command grammar stays inside
// `creation_kit`, behind the four domain methods. `CkRun` is not re-exported either: the
// Generate Precombines Operation reads its log content without ever naming the type.
pub(crate) use creation_kit::{CkPorts, CreationKitOps, CreationKitPaths};
