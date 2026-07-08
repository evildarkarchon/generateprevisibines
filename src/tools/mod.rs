//! Wrappers for external tools.
//!
//! See `docs/workarounds.md` — do not remove MO2 delays, DLL renaming, `FO4Edit`
//! keystroke automation, or Archive2 extract-repack behavior when implementing these.

mod archive;
mod creation_kit;
mod dll;
mod fo4edit;

pub use archive::ArchiveOps;
pub use creation_kit::{CkOperation, CreationKitOps};
pub use dll::DllGuard;
pub use fo4edit::Fo4EditOps;

/// Shared context passed to each tool invocation.
#[derive(Debug, Clone)]
pub struct ToolContext {
    pub session_log: Option<std::path::PathBuf>,
    pub unattended_log: Option<std::path::PathBuf>,
    pub fallout4_dir: std::path::PathBuf,
    pub creation_kit: std::path::PathBuf,
    pub ck_log_path: std::path::PathBuf,
}

impl Default for ToolContext {
    fn default() -> Self {
        Self {
            session_log: None,
            unattended_log: None,
            fallout4_dir: std::path::PathBuf::new(),
            creation_kit: std::path::PathBuf::new(),
            ck_log_path: std::path::PathBuf::new(),
        }
    }
}
