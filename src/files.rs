//! The `FileSpace` seam: path-shaped filesystem access for Workflow Operations.
//!
//! Workflow Operations decide success by observing what the external tools left on disk.
//! Those observations go through this seam so the rules — ordering, which error wins, and
//! how artifact paths are derived — stay above it and remain testable without a real
//! filesystem. Both adapters live here so the two-adapter justification for the seam is
//! visible in one place.
//!
//! The interface long had no create or write operation, because no production rule creates
//! an *artifact* — only the external tools do. That reasoning still holds for generated
//! meshes and `.uvd` files. The crate does author session logs and disabled-DLL placeholders,
//! while seed plugins require a separate opaque copy that never interprets their bytes.

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
// `write`, `append` and `temp_dir`, `DllGuard` renames through `rename`, `exists`, `is_file`
// and `remove_file`, and seed-plugin setup uses `copy`. The Archive episode creates and fills
// its work folder through `create_dir_all`, `is_dir` and `move_dir`, and the run-start restore
// of leftover work folders lists them through `child_dirs`. The `dead_code` allow that once
// covered the unadopted half of the trait is gone with them; if one of these goes quiet again,
// delete it rather than re-adding the allow.
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
    /// A missing directory is success, so callers need no separate existence check. Otherwise
    /// it succeeds only when the directory is gone afterwards. Returns
    /// [`crate::error::Error::Io`], naming the directory, in every other case.
    fn remove_dir_all(&self, directory: &Path) -> Result<()>;

    /// Read a file's contents tolerantly, replacing non-UTF-8 bytes rather than failing.
    ///
    /// External tool logs are not guaranteed to be valid UTF-8. Returns
    /// [`crate::error::Error::Io`] when the file cannot be read at all.
    fn read_lossy(&self, path: &Path) -> Result<String>;

    /// Copy `from` to `to` without interpreting the file's bytes.
    ///
    /// Creates missing destination parent directories and replaces an existing destination.
    /// Returns [`crate::error::Error::Io`] when directory creation or copying fails.
    fn copy(&self, from: &Path, to: &Path) -> Result<()>;

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

    /// Create `path` and any missing parent directories.
    ///
    /// An existing directory is success. Returns [`crate::error::Error::Io`] when a directory
    /// cannot be created, including when a file already sits at `path`.
    fn create_dir_all(&self, path: &Path) -> Result<()>;

    /// Whether `path` names an existing directory (not a file).
    fn is_dir(&self, path: &Path) -> bool;

    /// The immediate subdirectories of `directory`, in no particular order.
    ///
    /// Files and deeper descendants are not listed. A missing or unreadable `directory`
    /// yields an empty list, as in [`FileSpace::find_first_file_with_extension`]: "cannot
    /// look" answers the caller's question the same way "nothing there" does.
    fn child_dirs(&self, directory: &Path) -> Vec<PathBuf>;

    /// Move the directory `from`, with everything beneath it, to `to`.
    ///
    /// `from` must exist and `to` must not. As with [`FileSpace::rename`], the parent of `to`
    /// is **not** created: a missing parent is an error. Returns [`crate::error::Error::Io`]
    /// in each of those cases. This is a separate operation rather than a wider `rename`,
    /// because `rename` replaces an existing destination, which is a file contract.
    fn move_dir(&self, from: &Path, to: &Path) -> Result<()>;

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
        // A directory that is already gone is the state the caller asked for.
        if is_gone(directory) {
            return Ok(());
        }

        // Required workaround, not a reinvented `std::fs::remove_dir_all`: under MO2's usvfs
        // that call fails with os error 2 on any folder with a subfolder, because it opens
        // children relative to the parent's handle and usvfs reroutes only full-path calls.
        // See docs/workarounds.md § "Full-path directory removal under MO2". Do not simplify
        // this back.
        let walked = remove_tree_by_full_paths(directory);

        // The walk's own result is not trusted either way: the probe saw usvfs report a tree
        // as missing while it was still enumerable. Only the folder's absence counts.
        if is_gone(directory) {
            return Ok(());
        }

        let reason = match walked {
            Ok(()) => "it is still present after removal".to_owned(),
            Err(error) => error.to_string(),
        };
        Err(std::io::Error::other(format!(
            "could not remove directory {}: {reason}",
            directory.display()
        ))
        .into())
    }

    fn read_lossy(&self, path: &Path) -> Result<String> {
        // The crate's lossy-read helper stays the single definition of tolerant reading;
        // it still serves the logging, toolchain and validation modules directly.
        crate::text::read_lossy(path)
    }

    fn copy(&self, from: &Path, to: &Path) -> Result<()> {
        create_parent_directories(to)?;
        // Plugin files are opaque binary artifacts; routing them through a text read would
        // silently replace invalid UTF-8 and corrupt the destination.
        std::fs::copy(from, to)?;
        Ok(())
    }

    fn rename(&self, from: &Path, to: &Path) -> Result<()> {
        std::fs::rename(from, to)?;
        Ok(())
    }

    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }

    fn create_dir_all(&self, path: &Path) -> Result<()> {
        std::fs::create_dir_all(path)?;
        Ok(())
    }

    fn is_dir(&self, path: &Path) -> bool {
        path.is_dir()
    }

    fn child_dirs(&self, directory: &Path) -> Vec<PathBuf> {
        let Ok(entries) = std::fs::read_dir(directory) else {
            return Vec::new();
        };

        entries
            .flatten()
            // `DirEntry::file_type` does not follow links, so a junction is not listed as a
            // folder the caller might go on to move or remove.
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
            .map(|entry| entry.path())
            .collect()
    }

    fn move_dir(&self, from: &Path, to: &Path) -> Result<()> {
        // `std::fs::rename` replaces an existing *empty* directory on some platforms, so the
        // "`to` must be absent" half of the contract is checked here rather than left to it.
        if to.exists() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                format!("cannot move a directory onto {}: it exists", to.display()),
            )
            .into());
        }

        // The Archive probe (#44) moved `Data\meshes\precombined` and `Data\vis`, physically in
        // MO2's `overwrite`, into the work folder and back with exactly this call.
        std::fs::rename(from, to)?;
        Ok(())
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
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)?;
    }

    Ok(())
}

