//! The `FileSpace` seam: path-shaped filesystem access for Workflow Operations.
//!
//! Workflow Operations decide success by observing what the external tools left on disk.
//! Those observations go through this seam so the rules — ordering, which error wins, and
//! how artifact paths are derived — stay above it and remain testable without a real
//! filesystem. Both adapters live here so the two-adapter justification for the seam is
//! visible in one place.
//!
//! The interface deliberately has no create or write operation: no production rule creates
//! an artifact, only the external tools do.

use std::path::{Path, PathBuf};

use crate::error::Result;

/// Path-shaped filesystem access used to observe and clean up external-tool artifacts.
///
/// Every operation takes a shared reference: the space is observed at several moments
/// separated by mutations (a resume clear, stale-artifact cleanup, and the external tool
/// itself), so answers may legitimately differ between calls.
///
/// Implementors must be `Debug` so the domain types that hold a `FileSpace` — the
/// Precombine Workspace among them — can keep deriving `Debug`.
pub(crate) trait FileSpace: std::fmt::Debug {
    /// Whether `path` names an existing file (not a directory).
    fn is_file(&self, path: &Path) -> bool;

    /// Find the first file with `extension` beneath `directory`, descending recursively.
    ///
    /// The extension is matched case-insensitively and without a leading dot. A directory
    /// whose own name ends in `extension` is not a match. Returns `None` when `directory`
    /// does not exist or holds no such file.
    fn find_first_file_with_extension(&self, directory: &Path, extension: &str) -> Option<PathBuf>;

    /// Remove a single file.
    ///
    /// Returns [`crate::error::Error::Io`] when the file cannot be removed. Callers are
    /// expected to check [`FileSpace::is_file`] first; this is not idempotent.
    fn remove_file(&self, path: &Path) -> Result<()>;

    /// Remove a directory and everything beneath it, idempotently.
    ///
    /// A missing directory is success, so callers need no separate existence check.
    /// Returns [`crate::error::Error::Io`] for any other removal failure.
    fn remove_dir_all(&self, directory: &Path) -> Result<()>;

    /// Read a file's contents tolerantly, replacing non-UTF-8 bytes rather than failing.
    ///
    /// External tool logs are not guaranteed to be valid UTF-8. Returns
    /// [`crate::error::Error::Io`] when the file cannot be read at all.
    fn read_lossy(&self, path: &Path) -> Result<String>;
}

/// The production `FileSpace`, backed by the standard library.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct SystemFileSpace;

impl FileSpace for SystemFileSpace {
    fn is_file(&self, path: &Path) -> bool {
        path.is_file()
    }

    fn find_first_file_with_extension(&self, directory: &Path, extension: &str) -> Option<PathBuf> {
        find_first_file_with_extension(directory, extension)
    }

    fn remove_file(&self, path: &Path) -> Result<()> {
        std::fs::remove_file(path)?;
        Ok(())
    }

    fn remove_dir_all(&self, directory: &Path) -> Result<()> {
        match std::fs::remove_dir_all(directory) {
            Ok(()) => Ok(()),
            // A directory that is already gone is the state the caller asked for.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

    fn read_lossy(&self, path: &Path) -> Result<String> {
        // The crate's lossy-read helper stays the single definition of tolerant reading;
        // it still serves the logging, toolchain and validation modules directly.
        crate::text::read_lossy(path)
    }
}

/// Depth-first search for the first file matching `extension` beneath `directory`.
///
/// An unreadable directory yields `None` rather than an error: the callers ask an
/// existence question, and "cannot look" answers it the same way "nothing there" does.
fn find_first_file_with_extension(directory: &Path, extension: &str) -> Option<PathBuf> {
    let entries = std::fs::read_dir(directory).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_first_file_with_extension(&path, extension) {
                return Some(found);
            }
        } else if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case(extension))
        {
            return Some(path);
        }
    }
    None
}

#[cfg(test)]
pub(crate) use in_memory::InMemoryFileSpace;

