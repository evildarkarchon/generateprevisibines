//! The `Clock` seam: the wall-clock readings the session log records around a tool run.
//!
//! An internal seam, private to the tools layer, for the same reason `Wait` is one (ADR-0002):
//! what time of day a Creation Kit run started is not build meaning, it is part of how the
//! episode writes its record. A Workflow Operation never names it.
//!
//! It exists at all because the batch's `:RunCK` brackets every run with `Start %time%` and
//! `Ended %time%` (lines 454 and 457), and a session log whose contents come from
//! `SystemTime::now()` is a session log no test can assert on.

/// The local wall-clock time of day, as the batch's `%time%` reports it.
///
/// Implementors must be `Debug` so the adapters that hold a `Clock` can keep deriving `Debug`.
pub(crate) trait Clock: std::fmt::Debug {
    /// The current local time of day, formatted `HH:MM:SS.hh`.
    ///
    /// A preformatted string rather than an instant or a duration: the only reader is the
    /// session log, which reproduces a batch `echo`, and `%time%` is a time of *day* — not a
    /// duration, and not a date. Handing back a timestamp type would put the batch's format
    /// somewhere further from the one line that depends on it.
    fn time_of_day(&self) -> String;
}

/// The production `Clock`, reading the machine's local time (batch `%time%`).
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct SystemClock;

impl Clock for SystemClock {
    fn time_of_day(&self) -> String {
        use chrono::Timelike;

        let now = chrono::Local::now();
        // Assembled field by field rather than through a `strftime` pattern: chrono's
        // fractional-second specifiers only offer 3, 6 and 9 digits, and `%time%` reports
        // hundredths.
        //
        // `cmd`'s `%time%` is also locale-formatted and pads the hour with a space below 10
        // (` 9:05:03.21`). Zero-padding it instead is the one deliberate divergence: the locale
        // quirk is not reproducible without carrying the user's locale settings, and a
        // fixed-width field is what anyone reading two of these lines together wants anyway.
        //
        // `nanosecond` runs past a second during a leap second, which would widen the field to
        // three digits; clamping keeps the shape fixed.
        let hundredths = (now.nanosecond() / 10_000_000).min(99);

        format!(
            "{:02}:{:02}:{:02}.{hundredths:02}",
            now.hour(),
            now.minute(),
            now.second()
        )
    }
}

#[cfg(test)]
pub(crate) use recording::ScriptedClock;

#[cfg(test)]
mod recording {
    use std::cell::RefCell;
    use std::collections::VecDeque;

    use super::Clock;

    /// A test [`Clock`] that hands out scripted readings instead of asking the machine.
    ///
    /// Interior mutability, like the other recording adapters: the caller under test holds the
    /// clock by shared reference across the whole episode.
    #[derive(Debug)]
    pub(crate) struct ScriptedClock {
        readings: RefCell<VecDeque<String>>,
        /// The last reading handed out, repeated once the script runs dry.
        last: RefCell<String>,
    }

    impl ScriptedClock {
        /// A clock that reads `readings` in order, then repeats the final one forever.
        ///
        /// Repeating rather than panicking on exhaustion is what lets a fixture that does not
        /// care about time supply a single reading and stop thinking about it, while a test
        /// that *is* asserting on `Start` and `Ended` supplies two distinct ones and gets them.
        ///
        /// # Panics
        ///
        /// If `readings` is empty — a clock with nothing to say is a mis-scripted test.
        #[must_use]
        pub(crate) fn new(readings: impl IntoIterator<Item = impl Into<String>>) -> Self {
            let readings: VecDeque<String> = readings.into_iter().map(Into::into).collect();
            let last = readings
                .back()
                .expect("a ScriptedClock needs at least one reading")
                .clone();

            Self {
                readings: RefCell::new(readings),
                last: RefCell::new(last),
            }
        }

        /// A clock stuck at one reading, for tests whose subject is not the timestamps.
        #[must_use]
        pub(crate) fn fixed() -> Self {
            Self::new(["00:00:00.00"])
        }
    }

    impl Clock for ScriptedClock {
        fn time_of_day(&self) -> String {
            match self.readings.borrow_mut().pop_front() {
                Some(reading) => {
                    *self.last.borrow_mut() = reading.clone();
                    reading
                }
                None => self.last.borrow().clone(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Clock, ScriptedClock, SystemClock};

    #[test]
    fn scripted_clock_reads_in_order_then_holds_its_final_reading() {
        let clock = ScriptedClock::new(["09:00:00.00", "09:04:12.34"]);

        assert_eq!(clock.time_of_day(), "09:00:00.00");
        assert_eq!(clock.time_of_day(), "09:04:12.34");
        // Held rather than exhausted, so a fixture that outlives its script keeps working.
        assert_eq!(clock.time_of_day(), "09:04:12.34");
    }

    #[test]
    fn a_fixed_clock_never_moves() {
        let clock = ScriptedClock::fixed();

        assert_eq!(clock.time_of_day(), clock.time_of_day());
    }

    /// The production reading has the batch's shape: `HH:MM:SS.hh`, no date.
    #[test]
    fn system_clock_reports_a_time_of_day() {
        let now = SystemClock.time_of_day();

        let parts: Vec<&str> = now.split(':').collect();
        assert_eq!(parts.len(), 3, "unexpected reading: {now}");
        assert_eq!(parts[0].len(), 2, "hour is not zero-padded: {now}");
        assert_eq!(parts[1].len(), 2, "minute is not zero-padded: {now}");
        // `SS.hh`: seconds, a decimal point, and two hundredths digits.
        assert_eq!(parts[2].len(), 5, "unexpected seconds field: {now}");
        assert!(parts[2].contains('.'), "no fractional seconds: {now}");
        assert!(
            now.chars()
                .all(|c| c.is_ascii_digit() || c == ':' || c == '.'),
            "unexpected characters: {now}"
        );
    }
}
