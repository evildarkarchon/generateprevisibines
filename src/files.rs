//! The `FileSpace` seam: path-shaped filesystem access for Workflow Operations.
//!
//! Workflow Operations decide success by observing what the external tools left on disk.
//! Those observations go through this seam so the rules — ordering, which error wins, and
//! how artifact paths are derived — stay above it and remain testable without a real
//! filesystem. Both adapters live here so the two-adapter justification for the seam is
//! visible in one place.
//!
//! The interface long had no create or write operation, because no production rule creates
//! an *artifact* — only the external tools do. That reasoning still holds for artifacts, and
//! is why nothing here writes a mesh, a `.uvd` or a plugin. Write operations exist now for a
//! different class of file the crate does author itself: the session log and the disabled-DLL
//! placeholders. Those went straight to `std::fs`, which put the machine's real `%TEMP%` and
//! the real Fallout 4 directory in the path of a unit test, and hid *when* the DLL guard
//! renames relative to the tool spawn.

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
// Every operation here has a production caller: `logging` routes the session log through
// `write`, `append` and `temp_dir`, and `DllGuard` renames through `rename`, `exists`,
// `is_file` and `remove_file`. The `dead_code` allow that once covered the unadopted half of
// the trait is gone with them; if one of these goes quiet again, delete it rather than
// re-adding the allow.
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

    /// Rename `from` to `to`, replacing any existing file at `to`.
    ///
    /// Returns [`crate::error::Error::Io`] when `from` does not exist, so this is not
    /// idempotent. Does **not** create parent directories: a `to` whose parent is missing is
    /// an error, not a silent `mkdir -p`.
    fn rename(&self, from: &Path, to: &Path) -> Result<()>;

    /// Whether anything exists at `path` — file **or** directory.
    ///
    /// Deliberately distinct from [`FileSpace::is_file`]. The DLL guard uses it to detect that
    /// something has reappeared at a path it is about to write to, and a directory counts;
    /// narrowing it to `is_file` would turn a deliberate skip into an attempted rename that
    /// cannot succeed.
    fn exists(&self, path: &Path) -> bool;

    /// Write `contents` to `path`, replacing any existing file rather than appending.
    ///
    /// Creates missing parent directories. Returns [`crate::error::Error::Io`] when the
    /// directories or the file cannot be created.
    fn write(&self, path: &Path, contents: &str) -> Result<()>;

    /// Append `contents` to `path`, creating the file when it is absent.
    ///
    /// Creates missing parent directories, like [`FileSpace::write`]. Returns
    /// [`crate::error::Error::Io`] when the file cannot be opened or extended.
    fn append(&self, path: &Path, contents: &str) -> Result<()>;

    /// Root for per-run temporary files.
    ///
    /// [`SystemFileSpace`] returns [`std::env::temp_dir`]; the in-memory adapter returns a
    /// synthetic root, so a test that logs never touches the machine's real `%TEMP%`.
    fn temp_dir(&self) -> PathBuf;
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

    fn rename(&self, from: &Path, to: &Path) -> Result<()> {
        std::fs::rename(from, to)?;
        Ok(())
    }

    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }

    fn write(&self, path: &Path, contents: &str) -> Result<()> {
        create_parent_directories(path)?;
        std::fs::write(path, contents)?;
        Ok(())
    }

    fn append(&self, path: &Path, contents: &str) -> Result<()> {
        use std::io::Write as _;

        create_parent_directories(path)?;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        file.write_all(contents.as_bytes())?;
        Ok(())
    }

    fn temp_dir(&self) -> PathBuf {
        std::env::temp_dir()
    }
}

