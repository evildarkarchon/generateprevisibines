//! The per-installation lock that keeps one Workflow Run per Fallout 4 installation (ADR-0005).
//!
//! Several rules read what a run finds at start as the leftovers of a run that has ended: the
//! archive run-start restore and `DllGuard`'s orphan adoption both do. Holding this lock for the
//! whole process makes that true, because a second invocation against the same installation
//! stops before intake instead of treating the first run's live work as crash leftovers.
//!
//! WHY this does not go through `FileSpace`: the lock is a live OS handle whose lifetime is the
//! process's, released when the handle closes or the process dies. The in-memory adapter has no
//! handles and no processes, so it cannot model it, and a fake that pretended to would test
//! nothing. The tests here use a real directory instead.

use std::fs::{File, OpenOptions, TryLockError};
use std::io;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// The lock file's name, directly under the Fallout 4 directory.
const LOCK_FILE_NAME: &str = "GeneratePrevisibines.lock";

/// An exclusive lock on one Fallout 4 installation, held for as long as this value lives.
///
/// There is no `Drop` impl: closing the `File` inside releases the lock, and the OS also
/// releases it when the process exits or crashes. The lock file itself is never deleted.
#[derive(Debug)]
pub struct InstallationLock {
    // Never read: the open handle is the lock, so holding the field is the whole point.
    _file: File,
}

impl InstallationLock {
    /// Take the installation lock on `<fallout4_dir>\GeneratePrevisibines.lock`, without waiting.
    ///
    /// The file is created if it is missing, and is never truncated, read or deleted: a lock file
    /// left behind by a crashed run is just a file, and this locks it normally. The lock lasts
    /// until the returned value is dropped or the process exits, so callers bind it to a named
    /// local for as long as the run needs it.
    ///
    /// # Errors
    ///
    /// [`Error::InstallationInUse`] when another process holds the lock. An [`Error::Io`] naming
    /// the lock path when the file cannot be opened (a missing Fallout 4 directory included) or
    /// the lock call itself fails. Neither waits nor retries.
    pub fn acquire(fallout4_dir: &Path) -> Result<Self> {
        let lock_path = lock_path(fallout4_dir);
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .map_err(|err| lock_io_error(&lock_path, "open", &err))?;

        match file.try_lock() {
            Ok(()) => Ok(Self { _file: file }),
            Err(TryLockError::WouldBlock) => Err(Error::InstallationInUse {
                fallout4_dir: fallout4_dir.to_path_buf(),
                lock_path,
            }),
            Err(TryLockError::Error(err)) => Err(lock_io_error(&lock_path, "lock", &err)),
        }
    }
}

/// Where an installation's lock file lives: directly under the Fallout 4 directory.
fn lock_path(fallout4_dir: &Path) -> PathBuf {
    fallout4_dir.join(LOCK_FILE_NAME)
}

/// Wrap an I/O failure on the lock file so its message names the lock path, keeping the kind.
fn lock_io_error(lock_path: &Path, action: &str, err: &io::Error) -> Error {
    io::Error::new(
        err.kind(),
        format!(
            "could not {action} the installation lock {}: {err}",
            lock_path.display()
        ),
    )
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acquiring_on_an_empty_directory_creates_the_lock_file() {
        let dir = tempfile::tempdir().unwrap();

        let _lock = InstallationLock::acquire(dir.path()).unwrap();

        assert!(lock_path(dir.path()).is_file());
    }

    #[test]
    fn a_second_acquire_while_the_first_lives_is_in_use_naming_both_paths() {
        let dir = tempfile::tempdir().unwrap();
        let _first = InstallationLock::acquire(dir.path()).unwrap();

        let err = InstallationLock::acquire(dir.path()).unwrap_err();

        match &err {
            Error::InstallationInUse {
                fallout4_dir,
                lock_path: held,
            } => {
                assert_eq!(fallout4_dir, dir.path());
                assert_eq!(held, &lock_path(dir.path()));
            }
            other => panic!("expected InstallationInUse, got {other:?}"),
        }
        assert_eq!(
            err.to_string(),
            format!(
                "Another GeneratePrevisibines run is using {} (it holds {}). Wait for it to \
                 finish, then rerun.",
                dir.path().display(),
                lock_path(dir.path()).display()
            )
        );
    }

    #[test]
    fn dropping_the_first_lock_lets_a_second_acquire_succeed() {
        let dir = tempfile::tempdir().unwrap();
        let first = InstallationLock::acquire(dir.path()).unwrap();
        drop(first);

        assert!(InstallationLock::acquire(dir.path()).is_ok());
    }

    #[test]
    fn a_leftover_lock_file_does_not_block_and_keeps_its_content() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(lock_path(dir.path()), "left by a crashed run").unwrap();

        let lock = InstallationLock::acquire(dir.path()).unwrap();
        // Released before reading: on Windows `LockFileEx` locks are mandatory, so even this
        // process cannot read the locked bytes through a second handle.
        drop(lock);

        assert_eq!(
            std::fs::read_to_string(lock_path(dir.path())).unwrap(),
            "left by a crashed run"
        );
    }

    #[test]
    fn a_missing_fallout4_directory_is_an_io_error_naming_the_lock_path() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("Fallout 4");

        let err = InstallationLock::acquire(&missing).unwrap_err();

        assert!(matches!(err, Error::Io(_)), "expected Io, got {err:?}");
        assert!(
            err.to_string()
                .contains(&lock_path(&missing).display().to_string()),
            "{err}"
        );
    }
}
