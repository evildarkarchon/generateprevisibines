//! ENB/ReShade DLL disable/restore around Creation Kit runs (batch `:RunCK` / `:Done`).
//!
//! DLLs renamed to `*.dll-PJMdisabled`:
//! d3d11, d3d10, d3d9, dxgi, enbimgui, d3dcompiler_46e

use std::path::{Path, PathBuf};

use crate::error::Result;

const DLL_NAMES: &[&str] = &[
    "d3d11.dll",
    "d3d10.dll",
    "d3d9.dll",
    "dxgi.dll",
    "enbimgui.dll",
    "d3dcompiler_46e.dll",
];

const DISABLED_SUFFIX: &str = "-PJMdisabled";

/// Pair of `(disabled_path, original_path)` for restore on drop.
type RenamePair = (PathBuf, PathBuf);

/// RAII guard that restores DLL names on drop (including on panic).
#[derive(Debug)]
pub struct DllGuard {
    pairs: Vec<RenamePair>,
}

impl DllGuard {
    /// Disable conflicting DLLs before CK launch.
    pub fn disable(fallout4_dir: &Path) -> Result<Self> {
        let mut pairs = Vec::new();

        for name in DLL_NAMES {
            let original = fallout4_dir.join(name);
            if !original.is_file() {
                continue;
            }

            let disabled_name = format!("{name}{DISABLED_SUFFIX}");
            let disabled = fallout4_dir.join(disabled_name);

            if disabled.exists() {
                std::fs::remove_file(&disabled)?;
            }

            std::fs::rename(&original, &disabled)?;
            pairs.push((disabled, original));
        }

        Ok(Self { pairs })
    }

    /// Renames performed by this guard (for tests).
    #[must_use]
    pub fn rename_pairs(&self) -> &[(PathBuf, PathBuf)] {
        &self.pairs
    }
}

impl Drop for DllGuard {
    fn drop(&mut self) {
        for (disabled, original) in self.pairs.drain(..) {
            if !disabled.is_file() {
                continue;
            }
            if original.exists() {
                tracing::warn!(
                    original = %original.display(),
                    "DLL restore skipped: original path already exists"
                );
                continue;
            }
            if let Err(err) = std::fs::rename(&disabled, &original) {
                tracing::error!(
                    disabled = %disabled.display(),
                    original = %original.display(),
                    error = %err,
                    "failed to restore DLL after CK run"
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn renames_and_restores_on_drop() {
        let dir = tempdir().unwrap();
        let fo4 = dir.path();
        let dll = fo4.join("d3d11.dll");
        fs::write(&dll, b"x").unwrap();

        {
            let guard = DllGuard::disable(fo4).unwrap();
            assert_eq!(guard.rename_pairs().len(), 1);
            assert!(!dll.is_file());
            assert!(fo4.join("d3d11.dll-PJMdisabled").is_file());
        }

        assert!(dll.is_file());
        assert!(!fo4.join("d3d11.dll-PJMdisabled").is_file());
    }
}