/// Create `path`'s parent directory chain, tolerating a path that has no parent.
///
/// A bare file name (`"previs.log"`) yields `Some("")` from [`Path::parent`], and asking
/// `create_dir_all` for the empty path is an error on Windows, so the empty parent is skipped.
fn create_parent_directories(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }

    Ok(())
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

        fn rename(&self, from: &Path, to: &Path) -> Result<()> {
            let mut files = self.files.borrow_mut();
            let Some(contents) = files.remove(from) else {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("no such file in space: {}", from.display()),
                )
                .into());
            };

            // Replacing whatever sat at `to` matches `std::fs::rename` for files, which is
            // the only case the flat map can represent. The trait's "does not create parent
            // directories" clause is vacuous for the same reason, so it too is pinned on
            // `SystemFileSpace`.
            files.insert(to.to_path_buf(), contents);

            Ok(())
        }

        /// Coincides with [`InMemoryFileSpace::is_file`], and that is correct rather than a
        /// gap to close.
        ///
        /// This adapter is a flat map with no directory concept, by the explicit design noted
        /// on the struct: file-versus-directory discrimination is verified against
        /// [`super::SystemFileSpace`], where directories actually live. Adding directory
        /// modelling here to make the two answers differ would contradict that. Any test that
        /// turns on the distinction belongs on `SystemFileSpace` with `tempfile`.
        fn exists(&self, path: &Path) -> bool {
            self.is_file(path)
        }

        fn write(&self, path: &Path, contents: &str) -> Result<()> {
            // Parent creation is meaningless in a flat map, so the "creates parents" half of
            // the contract is vacuously satisfied here and pinned on `SystemFileSpace`.
            self.files
                .borrow_mut()
                .insert(path.to_path_buf(), Some(contents.to_owned()));

            Ok(())
        }

        fn append(&self, path: &Path, contents: &str) -> Result<()> {
            // Parent creation is vacuous here exactly as it is in `write` above.
            let mut files = self.files.borrow_mut();
            let entry = files.entry(path.to_path_buf()).or_default();
            match entry {
                Some(existing) => existing.push_str(contents),
                // A recorded-but-contentless file means "unreadable", so an appender has
                // nothing to preserve and starts from empty rather than failing.
                None => *entry = Some(contents.to_owned()),
            }

            Ok(())
        }

        fn temp_dir(&self) -> PathBuf {
            PathBuf::from(IN_MEMORY_TEMP_ROOT)
        }
    }

    /// The synthetic temporary root handed out by [`InMemoryFileSpace::temp_dir`].
    ///
    /// Deliberately not a real path: if a caller ever escapes the seam and hands one of these
    /// paths to `std::fs`, the failure should be obvious rather than land in the machine's
    /// real `%TEMP%`.
    const IN_MEMORY_TEMP_ROOT: &str = "in-memory-temp";
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
    fn in_memory_space_renames_carrying_contents_and_replacing_the_destination() {
        let space = InMemoryFileSpace::new();
        let from = PathBuf::from("Fallout4").join("d3d11.dll");
        let to = PathBuf::from("Fallout4").join("d3d11.dll-PJMdisabled");
        space.add_file_with_contents(&from, "enb");
        space.add_file_with_contents(&to, "stale");

        space.rename(&from, &to).unwrap();

        assert!(!space.is_file(&from));
        assert_eq!(space.read_lossy(&to).unwrap(), "enb");
    }

    #[test]
    fn in_memory_space_rename_of_a_missing_source_errors() {
        let space = InMemoryFileSpace::new();

        assert!(
            space
                .rename(Path::new("absent.dll"), Path::new("absent.dll-disabled"))
                .is_err()
        );
    }

    /// `exists` coincides with `is_file` here, and that is the design — see the adapter's
    /// own doc comment. The two are pinned apart on [`SystemFileSpace`].
    #[test]
    fn in_memory_space_exists_answers_for_recorded_paths_only() {
        let space = InMemoryFileSpace::new();
        let dll = PathBuf::from("Fallout4").join("d3d11.dll");

        assert!(!space.exists(&dll));

        space.add_file(&dll);

        assert!(space.exists(&dll));
        assert_eq!(space.exists(&dll), space.is_file(&dll));
    }

    #[test]
    fn in_memory_space_write_replaces_rather_than_appends() {
        let space = InMemoryFileSpace::new();
        let log = PathBuf::from("temp").join("previs.log");

        space.write(&log, "first\n").unwrap();
        assert_eq!(space.read_lossy(&log).unwrap(), "first\n");

        space.write(&log, "second\n").unwrap();
        assert_eq!(space.read_lossy(&log).unwrap(), "second\n");
    }

    #[test]
    fn in_memory_space_append_creates_then_accumulates() {
        let space = InMemoryFileSpace::new();
        let log = PathBuf::from("temp").join("previs.log");

        space.append(&log, "first\n").unwrap();
        assert!(space.is_file(&log));

        space.append(&log, "second\n").unwrap();
        assert_eq!(space.read_lossy(&log).unwrap(), "first\nsecond\n");
    }

    /// Appending to a recorded-but-contentless file treats it as empty rather than erroring:
    /// the contentless marker means "unreadable", and a writer has nothing to preserve.
    #[test]
    fn in_memory_space_append_to_a_contentless_file_starts_from_empty() {
        let space = InMemoryFileSpace::new();
        let log = PathBuf::from("temp").join("previs.log");
        space.add_file(&log);

        space.append(&log, "line\n").unwrap();

        assert_eq!(space.read_lossy(&log).unwrap(), "line\n");
    }

    #[test]
    fn in_memory_space_temp_dir_is_synthetic() {
        let space = InMemoryFileSpace::new();

        assert_ne!(space.temp_dir(), std::env::temp_dir());
    }

    #[test]
    fn system_space_renames_replacing_the_destination() {
        let dir = tempdir().unwrap();
        let space = SystemFileSpace;
        let from = dir.path().join("d3d11.dll");
        let to = dir.path().join("d3d11.dll-PJMdisabled");
        fs::write(&from, b"enb").unwrap();
        fs::write(&to, b"stale").unwrap();

        space.rename(&from, &to).unwrap();

        assert!(!from.exists());
        assert_eq!(fs::read(&to).unwrap(), b"enb");
    }

    #[test]
    fn system_space_rename_of_a_missing_source_errors() {
        let dir = tempdir().unwrap();
        let space = SystemFileSpace;

        assert!(
            space
                .rename(&dir.path().join("absent.dll"), &dir.path().join("moved.dll"))
                .is_err()
        );
    }

    #[test]
    fn system_space_rename_does_not_create_parent_directories() {
        let dir = tempdir().unwrap();
        let space = SystemFileSpace;
        let from = dir.path().join("d3d11.dll");
        fs::write(&from, b"enb").unwrap();
        let to = dir.path().join("absent").join("d3d11.dll");

        assert!(space.rename(&from, &to).is_err());
        assert!(from.is_file());
        assert!(!to.parent().unwrap().exists());
    }

    /// The one place `exists` and `is_file` are pinned *apart*, which is why `exists` is a
    /// separate operation: a directory at the queried path exists but is not a file.
    #[test]
    fn system_space_exists_is_true_for_a_directory_where_is_file_is_false() {
        let dir = tempdir().unwrap();
        let space = SystemFileSpace;
        let subdirectory = dir.path().join("d3d11.dll");
        fs::create_dir_all(&subdirectory).unwrap();

        assert!(space.exists(&subdirectory));
        assert!(!space.is_file(&subdirectory));
        assert!(!space.exists(&dir.path().join("absent")));
    }

    #[test]
    fn system_space_write_creates_parents_and_replaces_rather_than_appends() {
        let dir = tempdir().unwrap();
        let space = SystemFileSpace;
        let log = dir.path().join("logs").join("previs.log");

        space.write(&log, "first\n").unwrap();
        assert_eq!(space.read_lossy(&log).unwrap(), "first\n");

        space.write(&log, "second\n").unwrap();
        assert_eq!(space.read_lossy(&log).unwrap(), "second\n");
    }

    #[test]
    fn system_space_append_creates_then_accumulates() {
        let dir = tempdir().unwrap();
        let space = SystemFileSpace;
        let log = dir.path().join("logs").join("previs.log");

        space.append(&log, "first\n").unwrap();
        assert!(log.is_file());

        space.append(&log, "second\n").unwrap();
        assert_eq!(space.read_lossy(&log).unwrap(), "first\nsecond\n");
    }

    #[test]
    fn system_space_temp_dir_is_the_process_temp_dir() {
        let space = SystemFileSpace;

        assert_eq!(space.temp_dir(), std::env::temp_dir());
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
