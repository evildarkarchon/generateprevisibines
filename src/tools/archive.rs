//! BA2 archive operations (`:Archive`, `:Extract`, `:AddToArchive` in batch).
//!
//! Required behaviors when implemented:
//! - Archive2: no append — extract, wait 5s, delete archive, re-pack (see workarounds doc)
//! - BSArch: pack with `-mt -fo4 -z`, optional append path
//! - Xbox mode: `-compression=XBox` for Archive2

use std::path::Path;

use crate::config::{ArchiveTool, BuildMode};

/// Placeholder for archive tool automation.
#[derive(Debug, Default)]
pub struct ArchiveOps;

impl ArchiveOps {
    /// Create or update plugin BA2 (not yet implemented).
    pub fn archive_folder(
        &self,
        _data_dir: &Path,
        _relative_folder: &str,
        _archive_name: &str,
        _tool: ArchiveTool,
        _mode: BuildMode,
    ) -> crate::error::Result<()> {
        Err(crate::error::Error::Other(
            "Archive automation not yet implemented".into(),
        ))
    }
}
