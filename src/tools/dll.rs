//! ENB/ReShade DLL disable/restore around Creation Kit runs (batch `:RunCK` / `:Done`).
//!
//! DLLs renamed to `*.dll-PJMdisabled`:
//! d3d11, d3d10, d3d9, dxgi, enbimgui, `d3dcompiler_46e`

use std::path::{Path, PathBuf};

use crate::error::Result;
use crate::files::FileSpace;

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
///
/// The guard borrows the [`FileSpace`] it renamed through so that [`Drop`] can undo the
/// renames in the same space. That is what ties the lifetime parameter to the caller: the
/// guard must not outlive the space, and the restore must not land somewhere else.
#[derive(Debug)]
pub(crate) struct DllGuard<'a> {
    files: &'a dyn FileSpace,
    pairs: Vec<RenamePair>,
}

impl<'a> DllGuard<'a> {
    /// Disable conflicting DLLs before CK launch.
    ///
    /// Creation Kit crashes when ENB/ReShade DLLs are loaded, so they are moved out of its
    /// way for the duration of the run — see `docs/workarounds.md` §3. This is a required
    /// workaround, not an optimization to remove.
    ///
    /// Renames every DLL in [`DLL_NAMES`] that exists under `fallout4_dir` to
    /// `<name>-PJMdisabled`, recording each move for restore on drop. Returns
    /// [`crate::error::Error::Io`] when clearing the target path or the rename itself fails;
    /// the partial renames made so far are rolled back by the guard's own `Drop`, since the
    /// half-built guard is dropped as the error propagates.
    pub(crate) fn disable(files: &'a dyn FileSpace, fallout4_dir: &Path) -> Result<Self> {
        let mut guard = Self {
            files,
            pairs: Vec::new(),
        };

        for name in DLL_NAMES {
            let original = fallout4_dir.join(name);
            if !files.is_file(&original) {
                continue;
            }

            let disabled_name = format!("{name}{DISABLED_SUFFIX}");
            let disabled = fallout4_dir.join(disabled_name);

            // `exists`, not `is_file` — see the distinction on [`FileSpace::exists`]. Here the
            // outcome would survive the swap (the failure would just move from `remove_file`
            // to `rename`), but there is no reason to accept a different failure mechanism
            // when the question being asked is "is anything on this path?".
            if files.exists(&disabled) {
                files.remove_file(&disabled)?;
            }

            files.rename(&original, &disabled)?;
            guard.pairs.push((disabled, original));
        }

        Ok(guard)
    }

    /// Renames performed by this guard, as `(disabled_path, original_path)` pairs.
    ///
    /// Gated to test builds: nothing in production reads the guard back, and leaving it
    /// crate-visible everywhere would only need a `dead_code` allow to say the same thing.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn rename_pairs(&self) -> &[RenamePair] {
        &self.pairs
    }
}

impl Drop for DllGuard<'_> {
    fn drop(&mut self) {
        for (disabled, original) in self.pairs.drain(..) {
            if !self.files.is_file(&disabled) {
                continue;
            }
            // `exists`, not `is_file` — see the distinction on [`FileSpace::exists`]. Unlike
            // the call in `disable`, the two genuinely differ here: the question is whether
            // *anything* has reappeared at the original path, and a directory counts.
            // Narrowing this to `is_file` would turn a deliberate skip-with-warning into an
            // attempted rename that cannot succeed, reported as a restore failure.
            if self.files.exists(&original) {
                tracing::warn!(
                    original = %original.display(),
                    "DLL restore skipped: original path already exists"
                );
                continue;
            }
            // `Drop` cannot report an error, and a failed restore must not abort unwinding,
            // so this logs rather than panics.
            if let Err(err) = self.files.rename(&disabled, &original) {
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

    use crate::files::{InMemoryFileSpace, SystemFileSpace};

    #[test]
    fn renames_and_restores_on_drop() {
        let files = InMemoryFileSpace::new();
        let fo4 = Path::new("Fallout 4");
        let dll = fo4.join("d3d11.dll");
        let disabled = fo4.join("d3d11.dll-PJMdisabled");
        files.add_file_with_contents(dll.clone(), "x");

        {
            let guard = DllGuard::disable(&files, fo4).unwrap();
            assert_eq!(guard.rename_pairs().len(), 1);
            assert!(!files.is_file(&dll));
            assert!(files.is_file(&disabled));
        }

        assert!(files.is_file(&dll));
        assert!(!files.is_file(&disabled));
        // Contents survive the round trip, which is what distinguishes a restore from a
        // recreated placeholder at the same path.
        assert_eq!(files.read_lossy(&dll).unwrap(), "x");
    }

    // Stays on `SystemFileSpace`: the failure is forced by putting a *directory* where the
    // rename target goes, and `InMemoryFileSpace` is a flat map with no directory concept by
    // design. File-versus-directory discrimination is verified where it actually lives.
    #[test]
    fn restores_previous_renames_when_disable_fails() {
        let dir = tempdir().unwrap();
        let fo4 = dir.path();
        let first = fo4.join("d3d11.dll");
        let second = fo4.join("d3d10.dll");
        let failing = fo4.join("d3d9.dll");
        fs::write(&first, b"first").unwrap();
        fs::write(&second, b"second").unwrap();
        fs::write(&failing, b"third").unwrap();
        fs::create_dir(fo4.join("d3d9.dll-PJMdisabled")).unwrap();

        let err = DllGuard::disable(&SystemFileSpace, fo4).unwrap_err();

        assert!(matches!(err, crate::error::Error::Io(_)));
        assert!(first.is_file());
        assert!(second.is_file());
        assert!(failing.is_file());
        assert!(!fo4.join("d3d11.dll-PJMdisabled").exists());
        assert!(!fo4.join("d3d10.dll-PJMdisabled").exists());
    }
}
