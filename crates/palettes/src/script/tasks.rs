//! Work a script starts off the UI thread: commands and timers. Each reports
//! through the shell's channel and stops when its script unloads.

use palettes_shell::Sender;
use std::{
    io::{BufRead, BufReader, Read},
    os::unix::process::CommandExt,
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::Thread,
    time::{Duration, Instant},
};

/// What a task reports.
pub(crate) enum Event {
    /// A command finished: its standard output (empty when read by line)
    /// and exit status.
    Output(String, i32),
    /// A command wrote a line.
    Line(String),
    /// A timer fired.
    Tick,
}

/// A report for the callback `callback` of script `script`.
pub(crate) struct Message {
    pub script: u64,
    pub callback: u64,
    pub event: Event,
}

/// A running command or timer; dropping it stops it.
pub(crate) enum Task {
    /// The command's process group and whether it has exited.
    Process(i32, Arc<AtomicBool>),
    Timer(Arc<AtomicBool>, Thread),
}

impl Task {
    /// Whether the task can still report.
    pub fn running(&self) -> bool {
        match self {
            Task::Process(_, stopped) | Task::Timer(stopped, _) => !stopped.load(Ordering::Relaxed),
        }
    }
}

impl Drop for Task {
    fn drop(&mut self) {
        match self {
            Task::Process(group, exited) => {
                // The command runs in its own group: end the whole pipeline.
                if !exited.load(Ordering::Relaxed)
                    && let Some(group) = rustix::process::Pid::from_raw(*group)
                {
                    let _ =
                        rustix::process::kill_process_group(group, rustix::process::Signal::TERM);
                }
            }
            Task::Timer(stopped, thread) => {
                stopped.store(true, Ordering::Relaxed);
                thread.unpark();
            }
        }
    }
}

/// Output kept from one command, so a runaway one cannot exhaust memory.
const MAX_OUTPUT: u64 = 16 << 20;

/// Runs `command` through `sh`; reports each line, or the whole output and
/// status once it exits.
pub(crate) fn command(
    command: &str,
    lines: bool,
    sender: Sender<Message>,
    script: u64,
    callback: u64,
) -> std::io::Result<Task> {
    let mut child = Command::new("sh")
        .args(["-c", command])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .process_group(0)
        .spawn()?;
    let stdout = child.stdout.take().expect("piped");
    let group = child.id() as i32;
    let exited = Arc::new(AtomicBool::new(false));
    let done = exited.clone();
    let report = move |event| {
        sender.send(Message {
            script,
            callback,
            event,
        })
    };
    std::thread::Builder::new()
        .name("palettes-command".into())
        .spawn(move || {
            let mut text = String::new();
            if lines {
                for line in BufReader::new(stdout).lines() {
                    match line {
                        Ok(line) => report(Event::Line(line)),
                        Err(_) => break,
                    }
                }
            } else {
                let mut bytes = Vec::new();
                let _ = stdout.take(MAX_OUTPUT).read_to_end(&mut bytes);
                text = String::from_utf8_lossy(&bytes).into_owned();
            }
            let status = child.wait().ok().and_then(|s| s.code()).unwrap_or(-1);
            done.store(true, Ordering::Relaxed);
            report(Event::Output(text, status));
        })?;
    Ok(Task::Process(group, exited))
}

/// Fires after `period`, and again every `period` if `repeat`.
pub(crate) fn timer(
    period: Duration,
    repeat: bool,
    sender: Sender<Message>,
    script: u64,
    callback: u64,
) -> std::io::Result<Task> {
    let stopped = Arc::new(AtomicBool::new(false));
    let flag = stopped.clone();
    let handle = std::thread::Builder::new()
        .name("palettes-timer".into())
        .spawn(move || {
            let mut due = Instant::now() + period;
            loop {
                // Parking may end early; wait out the rest.
                while !flag.load(Ordering::Relaxed) && Instant::now() < due {
                    std::thread::park_timeout(due - Instant::now());
                }
                if flag.load(Ordering::Relaxed) {
                    return;
                }
                // A one-shot timer reads as stopped by the time it is handled.
                if !repeat {
                    flag.store(true, Ordering::Relaxed);
                }
                sender.send(Message {
                    script,
                    callback,
                    event: Event::Tick,
                });
                if !repeat {
                    return;
                }
                due += period;
            }
        })?;
    Ok(Task::Timer(stopped, handle.thread().clone()))
}
