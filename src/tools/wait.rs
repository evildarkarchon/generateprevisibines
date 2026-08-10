//! The `Wait` seam: the MO2 virtual-filesystem sync delays.
//!
//! An internal seam, private to the tools layer, holding the delays that were `src/timing.rs`.
//!
//! The delays are required, not incidental — see `docs/workarounds.md` §2. That is exactly why
//! they sit behind a port rather than behind a `#[cfg(test)]` constant of zero: the batch has
//! nine distinct wait sites (10s after every Creation Kit exit, ~50s of fixed delay per xEdit
//! run), so a suite that really sleeps is not viable, while a suite that compiles the delays
//! away would assert a production behaviour that is not there. Recording them keeps both the
//! delay and the proof of it.

/// Delay after Creation Kit exits so MO2 can sync files to disk.
pub(crate) const MO2_DELAY_AFTER_CK_SECS: u64 = 10;

/// Delay after copying seed plugin when the file is not immediately visible.
pub(crate) const MO2_DELAY_AFTER_SEED_COPY_SECS: u64 = 5;

/// Pause so the MO2 virtual filesystem can settle.
///
/// Implementors must be `Debug` so the adapters that hold a `Wait` can keep deriving `Debug`.
pub(crate) trait Wait: std::fmt::Debug {
    /// Sleep for `seconds` to let the MO2 virtual filesystem settle.
    ///
    /// Required, not incidental — see docs/workarounds.md §2. Callers must not skip it.
    fn sync_delay(&self, seconds: u64);
}

/// The production `Wait`, backed by [`std::thread::sleep`] (batch `timeout /t`).
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct SystemWait;

impl Wait for SystemWait {
    fn sync_delay(&self, seconds: u64) {
        std::thread::sleep(std::time::Duration::from_secs(seconds));
    }
}

#[cfg(test)]
pub(crate) use recording::RecordingWait;

#[cfg(test)]
mod recording {
    use std::cell::RefCell;

    use super::Wait;

    /// A test [`Wait`] that records every requested delay and sleeps for none of them.
    ///
    /// Interior mutability, like the other recording adapters: the caller under test holds
    /// the wait by shared reference across the whole episode.
    #[derive(Debug, Default)]
    pub(crate) struct RecordingWait {
        delays: RefCell<Vec<u64>>,
    }

    impl RecordingWait {
        /// A wait that has recorded nothing yet.
        #[must_use]
        pub(crate) fn new() -> Self {
            Self::default()
        }

        /// Every delay requested so far, in seconds, in call order.
        ///
        /// Order matters as much as the values: what the workarounds mandate is a delay
        /// *between* two particular steps, not a delay somewhere in the episode.
        #[must_use]
        pub(crate) fn delays(&self) -> Vec<u64> {
            self.delays.borrow().clone()
        }
    }

    impl Wait for RecordingWait {
        fn sync_delay(&self, seconds: u64) {
            self.delays.borrow_mut().push(seconds);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::{
        MO2_DELAY_AFTER_CK_SECS, MO2_DELAY_AFTER_SEED_COPY_SECS, RecordingWait, SystemWait, Wait,
    };

    #[test]
    fn mandated_delays_keep_their_batch_durations() {
        assert_eq!(MO2_DELAY_AFTER_CK_SECS, 10);
        assert_eq!(MO2_DELAY_AFTER_SEED_COPY_SECS, 5);
    }

    #[test]
    fn recording_wait_records_every_delay_in_order_without_sleeping() {
        let wait = RecordingWait::new();
        let started = Instant::now();

        wait.sync_delay(MO2_DELAY_AFTER_CK_SECS);
        wait.sync_delay(MO2_DELAY_AFTER_SEED_COPY_SECS);
        wait.sync_delay(MO2_DELAY_AFTER_CK_SECS);

        assert_eq!(wait.delays(), vec![10, 5, 10]);
        // 25s of mandated delay, and the test is still instant.
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn system_wait_actually_sleeps() {
        // One second rather than the mandated ten: this pins that `SystemWait` really blocks,
        // which is the whole point of keeping the recording double separate from it.
        let started = Instant::now();
        SystemWait.sync_delay(1);

        assert!(started.elapsed() >= Duration::from_secs(1));
    }
}
