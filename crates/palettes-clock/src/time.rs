//! Local time through the C library, which honours `TZ`, `/etc/localtime` and
//! `LC_TIME` exactly like every other program on the system.
#![allow(unsafe_code)]

use std::{ffi::CString, sync::Once, time::SystemTime};

/// A broken-down local time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalTime {
    /// Calendar year, such as 2026.
    pub year: i32,
    /// Month, 1–12.
    pub month: u8,
    /// Day of the month, 1–31.
    pub day: u8,
    /// Day of the week, 0 (Sunday) – 6.
    pub weekday: u8,
    /// Hour, 0–23.
    pub hour: u8,
    /// Minute, 0–59.
    pub minute: u8,
    /// Second, 0–60.
    pub second: u8,
}

fn epoch_seconds(at: SystemTime) -> libc::time_t {
    match at.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(after) => after.as_secs() as libc::time_t,
        Err(before) => -(before.duration().as_secs_f64().ceil() as libc::time_t),
    }
}

/// `setlocale(LC_TIME, "")` once, so names follow the user's locale.
fn init_locale() {
    static INIT: Once = Once::new();
    // SAFETY: called once, before any formatting this crate does; the argument
    // is a valid NUL-terminated string.
    INIT.call_once(|| unsafe {
        libc::setlocale(libc::LC_TIME, c"".as_ptr());
    });
}

fn broken_down(at: SystemTime) -> libc::tm {
    let seconds = epoch_seconds(at);
    // SAFETY: tm is plain data; zeroed is a valid value.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    // SAFETY: both pointers are valid for the call; localtime_r is reentrant.
    unsafe { libc::localtime_r(&seconds, &mut tm) };
    tm
}

impl LocalTime {
    /// The local time at `at`.
    pub fn at(at: SystemTime) -> Self {
        let tm = broken_down(at);
        Self {
            year: tm.tm_year + 1900,
            month: (tm.tm_mon + 1) as u8,
            day: tm.tm_mday as u8,
            weekday: tm.tm_wday as u8,
            hour: tm.tm_hour as u8,
            minute: tm.tm_min as u8,
            second: tm.tm_sec as u8,
        }
    }

    /// The local time now.
    pub fn now() -> Self {
        Self::at(SystemTime::now())
    }
}

/// Formats `at` with a `strftime` pattern in the local zone and locale. A
/// pattern with NUL yields an empty string.
pub fn format(pattern: &str, at: SystemTime) -> String {
    init_locale();
    let Ok(c_pattern) = CString::new(pattern) else {
        return String::new();
    };
    let tm = broken_down(at);
    let mut buffer = vec![0u8; 64.max(pattern.len() * 4)];
    loop {
        // SAFETY: buffer is writable for its length; pattern and tm are valid.
        let written = unsafe {
            libc::strftime(
                buffer.as_mut_ptr().cast(),
                buffer.len(),
                c_pattern.as_ptr(),
                &tm,
            )
        };
        // Zero is either an empty result or a short buffer; strftime can't
        // tell them apart, so grow a few times before accepting empty.
        if written > 0 || buffer.len() >= 4096 {
            buffer.truncate(written);
            return String::from_utf8_lossy(&buffer).into_owned();
        }
        buffer.resize(buffer.len() * 4, 0);
    }
}