#[cfg(test)]
mod in_memory {
    use std::cell::RefCell;
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};

    use super::FileSpace;
    use crate::error::Result;

    /// A test `FileSpace`: a flat, ordered collection of file paths with optional contents.
    ///
    /// Deliberately simple. Directory-versus-file discrimination and recursive descent are
    /// verified against [`super::SystemFileSpace`], where they actually live, so this
    /// adapter only needs to answer path questions the rules ask.
    ///
    /// Interior mutability mirrors the real thing: the space is mutated between the
    /// observations a Workflow Operation makes.
    #[derive(Debug, Default)]
    pub(crate) struct InMemoryFileSpace {
        files: RefCell<BTreeMap<PathBuf, Option<String>>>,
    }

    impl InMemoryFileSpace {
        /// Create an empty space.
        #[must_use]
        pub(crate) fn new() -> Self {
            Self::default()
        }

        /// Record `path` as an existing file with no readable contents.
        pub(crate) fn add_file(&self, path: impl Into<PathBuf>) {
            self.files.borrow_mut().insert(path.into(), None);
        }

        /// Record `path` as an existing file whose tolerant read yields `contents`.
        pub(crate) fn add_file_with_contents(
            &self,
            path: impl Into<PathBuf>,
            contents: impl Into<String>,
        ) {
            self.files
                .borrow_mut()
                .insert(path.into(), Some(contents.into()));
        }
    }

    impl FileSpace for InMemoryFileSpace {
        fn is_file(&self, path: &Path) -> bool {
            self.files.borrow().contains_key(path)
        }

        fn find_first_file_with_extension(
            &self,
            directory: &Path,
            extension: &str,
        ) -> Option<PathBuf> {
            self.files
                .borrow()
                .keys()
                .find(|path| {
                    path.starts_with(directory)
                        && path
                            .extension()
                            .is_some_and(|e| e.eq_ignore_ascii_case(extension))
                })
                .cloned()
        }

        fn remove_file(&self, path: &Path) -> Result<()> {
            if self.files.borrow_mut().remove(path).is_none() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("no such file in space: {}", path.display()),
                )
                .into());
            }

            Ok(())
        }

        fn remove_dir_all(&self, directory: &Path) -> Result<()> {
            self.files
                .borrow_mut()
                .retain(|path, _| !path.starts_with(directory));

            Ok(())
        }

        fn read_lossy(&self, path: &Path) -> Result<String> {
            match self.files.borrow().get(path) {
                Some(Some(contents)) => Ok(contents.clone()),
                // A recorded file with no contents reads like an unreadable file, which is
                // what lets a test say "the log exists but says nothing useful".
                Some(None) | None => Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("no readable contents in space: {}", path.display()),
                )
                .into()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use tempfile::tempdir;

    use super::{FileSpace, InMemoryFileSpace, SystemFileSpace};

    #[test]
    fn in_memory_space_answers_path_questions_and_mutates() {
        let space = InMemoryFileSpace::new();
        let root = PathBuf::from("Data");
        let mesh = root.join("meshes").join("precombined").join("cell.NIF");
        let log = root.join("CK.log");

        assert!(!space.is_file(&mesh));

        space.add_file(&mesh);
        space.add_file_with_contents(&log, "DEFAULT: OUT OF HANDLE ARRAY ENTRIES\n");

        assert!(space.is_file(&mesh));
        assert_eq!(
            space.find_first_file_with_extension(&root.join("meshes"), "nif"),
            Some(mesh.clone())
        );
        assert!(
            space
                .find_first_file_with_extension(&root.join("vis"), "uvd")
                .is_none()
        );
        assert!(space.read_lossy(&log).unwrap().contains("HANDLE ARRAY"));
        // A recorded-but-contentless file reads as unreadable.
        assert!(space.read_lossy(&mesh).is_err());

        space.remove_file(&log).unwrap();
        assert!(!space.is_file(&log));
        assert!(space.remove_file(&log).is_err());

        space.remove_dir_all(&root.join("meshes")).unwrap();
        assert!(!space.is_file(&mesh));
        // Removing an absent tree is success.
        space.remove_dir_all(&root.join("meshes")).unwrap();
    }

    #[test]
    fn system_space_finds_nested_file_ignoring_case_and_other_extensions() {
        let dir = tempdir().unwrap();
        let space = SystemFileSpace;
        let nested = dir.path().join("a").join("b").join("mesh.NIF");
        fs::create_dir_all(nested.parent().unwrap()).unwrap();
        fs::write(&nested, b"nif").unwrap();
        fs::write(dir.path().join("notes.txt"), b"text").unwrap();

        assert_eq!(
            space.find_first_file_with_extension(dir.path(), "nif"),
            Some(nested)
        );
        assert!(
            space
                .find_first_file_with_extension(dir.path(), "uvd")
                .is_none()
        );
    }

    /// Not a duplicate of the nested-`.nif` test above, despite exercising the same method.
    ///
    /// Precombined meshes and compiled visibility files are separate artifacts in separate
    /// locations — `meshes\precombined\*.nif` and `vis\*.uvd`. The rules search each one
    /// independently, so each descent is pinned independently.
    #[test]
    fn system_space_finds_a_nested_visibility_file() {
        let dir = tempdir().unwrap();
        let space = SystemFileSpace;
        let nested = dir.path().join("vis").join("MyMod").join("cell.UVD");
        fs::create_dir_all(nested.parent().unwrap()).unwrap();
        fs::write(&nested, b"uvd").unwrap();

        assert_eq!(
            space.find_first_file_with_extension(&dir.path().join("vis"), "uvd"),
            Some(nested)
        );
    }

    #[test]
    fn system_space_does_not_mistake_a_directory_for_a_file() {
        let dir = tempdir().unwrap();
        let space = SystemFileSpace;
        // A directory whose own name ends in the artifact extension is not a match.
        let looks_like = dir.path().join("LooksLike.uvd");
        fs::create_dir_all(&looks_like).unwrap();

        assert!(!space.is_file(&looks_like));
        assert!(
            space
                .find_first_file_with_extension(dir.path(), "uvd")
                .is_none()
        );
    }

    #[test]
    fn system_space_ignores_a_nested_file_with_a_different_extension() {
        let dir = tempdir().unwrap();
        let space = SystemFileSpace;
        let nested_other = dir.path().join("MyMod").join("cell.txt");
        fs::create_dir_all(nested_other.parent().unwrap()).unwrap();
        fs::write(nested_other, b"text").unwrap();

        assert!(
            space
                .find_first_file_with_extension(dir.path(), "uvd")
                .is_none()
        );
    }

    #[test]
    fn system_space_finds_nothing_beneath_a_missing_directory() {
        let dir = tempdir().unwrap();
        let space = SystemFileSpace;

        assert!(
            space
                .find_first_file_with_extension(&dir.path().join("absent"), "nif")
                .is_none()
        );
    }

    #[test]
    fn system_space_reads_non_utf8_contents_tolerantly() {
        let dir = tempdir().unwrap();
        let space = SystemFileSpace;
        let log = dir.path().join("CK.log");
        fs::write(&log, b"\xFFDEFAULT: OUT OF HANDLE ARRAY ENTRIES\n").unwrap();

        assert!(
            space
                .read_lossy(&log)
                .unwrap()
                .contains("DEFAULT: OUT OF HANDLE ARRAY ENTRIES")
        );
        assert!(space.read_lossy(&dir.path().join("absent.log")).is_err());
    }

    #[test]
    fn system_space_removes_files_and_trees_with_idempotent_tree_removal() {
        let dir = tempdir().unwrap();
        let space = SystemFileSpace;
        let tree = dir.path().join("meshes").join("precombined");
        let mesh = tree.join("cell").join("mesh.nif");
        fs::create_dir_all(mesh.parent().unwrap()).unwrap();
        fs::write(&mesh, b"nif").unwrap();
        let stale: &Path = &dir.path().join("CombinedObjects.esp");
        fs::write(stale, b"esp").unwrap();

        space.remove_file(stale).unwrap();
        assert!(!stale.exists());

        space.remove_dir_all(&tree).unwrap();
        assert!(!tree.exists());
        space.remove_dir_all(&tree).unwrap();
    }
}
