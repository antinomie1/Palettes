//! The screen's backlight from sysfs, set through logind so no permission is
//! needed. The kernel announces each change, by any program or a brightness
//! key, on the device's `actual_brightness` attribute.

use crate::{
    Report, System,
    client::{self, Bus, Client, Handle},
};
use aegle_dbus::{Message, Value};
use std::{
    fs::File,
    io::{self, Read, Seek},
    os::fd::{AsFd, BorrowedFd},
    path::{Path, PathBuf},
};

/// Where backlights are listed.
pub const BACKLIGHTS: &str = "/sys/class/backlight";

/// The backlight.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BrightnessState {
    /// Whether there is a backlight.
    pub present: bool,
    /// Its level, 0–1.
    pub level: f64,
}

enum Command {
    Set(f64),
}

struct Service {
    changed: Report<BrightnessState>,
    /// The device's name, its maximum and its `actual_brightness`.
    device: Option<(String, u64, File)>,
    level: u64,
    last: Option<BrightnessState>,
}

/// The backlight to use: firmware ones first, as they work best, then
/// platform, then raw, as systemd chooses.
fn choose(root: &Path) -> Option<PathBuf> {
    let rank = |dir: &Path| match std::fs::read_to_string(dir.join("type")).as_deref() {
        Ok("firmware\n") => 0,
        Ok("platform\n") => 1,
        _ => 2,
    };
    let mut found: Vec<PathBuf> = std::fs::read_dir(root)
        .ok()?
        .filter_map(|e| Some(e.ok()?.path()))
        .collect();
    found.sort_by_key(|dir| (rank(dir), dir.clone()));
    found.into_iter().next()
}

fn number(text: &str) -> u64 {
    text.trim().parse().unwrap_or(0)
}

impl Service {
    /// Reads the level again; the read also arms the next change.
    fn read(&mut self) {
        let Some((_, _, file)) = &mut self.device else {
            return;
        };
        let mut text = String::new();
        if file.rewind().is_ok() && file.read_to_string(&mut text).is_ok() {
            self.level = number(&text);
        }
    }
}

impl client::Service for Service {
    type Command = Command;

    fn start(&mut self, _: &mut Client) -> io::Result<()> {
        self.read();
        Ok(())
    }

    fn message(&mut self, _: &mut Client, _: Message) {}

    fn command(&mut self, bus: &mut Client, command: Command) {
        let Command::Set(level) = command;
        let Some((name, max, _)) = &self.device else {
            return;
        };
        let value = (level.clamp(0.0, 1.0) * *max as f64).round() as u32;
        let body = vec![Value::str("backlight"), Value::str(name), Value::U32(value)];
        bus.invoke(
            "org.freedesktop.login1",
            "/org/freedesktop/login1/session/auto",
            "org.freedesktop.login1.Session",
            "SetBrightness",
            body,
        );
    }

    fn watched(&self) -> Option<BorrowedFd<'_>> {
        self.device.as_ref().map(|(_, _, file)| file.as_fd())
    }

    fn ready(&mut self, _: &mut Client) {
        self.read();
    }

    fn settle(&mut self, _: &mut Client) {
        let state = match &self.device {
            Some((_, max, _)) => BrightnessState {
                present: true,
                level: self.level as f64 / (*max).max(1) as f64,
            },
            None => BrightnessState::default(),
        };
        if self.last.as_ref() != Some(&state) {
            (self.changed)(&state);
            self.last = Some(state);
        }
    }
}

/// The running backlight service.
pub struct Brightness(Handle<Command>);

impl Brightness {
    /// Sets the level, 0–1.
    pub fn set(&self, level: f64) {
        if level.is_finite() {
            self.0.send(Command::Set(level));
        }
    }

    /// Follows the backlights listed under `root` instead of
    /// [`BACKLIGHTS`], with logind on `bus`.
    pub fn at(root: &Path, bus: &Bus, changed: Report<BrightnessState>) -> io::Result<Self> {
        let device = choose(root).and_then(|dir| {
            let name = dir.file_name()?.to_string_lossy().into_owned();
            let max = number(&std::fs::read_to_string(dir.join("max_brightness")).ok()?);
            let file = File::open(dir.join("actual_brightness")).ok()?;
            Some((name, max, file))
        });
        let service = Service {
            changed,
            device,
            level: 0,
            last: None,
        };
        client::spawn("brightness", bus, service).map(Self)
    }
}

impl System for Brightness {
    type State = BrightnessState;
    const BUS: Bus = Bus::System;

    fn spawn(bus: &Bus, changed: Report<BrightnessState>) -> io::Result<Self> {
        Self::at(Path::new(BACKLIGHTS), bus, changed)
    }
}
