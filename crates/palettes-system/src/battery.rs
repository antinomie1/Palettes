//! The battery from UPower's display device, and the power profile from
//! power-profiles-daemon (under UPower's name or its older `net.hadess` one).

use crate::{
    Report, System,
    client::{self, Bus, Client, Handle, Props, properties_changed},
};
use aegle_dbus::{Message, Value};
use std::io;

const UPOWER: &str = "org.freedesktop.UPower";
const UPOWER_PATH: &str = "/org/freedesktop/UPower";
const DISPLAY: &str = "/org/freedesktop/UPower/devices/DisplayDevice";
const DEVICE: &str = "org.freedesktop.UPower.Device";
/// power-profiles-daemon's names: since 0.20, then before.
const PROFILES: [(&str, &str); 2] = [
    (
        "org.freedesktop.UPower.PowerProfiles",
        "/org/freedesktop/UPower/PowerProfiles",
    ),
    ("net.hadess.PowerProfiles", "/net/hadess/PowerProfiles"),
];

/// What the battery is doing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Charge {
    /// Not known.
    #[default]
    Unknown,
    /// Charging.
    Charging,
    /// Running the system.
    Discharging,
    /// Empty.
    Empty,
    /// Full.
    Full,
    /// Plugged in, waiting to charge.
    PendingCharge,
    /// Waiting to discharge.
    PendingDischarge,
}

impl Charge {
    fn of(state: u64) -> Self {
        match state {
            1 => Self::Charging,
            2 => Self::Discharging,
            3 => Self::Empty,
            4 => Self::Full,
            5 => Self::PendingCharge,
            6 => Self::PendingDischarge,
            _ => Self::Unknown,
        }
    }

    /// Its name in states: `charging`, `discharging`, `empty`, `full`,
    /// `pending`, or empty when unknown.
    pub fn name(self) -> &'static str {
        match self {
            Self::Unknown => "",
            Self::Charging => "charging",
            Self::Discharging => "discharging",
            Self::Empty => "empty",
            Self::Full => "full",
            Self::PendingCharge | Self::PendingDischarge => "pending",
        }
    }
}

/// The battery and power profile.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BatteryState {
    /// Whether there is a battery.
    pub present: bool,
    /// Charge, 0–100.
    pub percent: f64,
    /// What it is doing.
    pub charge: Charge,
    /// Seconds until empty while discharging or full while charging; 0
    /// when unknown.
    pub time: u64,
    /// UPower's icon name for it.
    pub icon: String,
    /// Whether the system runs on battery.
    pub on_battery: bool,
    /// The active power profile, such as `balanced`; empty without
    /// power-profiles-daemon.
    pub profile: String,
    /// The profiles to choose from.
    pub profiles: Vec<String>,
}

enum Command {
    Profile(String),
}

struct Service {
    changed: Report<BatteryState>,
    device: Props,
    upower: Props,
    /// The profile daemon's name and path, and its properties.
    profiles: Option<(&'static str, &'static str, Props)>,
    last: Option<BatteryState>,
}

fn rule(path: &str) -> String {
    format!(
        "type='signal',interface='{}',member='PropertiesChanged',path='{path}'",
        client::PROPERTIES
    )
}

impl client::Service for Service {
    type Command = Command;

    fn start(&mut self, bus: &mut Client) -> io::Result<()> {
        bus.add_match(&rule(DISPLAY))?;
        bus.add_match(&rule(UPOWER_PATH))?;
        let device = bus.get_all(UPOWER, DISPLAY, DEVICE);
        self.upower = bus.get_all(UPOWER, UPOWER_PATH, UPOWER).unwrap_or_default();
        for (name, path) in PROFILES {
            if let Ok(props) = bus.get_all(name, path, name) {
                bus.add_match(&rule(path))?;
                self.profiles = Some((name, path, props));
                break;
            }
        }
        match device {
            Ok(device) => self.device = device,
            Err(e) if self.profiles.is_none() => return Err(e),
            Err(_) => {}
        }
        Ok(())
    }

    fn message(&mut self, _: &mut Client, message: Message) {
        let Some((interface, changed, _)) = properties_changed(&message) else {
            return;
        };
        let props = match (message.path.as_str(), interface) {
            (DISPLAY, DEVICE) => &mut self.device,
            (UPOWER_PATH, UPOWER) => &mut self.upower,
            _ => match &mut self.profiles {
                Some((name, path, props)) if *path == message.path && *name == interface => props,
                _ => return,
            },
        };
        props.merge(changed);
    }

    fn command(&mut self, bus: &mut Client, command: Command) {
        let Command::Profile(profile) = command;
        if let Some((name, path, _)) = self.profiles {
            bus.set(name, path, name, "ActiveProfile", Value::str(profile));
        }
    }

    fn settle(&mut self, _: &mut Client) {
        let d = &self.device;
        let charge = Charge::of(d.u64("State"));
        let time = match charge {
            Charge::Charging => d.i64("TimeToFull"),
            Charge::Discharging => d.i64("TimeToEmpty"),
            _ => 0,
        };
        let (profile, profiles) = match &self.profiles {
            Some((_, _, p)) => (
                p.str("ActiveProfile").to_owned(),
                (p.items("Profiles").iter())
                    .filter_map(|p| Some(p.get("Profile")?.as_str()?.to_owned()))
                    .collect(),
            ),
            None => Default::default(),
        };
        let state = BatteryState {
            present: d.bool("IsPresent"),
            percent: d.f64("Percentage"),
            charge,
            time: time.max(0) as u64,
            icon: d.str("IconName").to_owned(),
            on_battery: self.upower.bool("OnBattery"),
            profile,
            profiles,
        };
        if self.last.as_ref() != Some(&state) {
            (self.changed)(&state);
            self.last = Some(state);
        }
    }
}

/// The running battery service.
pub struct Battery(Handle<Command>);

impl Battery {
    /// Chooses a power profile, one of [`BatteryState::profiles`].
    pub fn set_profile(&self, profile: &str) {
        self.0.send(Command::Profile(profile.to_owned()));
    }
}

impl System for Battery {
    type State = BatteryState;
    const BUS: Bus = Bus::System;

    fn spawn(bus: &Bus, changed: Report<BatteryState>) -> io::Result<Self> {
        let service = Service {
            changed,
            device: Props::default(),
            upower: Props::default(),
            profiles: None,
            last: None,
        };
        client::spawn("battery", bus, service).map(Self)
    }
}
