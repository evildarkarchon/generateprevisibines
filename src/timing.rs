//! MO2 virtual-filesystem sync delays (see `docs/workarounds.md`).

/// Delay after Creation Kit exits so MO2 can sync files to disk.
pub const MO2_DELAY_AFTER_CK_SECS: u64 = 10;

/// Delay after copying seed plugin when the file is not immediately visible.
pub const MO2_DELAY_AFTER_SEED_COPY_SECS: u64 = 5;

/// Sleep for MO2 VFS sync (batch `timeout /t`).
pub fn mo2_sync_delay(seconds: u64) {
    std::thread::sleep(std::time::Duration::from_secs(seconds));
}
