//! Processor and memory use from `/proc`, sampled every two seconds on a
//! thread of its own; the only service that needs no bus, and the only one
//! that wakes by itself, as the kernel announces neither.

use crate::{Bus, Report, System};
use std::{io, sync::mpsc, time::Duration};

/// How often it samples.
pub const INTERVAL: Duration = Duration::from_secs(2);

/// Processor and memory use.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LoadState {
    /// Processor time spent busy since the last sample, 0–1.
    pub cpu: f64,
    /// Memory in use (total less available), in bytes.
    pub memory_used: u64,
    /// Memory in all, in bytes.
    pub memory_total: u64,
}

/// Busy and total jiffies of the first `cpu` line of `/proc/stat`.
pub fn cpu_times(stat: &str) -> Option<(u64, u64)> {
    let line = stat.lines().find(|l| l.starts_with("cpu "))?;
    let fields: Vec<u64> = (line.split_whitespace().skip(1))
        .filter_map(|f| f.parse().ok())
        .collect();
    // user nice system idle iowait irq softirq steal; guests are in user.
    let total: u64 = fields.iter().take(8).sum();
    let idle = fields.get(3)? + fields.get(4).unwrap_or(&0);
    Some((total - idle, total))
}

/// Used and total bytes from `/proc/meminfo`.
pub fn memory(meminfo: &str) -> Option<(u64, u64)> {
    let field = |name: &str| {
        let line = meminfo.lines().find(|l| l.starts_with(name))?;
        let kib: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
        Some(kib * 1024)
    };
    let (total, available) = (field("MemTotal:")?, field("MemAvailable:")?);
    Some((total.saturating_sub(available), total))
}

fn run(mut changed: Report<LoadState>, stop: mpsc::Receiver<()>) {
    let read = |path| std::fs::read_to_string(path).unwrap_or_default();
    let mut last = cpu_times(&read("/proc/stat"));
    let mut reported = None;
    while let Err(mpsc::RecvTimeoutError::Timeout) = stop.recv_timeout(INTERVAL) {
        let now = cpu_times(&read("/proc/stat"));
        let cpu = match (last, now) {
            (Some((busy0, total0)), Some((busy1, total1))) if total1 > total0 => {
                (busy1.saturating_sub(busy0)) as f64 / (total1 - total0) as f64
            }
            _ => 0.0,
        };
        last = now;
        let (memory_used, memory_total) = memory(&read("/proc/meminfo")).unwrap_or_default();
        let state = LoadState {
            cpu: (cpu * 100.0).round() / 100.0,
            memory_used,
            memory_total,
        };
        if reported.as_ref() != Some(&state) {
            changed(&state);
            reported = Some(state);
        }
    }
}

/// The running load sampler; dropping it ends its thread.
pub struct Load(#[allow(dead_code)] mpsc::Sender<()>);

impl System for Load {
    type State = LoadState;
    /// Unused: it reads `/proc`.
    const BUS: Bus = Bus::Session;

    fn spawn(_: &Bus, changed: Report<LoadState>) -> io::Result<Self> {
        let (stop, stopped) = mpsc::channel();
        std::thread::Builder::new()
            .name("palettes-load".into())
            .spawn(move || run(changed, stopped))?;
        Ok(Self(stop))
    }
}
