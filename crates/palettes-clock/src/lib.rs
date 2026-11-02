//! Local wall-clock time for shells.
//!
//! [`format`] renders a `strftime` pattern in the local time zone and locale.
//! [`Period::of`] tells whether a pattern shows seconds, and [`Ticker`] wakes on
//! each whole second or minute of the realtime clock: a thread blocked in a
//! `timerfd`, so a clock costs nothing between ticks and catches up at once
//! after a suspend or a clock change.
#![deny(unsafe_op_in_unsafe_fn)]

mod ticker;
mod time;

pub use ticker::Ticker;
pub use time::{LocalTime, format};

/// How often a formatted time can change.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Period {
    /// Every second.
    Second,
    /// Every minute.
    Minute,
}

impl Period {
    /// The finest unit `pattern` shows. Conversions that include seconds
    /// (`%S %T %s %r %X %c %+`) tick every second; everything else every minute.
    pub fn of(pattern: &str) -> Self {
        let mut chars = pattern.chars();
        while let Some(c) = chars.next() {
            if c != '%' {
                continue;
            }
            // Skip glibc flags and modifiers: `%-S`, `%OS`, `%ES`.
            let spec = chars.find(|c| !matches!(c, '-' | '_' | '0' | '^' | '#' | 'E' | 'O'));
            if matches!(spec, Some('S' | 'T' | 's' | 'r' | 'X' | 'c' | '+')) {
                return Self::Second;
            }
        }
        Self::Minute
    }

    /// Length in seconds.
    pub fn seconds(self) -> u64 {
        match self {
            Self::Second => 1,
            Self::Minute => 60,
        }
    }
}