/// Whether `directory` is really gone: nothing exists there **and** it cannot be listed.
///
/// Both halves are asked because the Archive probe (#44) saw them disagree under MO2's
/// usvfs, with a tree reported missing while it was still enumerable.
fn is_gone(directory: &Path) -> bool {
    !directory.exists() && std::fs::read_dir(directory).is_err()
}

/// Remove `directory` and everything beneath it, deepest first, using only full-path calls.
///
/// Each file goes through `std::fs::remove_file` and each folder through
/// `std::fs::remove_dir` (`DeleteFileW` and `RemoveDirectoryW` on Windows), which is what
/// `RD /S` does and what usvfs reroutes. Stops at the first failure; the caller decides the
/// outcome by checking whether the folder is gone, not by this result alone.
fn remove_tree_by_full_paths(directory: &Path) -> std::io::Result<()> {
    // Listed in full before anything is removed, so removal never races the enumeration.
    let entries = std::fs::read_dir(directory)?.collect::<std::io::Result<Vec<_>>>()?;
    for entry in entries {
        let path = entry.path();
        let kind = entry.file_type()?;
        if kind.is_dir() {
            remove_tree_by_full_paths(&path)?;
        } else if kind.is_symlink() {
            // `file_type` does not follow links, so a junction or directory symlink is never
            // descended into. Windows removes those with `remove_dir`, leaving the target alone.
            std::fs::remove_file(&path).or_else(|_| std::fs::remove_dir(&path))?;
        } else {
            std::fs::remove_file(&path)?;
        }
    }

    std::fs::remove_dir(directory)
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
    use std::collections::{BTreeMap, BTreeSet};
    use std::path::{Path, PathBuf};

    use super::FileSpace;
    use crate::error::Result;

    /// A test `FileSpace`: a flat, ordered collection of file paths with optional contents.
    ///
    /// Deliberately simple. Directory-versus-file discrimination and recursive descent are
    /// verified against [`super::SystemFileSpace`], where they actually live, so this
    /// adapter only needs to answer path questions the rules ask. `is_dir` and `child_dirs`
    /// are answered by prefix, from the recorded files beneath a path, so an empty directory
    /// cannot be represented.
    ///
    /// Interior mutability mirrors the real thing: the space is mutated between the
    /// observations a Workflow Operation makes.
    #[derive(Debug, Default)]
    pub(crate) struct InMemoryFileSpace {
        files: RefCell<BTreeMap<PathBuf, Option<Vec<u8>>>>,
        /// Paths whose appends fail, standing in for a session log something else has locked.
        refused_appends: RefCell<BTreeSet<PathBuf>>,
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
                .insert(path.into(), Some(contents.into().into_bytes()));
        }

        /// Record `path` as an existing file with an opaque byte payload.
        pub(crate) fn add_file_with_bytes(
            &self,
            path: impl Into<PathBuf>,
            contents: impl Into<Vec<u8>>,
        ) {
            self.files
                .borrow_mut()
                .insert(path.into(), Some(contents.into()));
        }

        /// Make every later append to `path` fail with an `Io` error, as a locked file would.
        ///
        /// Only appends: a test that needs the session log to stop growing mid-run still wants
        /// the reads that check what reached it to work.
        pub(crate) fn refuse_appends_to(&self, path: impl Into<PathBuf>) {
            self.refused_appends.borrow_mut().insert(path.into());
        }

        /// Return a copy of `path`'s byte payload, or `None` when it is absent or contentless.
        #[must_use]
        pub(crate) fn contents(&self, path: &Path) -> Option<Vec<u8>> {
            self.files.borrow().get(path).cloned().flatten()
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
                Some(Some(contents)) => Ok(String::from_utf8_lossy(contents).into_owned()),
                // A recorded file with no contents reads like an unreadable file, which is
                // what lets a test say "the log exists but says nothing useful".
                Some(None) | None => Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("no readable contents in space: {}", path.display()),
                )
                .into()),
            }
        }

        fn copy(&self, from: &Path, to: &Path) -> Result<()> {
            let mut files = self.files.borrow_mut();
            let Some(contents) = files.get(from).cloned() else {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("no such file in space: {}", from.display()),
                )
                .into());
            };

            // Clone the optional bytes as-is: `None` is a present contentless file, while a
            // lossy text round-trip would both collapse that state and corrupt plugin bytes.
            files.insert(to.to_path_buf(), contents);
            Ok(())
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

        fn create_dir_all(&self, _path: &Path) -> Result<()> {
            // An empty directory cannot be represented in the flat map, so creation is vacuous
            // here, as parent creation is in `write`. It is pinned on `SystemFileSpace`.
            Ok(())
        }

        /// True when any recorded file lies strictly beneath `path`: a derived prefix answer,
        /// not directory modelling, so [`InMemoryFileSpace::exists`] is left as it is.
        fn is_dir(&self, path: &Path) -> bool {
            self.files
                .borrow()
                .keys()
                .any(|file| lies_strictly_beneath(file, path))
        }

        /// The distinct first components beneath `directory` of recorded files at least two
        /// levels below it, joined back onto `directory`. Answered by prefix, like `is_dir`.
        fn child_dirs(&self, directory: &Path) -> Vec<PathBuf> {
            let children: BTreeSet<PathBuf> = self
                .files
                .borrow()
                .keys()
                .filter_map(|file| {
                    let mut beneath = file.strip_prefix(directory).ok()?.components();
                    let first = beneath.next()?;
                    // A file directly in `directory` names no subdirectory.
                    beneath.next()?;
                    Some(directory.join(first))
                })
                .collect();

            children.into_iter().collect()
        }

        /// Moves every file beneath `from` to the same relative path beneath `to`.
        ///
        /// Fails with `NotFound` when nothing lies beneath `from`, and with `AlreadyExists`
        /// when anything lies at or beneath `to`. The trait's missing-parent error is vacuous
        /// here, exactly as `rename`'s is, and is pinned on `SystemFileSpace`.
        fn move_dir(&self, from: &Path, to: &Path) -> Result<()> {
            let mut files = self.files.borrow_mut();
            let moving: Vec<PathBuf> = files
                .keys()
                .filter(|file| lies_strictly_beneath(file, from))
                .cloned()
                .collect();
            if moving.is_empty() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("no such directory in space: {}", from.display()),
                )
                .into());
            }
            if files.keys().any(|file| file.starts_with(to)) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::AlreadyExists,
                    format!("destination occupied in space: {}", to.display()),
                )
                .into());
            }

            for file in moving {
                let contents = files.remove(&file).flatten();
                let relative = file
                    .strip_prefix(from)
                    .expect("every moving path was selected by `starts_with(from)`");
                files.insert(to.join(relative), contents);
            }

            Ok(())
        }

        fn write(&self, path: &Path, contents: &str) -> Result<()> {
            // Parent creation is meaningless in a flat map, so the "creates parents" half of
            // the contract is vacuously satisfied here and pinned on `SystemFileSpace`.
            self.files
                .borrow_mut()
                .insert(path.to_path_buf(), Some(contents.as_bytes().to_vec()));

            Ok(())
        }

        fn append(&self, path: &Path, contents: &str) -> Result<()> {
            if self.refused_appends.borrow().contains(path) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    format!("appends refused in space: {}", path.display()),
                )
                .into());
            }

            // Parent creation is vacuous here exactly as it is in `write` above.
            let mut files = self.files.borrow_mut();
            let entry = files.entry(path.to_path_buf()).or_default();
            match entry {
                Some(existing) => existing.extend_from_slice(contents.as_bytes()),
                // A recorded-but-contentless file means "unreadable", so an appender has
                // nothing to preserve and starts from empty rather than failing.
                None => *entry = Some(contents.as_bytes().to_vec()),
            }

            Ok(())
        }

        fn temp_dir(&self) -> PathBuf {
            PathBuf::from(IN_MEMORY_TEMP_ROOT)
        }
    }

    /// Whether the recorded `file` lies strictly beneath `directory`, component-wise.
    ///
    /// `file == directory` is excluded: a recorded file is never its own directory.
    fn lies_strictly_beneath(file: &Path, directory: &Path) -> bool {
        file != directory && file.starts_with(directory)
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
    fn in_memory_space_opaque_copy_preserves_binary_bytes_and_replaces_the_destination() {
        let space = InMemoryFileSpace::new();
        let seed = PathBuf::from("Data").join("xPrevisPatch.esp");
        let plugin = PathBuf::from("Data").join("MyMod.esp");
        let plugin_bytes = vec![0x00, 0xFF, 0x80, b'E', b'S', b'P'];
        space.add_file_with_bytes(&seed, plugin_bytes.clone());
        space.add_file_with_contents(&plugin, "stale");

        space.copy(&seed, &plugin).unwrap();

        assert_eq!(space.contents(&plugin), Some(plugin_bytes));
        assert!(space.is_file(&seed));
    }

    #[test]
    fn in_memory_space_opaque_copy_preserves_a_contentless_file() {
        let space = InMemoryFileSpace::new();
        let seed = PathBuf::from("Data").join("xPrevisPatch.esp");
        let plugin = PathBuf::from("Data").join("MyMod.esp");
        space.add_file(&seed);
        space.add_file_with_contents(&plugin, "stale");

        space.copy(&seed, &plugin).unwrap();

        assert!(space.is_file(&plugin));
        assert!(space.read_lossy(&plugin).is_err());
    }

    #[test]
    fn in_memory_space_opaque_copy_of_a_missing_source_errors() {
        let space = InMemoryFileSpace::new();

        assert!(matches!(
            space
                .copy(Path::new("absent.esp"), Path::new("MyMod.esp"))
                .unwrap_err(),
            crate::error::Error::Io(_)
        ));
        assert!(!space.is_file(Path::new("MyMod.esp")));
    }

    #[test]
    fn in_memory_space_decodes_bytes_only_for_a_lossy_text_read() {
        let space = InMemoryFileSpace::new();
        let log = PathBuf::from("Data").join("CK.log");
        space.add_file_with_bytes(&log, [0xFF, b'D', b'E', b'F', b'A', b'U', b'L', b'T']);

        assert_eq!(
            space.contents(&log),
            Some(vec![0xFF, b'D', b'E', b'F', b'A', b'U', b'L', b'T'])
        );
        assert_eq!(space.read_lossy(&log).unwrap(), "�DEFAULT");
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
    fn system_space_opaque_copy_creates_parents_preserves_bytes_and_replaces_destination() {
        let dir = tempdir().unwrap();
        let space = SystemFileSpace;
        let seed = dir.path().join("xPrevisPatch.esp");
        let plugin = dir.path().join("nested").join("MyMod.esp");
        let first_bytes = [0x00, 0xFF, 0x80, b'E', b'S', b'P'];
        fs::write(&seed, first_bytes).unwrap();

        space.copy(&seed, &plugin).unwrap();

        assert_eq!(fs::read(&plugin).unwrap(), first_bytes);

        let replacement_bytes = [0xFE, b'N', b'E', b'W'];
        fs::write(&seed, replacement_bytes).unwrap();
        space.copy(&seed, &plugin).unwrap();

        assert_eq!(fs::read(&plugin).unwrap(), replacement_bytes);
    }

    #[test]
    fn system_space_opaque_copy_propagates_a_missing_source_error() {
        let dir = tempdir().unwrap();
        let space = SystemFileSpace;
        let plugin = dir.path().join("nested").join("MyMod.esp");

        assert!(matches!(
            space
                .copy(&dir.path().join("absent.esp"), &plugin)
                .unwrap_err(),
            crate::error::Error::Io(_)
        ));
        assert!(!plugin.is_file());
    }

    #[test]
    fn system_space_rename_of_a_missing_source_errors() {
        let dir = tempdir().unwrap();
        let space = SystemFileSpace;

        assert!(
            space
                .rename(
                    &dir.path().join("absent.dll"),
                    &dir.path().join("moved.dll")
                )
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

    /// A tree with siblings at several depths and an empty folder: the shape that
    /// `std::fs::remove_dir_all` fails on under MO2, which the walk exists to handle.
    #[test]
    fn system_space_remove_dir_all_walks_a_nested_tree_away() {
        let dir = tempdir().unwrap();
        let space = SystemFileSpace;
        let work = dir.path().join("ArchiveWork");
        let staging = work.join("staging");
        fs::create_dir_all(staging.join("meshes").join("precombined")).unwrap();
        fs::create_dir_all(staging.join("vis")).unwrap();
        fs::create_dir_all(work.join("empty")).unwrap();
        fs::write(
            staging.join("meshes").join("precombined").join("a.nif"),
            b"nif",
        )
        .unwrap();
        fs::write(staging.join("vis").join("cell.uvd"), b"uvd").unwrap();
        fs::write(work.join("restore.txt"), b"source\n").unwrap();

        space.remove_dir_all(&work).unwrap();

        assert!(!work.exists());
        assert!(fs::read_dir(&work).is_err());
        assert!(dir.path().is_dir());
    }

    /// Pointed at a file, the walk cannot list it, the path is still there afterwards, and so
    /// the call fails, naming the path, rather than reporting a removal that did not happen.
    #[test]
    fn system_space_remove_dir_all_of_a_file_errors_and_leaves_it() {
        let dir = tempdir().unwrap();
        let space = SystemFileSpace;
        let file = dir.path().join("Plugin - Main.ba2");
        fs::write(&file, b"ba2").unwrap();

        let error = space.remove_dir_all(&file).unwrap_err();

        assert!(matches!(error, crate::error::Error::Io(_)));
        assert!(error.to_string().contains("Plugin - Main.ba2"));
        assert!(file.is_file());
    }

    #[test]
    fn system_space_create_dir_all_creates_a_nested_path_and_tolerates_an_existing_one() {
        let dir = tempdir().unwrap();
        let space = SystemFileSpace;
        let nested = dir
            .path()
            .join("ArchiveWork")
            .join("staging")
            .join("meshes");

        space.create_dir_all(&nested).unwrap();
        assert!(nested.is_dir());

        space.create_dir_all(&nested).unwrap();
        assert!(nested.is_dir());
    }

    #[test]
    fn system_space_is_dir_is_true_only_for_a_directory() {
        let dir = tempdir().unwrap();
        let space = SystemFileSpace;
        let empty = dir.path().join("ArchiveWork");
        let file = dir.path().join("restore.txt");
        fs::create_dir_all(&empty).unwrap();
        fs::write(&file, b"source\n").unwrap();

        // An empty folder is a directory; the in-memory adapter cannot say so, so it is
        // pinned here.
        assert!(space.is_dir(&empty));
        assert!(!space.is_dir(&file));
        assert!(!space.is_dir(&dir.path().join("absent")));
    }

    #[test]
    fn system_space_child_dirs_lists_immediate_subdirectories_only() {
        let dir = tempdir().unwrap();
        let space = SystemFileSpace;
        let work = dir.path().join("ArchiveWork");
        let numbered = dir.path().join("ArchiveWork.3");
        fs::create_dir_all(work.join("staging")).unwrap();
        fs::create_dir_all(&numbered).unwrap();
        fs::write(dir.path().join("Fallout4.exe"), b"exe").unwrap();

        let mut children = space.child_dirs(dir.path());
        children.sort();

        assert_eq!(children, vec![work, numbered]);
    }

    #[test]
    fn system_space_child_dirs_of_a_missing_directory_is_empty() {
        let dir = tempdir().unwrap();
        let space = SystemFileSpace;

        assert_eq!(
            space.child_dirs(&dir.path().join("absent")),
            Vec::<PathBuf>::new()
        );
    }

    #[test]
    fn system_space_move_dir_moves_a_nested_tree_with_bytes_intact() {
        let dir = tempdir().unwrap();
        let space = SystemFileSpace;
        let from = dir.path().join("Data").join("meshes").join("precombined");
        let to = dir.path().join("ArchiveWork").join("precombined");
        let mesh_bytes = [0x00, 0xFF, b'N', b'I', b'F'];
        fs::create_dir_all(from.join("cell")).unwrap();
        fs::write(from.join("cell").join("a.nif"), mesh_bytes).unwrap();
        fs::write(from.join("b.nif"), b"b").unwrap();
        fs::create_dir_all(to.parent().unwrap()).unwrap();

        space.move_dir(&from, &to).unwrap();

        assert!(!from.exists());
        assert_eq!(fs::read(to.join("cell").join("a.nif")).unwrap(), mesh_bytes);
        assert_eq!(fs::read(to.join("b.nif")).unwrap(), b"b");
    }

    #[test]
    fn system_space_move_dir_fails_when_the_destination_exists() {
        let dir = tempdir().unwrap();
        let space = SystemFileSpace;
        let from = dir.path().join("vis");
        let to = dir.path().join("ArchiveWork").join("vis");
        fs::create_dir_all(&from).unwrap();
        fs::write(from.join("cell.uvd"), b"new").unwrap();
        fs::create_dir_all(&to).unwrap();
        fs::write(to.join("cell.uvd"), b"old").unwrap();

        assert!(matches!(
            space.move_dir(&from, &to).unwrap_err(),
            crate::error::Error::Io(error) if error.kind() == std::io::ErrorKind::AlreadyExists
        ));
        assert_eq!(fs::read(from.join("cell.uvd")).unwrap(), b"new");
        assert_eq!(fs::read(to.join("cell.uvd")).unwrap(), b"old");
    }

    #[test]
    fn system_space_move_dir_fails_when_the_source_is_missing() {
        let dir = tempdir().unwrap();
        let space = SystemFileSpace;
        let to = dir.path().join("moved");

        assert!(matches!(
            space.move_dir(&dir.path().join("absent"), &to).unwrap_err(),
            crate::error::Error::Io(error) if error.kind() == std::io::ErrorKind::NotFound
        ));
        assert!(!to.exists());
    }

    #[test]
    fn system_space_move_dir_does_not_create_the_destination_parent() {
        let dir = tempdir().unwrap();
        let space = SystemFileSpace;
        let from = dir.path().join("vis");
        fs::create_dir_all(&from).unwrap();
        fs::write(from.join("cell.uvd"), b"uvd").unwrap();
        let to = dir.path().join("absent").join("vis");

        assert!(space.move_dir(&from, &to).is_err());
        assert!(from.join("cell.uvd").is_file());
        assert!(!to.parent().unwrap().exists());
    }

    #[test]
    fn in_memory_space_is_dir_is_answered_by_a_file_beneath_the_path() {
        let space = InMemoryFileSpace::new();
        let work = PathBuf::from("Fallout4").join("ArchiveWork");
        let mesh = work.join("staging").join("meshes").join("a.nif");
        space.add_file(&mesh);

        assert!(space.is_dir(&work));
        assert!(space.is_dir(&work.join("staging")));
        // The file itself is not a directory, and neither is an unrelated path.
        assert!(!space.is_dir(&mesh));
        assert!(!space.is_dir(&PathBuf::from("Fallout4").join("Data")));
    }

    /// `create_dir_all` succeeds and records nothing: an empty directory cannot be represented
    /// in the flat map, so `is_dir` stays false until a file lies beneath it.
    #[test]
    fn in_memory_space_create_dir_all_is_vacuous() {
        let space = InMemoryFileSpace::new();
        let work = PathBuf::from("Fallout4").join("ArchiveWork");

        space.create_dir_all(&work).unwrap();

        assert!(!space.is_dir(&work));
    }

    #[test]
    fn in_memory_space_child_dirs_derives_immediate_subdirectories_from_nested_files() {
        let space = InMemoryFileSpace::new();
        let fo4 = PathBuf::from("Fallout4");
        space.add_file(fo4.join("Fallout4.exe"));
        space.add_file(fo4.join("ArchiveWork").join("restore.txt"));
        space.add_file(fo4.join("ArchiveWork").join("staging").join("a.nif"));
        space.add_file(fo4.join("ArchiveWork.3").join("Plugin - Main.ba2"));

        let mut children = space.child_dirs(&fo4);
        children.sort();

        assert_eq!(
            children,
            vec![fo4.join("ArchiveWork"), fo4.join("ArchiveWork.3")]
        );
        assert_eq!(space.child_dirs(&fo4.join("absent")), Vec::<PathBuf>::new());
    }

    #[test]
    fn in_memory_space_move_dir_moves_every_file_beneath_the_source() {
        let space = InMemoryFileSpace::new();
        let from = PathBuf::from("Data").join("meshes").join("precombined");
        let to = PathBuf::from("ArchiveWork").join("precombined");
        let mesh_bytes = vec![0x00, 0xFF, b'N', b'I', b'F'];
        space.add_file_with_bytes(from.join("cell").join("a.nif"), mesh_bytes.clone());
        space.add_file(from.join("b.nif"));
        let sibling = PathBuf::from("Data").join("meshes").join("other.nif");
        space.add_file(&sibling);

        space.move_dir(&from, &to).unwrap();

        assert!(!space.is_dir(&from));
        assert_eq!(
            space.contents(&to.join("cell").join("a.nif")),
            Some(mesh_bytes)
        );
        assert!(space.is_file(&to.join("b.nif")));
        assert!(space.is_file(&sibling));
    }

    #[test]
    fn in_memory_space_move_dir_fails_when_the_destination_exists() {
        let space = InMemoryFileSpace::new();
        let from = PathBuf::from("vis");
        let to = PathBuf::from("ArchiveWork").join("vis");
        space.add_file_with_contents(from.join("cell.uvd"), "new");
        space.add_file_with_contents(to.join("cell.uvd"), "old");

        assert!(matches!(
            space.move_dir(&from, &to).unwrap_err(),
            crate::error::Error::Io(error) if error.kind() == std::io::ErrorKind::AlreadyExists
        ));
        assert_eq!(space.read_lossy(&from.join("cell.uvd")).unwrap(), "new");
        assert_eq!(space.read_lossy(&to.join("cell.uvd")).unwrap(), "old");
    }

    /// A *file* at `to` occupies it as surely as a directory does.
    #[test]
    fn in_memory_space_move_dir_fails_when_a_file_sits_at_the_destination() {
        let space = InMemoryFileSpace::new();
        let from = PathBuf::from("vis");
        let to = PathBuf::from("ArchiveWork");
        space.add_file(from.join("cell.uvd"));
        space.add_file(&to);

        assert!(matches!(
            space.move_dir(&from, &to).unwrap_err(),
            crate::error::Error::Io(error) if error.kind() == std::io::ErrorKind::AlreadyExists
        ));
        assert!(space.is_file(&from.join("cell.uvd")));
    }

    #[test]
    fn in_memory_space_move_dir_fails_when_the_source_is_missing() {
        let space = InMemoryFileSpace::new();
        let to = PathBuf::from("moved");

        assert!(matches!(
            space.move_dir(Path::new("absent"), &to).unwrap_err(),
            crate::error::Error::Io(error) if error.kind() == std::io::ErrorKind::NotFound
        ));
        assert!(!space.is_dir(&to));
    }
}
