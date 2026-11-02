//! A thread that sleeps on a realtime `timerfd` until the next whole period.
#![allow(unsafe_code)]

use crate::Period;
use std::{
    io,
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::SystemTime,
};

/// Calls a function at every whole second or minute of wall-clock time, and
/// once at once. Dropping it stops the thread at its next tick.
pub struct Ticker {
    stop: Arc<AtomicBool>,
}

impl Ticker {
    /// Starts ticking every `period`; `tick` runs on the ticker's thread.
    pub fn spawn(
        period: Period,
        mut tick: impl FnMut(SystemTime) + Send + 'static,
    ) -> io::Result<Self> {
        // SAFETY: plain syscall; the result is checked.
        let fd = unsafe { libc::timerfd_create(libc::CLOCK_REALTIME, libc::TFD_CLOEXEC) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: timerfd_create returned a new descriptor nobody else owns.
        let fd = unsafe { OwnedFd::from_raw_fd(fd) };
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        std::thread::Builder::new()
            .name("palettes-clock".into())
            .spawn(move || {
                let mut now = SystemTime::now();
                while !stopped.load(Ordering::Relaxed) {
                    tick(now);
                    if arm(&fd, period, now).and_then(|()| wait(&fd)).is_err() {
                        return;
                    }
                    now = SystemTime::now();
                }
            })?;
        Ok(Self { stop })
    }
}

impl Drop for Ticker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// Arms an absolute expiry at the next period boundary after `now`.
/// `CANCEL_ON_SET` makes a clock change end the wait early, which ticks again.
fn arm(fd: &OwnedFd, period: Period, now: SystemTime) -> io::Result<()> {
    let seconds = now
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let step = period.seconds();
    // Minutes align to local minutes too: every zone offset is whole minutes.
    let next = (seconds / step + 1) * step;
    let spec = libc::itimerspec {
        it_interval: libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        },
        it_value: libc::timespec {
            tv_sec: next as libc::time_t,
            tv_nsec: 0,
        },
    };
    let flags = libc::TFD_TIMER_ABSTIME | libc::TFD_TIMER_CANCEL_ON_SET;
    // SAFETY: fd is a live timerfd; spec is valid; the old value is not wanted.
    let set = unsafe { libc::timerfd_settime(fd.as_raw_fd(), flags, &spec, std::ptr::null_mut()) };
    if set < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Blocks until expiry or a clock change.
fn wait(fd: &OwnedFd) -> io::Result<()> {
    let mut expirations = 0u64;
    loop {
        // SAFETY: reads 8 bytes into a u64 that outlives the call.
        let read = unsafe { libc::read(fd.as_raw_fd(), (&raw mut expirations).cast(), 8) };
        if read == 8 {
            return Ok(());
        }
        let error = io::Error::last_os_error();
        match error.raw_os_error() {
            Some(libc::EINTR) => continue,
            // The clock was set: the caller re-arms from the new time.
            Some(libc::ECANCELED) => return Ok(()),
            _ => return Err(error),
        }
    }
}
