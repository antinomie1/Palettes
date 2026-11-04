//! Network from NetworkManager: the primary connection, Wi-Fi and the
//! networks in range, mirrored through its object manager.

use crate::{
    Report, System,
    client::{self, Bus, Client, Handle, byte_array, bytes_text},
    objects::Objects,
};
use aegle_dbus::{Message, Value};
use std::{collections::HashMap, io};

const NM: &str = "org.freedesktop.NetworkManager";
const NM_PATH: &str = "/org/freedesktop/NetworkManager";
const ACTIVE: &str = "org.freedesktop.NetworkManager.Connection.Active";
const DEVICE: &str = "org.freedesktop.NetworkManager.Device";
const WIRELESS: &str = "org.freedesktop.NetworkManager.Device.Wireless";
const POINT: &str = "org.freedesktop.NetworkManager.AccessPoint";
const SETTINGS: &str = "org.freedesktop.NetworkManager.Settings.Connection";

/// A Wi-Fi network in range.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WifiNetwork {
    /// Its name.
    pub ssid: String,
    /// Signal strength, 0–100, of its strongest access point.
    pub strength: u8,
    /// Whether it needs a password.
    pub secure: bool,
    /// Whether this machine is connected to it.
    pub active: bool,
    /// Whether a connection to it is saved, so it needs no password.
    pub saved: bool,
}

/// What the network is.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NetworkState {
    /// The primary connection's kind: `wifi`, `ethernet`, `vpn`, `other`,
    /// or empty without one.
    pub kind: String,
    /// Its name: the Wi-Fi network or the connection's.
    pub name: String,
    /// Whether NetworkManager reports full connectivity.
    pub connected: bool,
    /// Signal strength of the Wi-Fi network in use, 0–100.
    pub strength: u8,
    /// Whether Wi-Fi is on.
    pub wifi: bool,
    /// Whether there is a Wi-Fi device.
    pub wifi_available: bool,
    /// Networks in range, the active one first, then by strength.
    pub networks: Vec<WifiNetwork>,
}

enum Command {
    Wifi(Option<bool>),
    Scan,
    Connect(String, String),
    Disconnect,
}

struct Service {
    changed: Report<NetworkState>,
    objects: Objects,
    /// Saved connections' Wi-Fi names, by settings path.
    saved: HashMap<String, Option<String>>,
    last: Option<NetworkState>,
}

impl Service {
    fn wireless(&self) -> impl Iterator<Item = &str> {
        self.objects.with(WIRELESS).map(|(path, _)| path)
    }

    /// Reads the Wi-Fi names of saved connections not read yet.
    fn read_saved(&mut self, bus: &mut Client) {
        let paths: Vec<String> = (self.objects.with(SETTINGS))
            .map(|(path, _)| path.to_owned())
            .filter(|path| !self.saved.contains_key(path))
            .collect();
        for path in paths {
            let reply = bus.method(NM, &path, SETTINGS, "GetSettings", vec![]);
            let ssid = reply.ok().and_then(|r| {
                let wireless = r.first()?.get("802-11-wireless")?.get("ssid").cloned();
                wireless.map(|s| bytes_text(Some(&s)))
            });
            self.saved.insert(path, ssid);
        }
        let objects = &self.objects;
        self.saved
            .retain(|path, _| objects.get(path, SETTINGS).is_some());
    }

    fn saved_for(&self, ssid: &str) -> Option<&str> {
        let mut saved = self.saved.iter();
        saved
            .find(|(_, s)| s.as_deref() == Some(ssid))
            .map(|(p, _)| p.as_str())
    }

    fn state(&self) -> NetworkState {
        let o = &self.objects;
        let nm = o.get(NM_PATH, NM).cloned().unwrap_or_default();
        let primary = nm.str("PrimaryConnection");
        let active = o.get(primary, ACTIVE);
        let point = active.and_then(|a| o.get(a.str("SpecificObject"), POINT));
        let kind = match active.map(|a| a.str("Type")) {
            None => "",
            Some("802-11-wireless") => "wifi",
            Some("802-3-ethernet") => "ethernet",
            Some("vpn" | "wireguard") => "vpn",
            Some(_) => "other",
        };
        let name = match point {
            Some(p) => bytes_text(p.get("Ssid")),
            None => active.map(|a| a.str("Id").to_owned()).unwrap_or_default(),
        };
        let current: Vec<&str> = (self.wireless())
            .filter_map(|d| o.get(d, WIRELESS).map(|w| w.str("ActiveAccessPoint")))
            .collect();
        let mut networks: Vec<WifiNetwork> = Vec::new();
        for device in self.wireless() {
            let points = o
                .get(device, WIRELESS)
                .map(|w| w.items("AccessPoints"))
                .unwrap_or(&[]);
            for path in points.iter().filter_map(Value::as_str) {
                let Some(p) = o.get(path, POINT) else {
                    continue;
                };
                let ssid = bytes_text(p.get("Ssid"));
                if ssid.is_empty() {
                    continue;
                }
                let found = WifiNetwork {
                    strength: p.u64("Strength").min(100) as u8,
                    secure: p.u64("Flags") & 1 != 0
                        || p.u64("WpaFlags") != 0
                        || p.u64("RsnFlags") != 0,
                    active: current.contains(&path),
                    saved: self.saved_for(&ssid).is_some(),
                    ssid,
                };
                match networks.iter_mut().find(|n| n.ssid == found.ssid) {
                    Some(n) => {
                        n.active |= found.active;
                        n.strength = n.strength.max(found.strength);
                    }
                    None => networks.push(found),
                }
            }
        }
        networks
            .sort_by(|a, b| (b.active, b.strength, &a.ssid).cmp(&(a.active, a.strength, &b.ssid)));
        NetworkState {
            kind: kind.into(),
            name,
            connected: nm.u64("Connectivity") == 4
                || (nm.get("Connectivity").is_none() && active.is_some()),
            strength: point.map_or(0, |p| p.u64("Strength").min(100) as u8),
            wifi: nm.bool("WirelessEnabled"),
            wifi_available: self.wireless().next().is_some(),
            networks,
        }
    }

    fn connect(&mut self, bus: &mut Client, ssid: &str, password: &str) {
        let Some(device) = self.wireless().next().map(str::to_owned) else {
            return;
        };
        let o = &self.objects;
        let points = o
            .get(&device, WIRELESS)
            .map(|w| w.items("AccessPoints"))
            .unwrap_or(&[]);
        let point = (points.iter().filter_map(Value::as_str))
            .filter(|p| {
                o.get(p, POINT)
                    .is_some_and(|p| bytes_text(p.get("Ssid")) == ssid)
            })
            .max_by_key(|p| o.get(p, POINT).map_or(0, |p| p.u64("Strength")))
            .unwrap_or("/")
            .to_owned();
        let (device, point) = (Value::Path(device), Value::Path(point));
        if let Some(saved) = self.saved_for(ssid).filter(|_| password.is_empty()) {
            let body = vec![Value::Path(saved.to_owned()), device, point];
            return bus.invoke(NM, NM_PATH, NM, "ActivateConnection", body);
        }
        let section = |entries: Vec<(&str, Value)>| Value::dict(entries).items().to_vec();
        let mut settings = vec![(
            "802-11-wireless",
            section(vec![("ssid", byte_array(ssid.as_bytes()))]),
        )];
        if !password.is_empty() {
            let security = vec![
                ("key-mgmt", Value::str("wpa-psk")),
                ("psk", Value::str(password)),
            ];
            settings.push(("802-11-wireless-security", section(security)));
        }
        let settings = settings.into_iter().map(|(name, entries)| {
            let section = Value::Array("{sv}".into(), entries);
            Value::Entry(Box::new((Value::str(name), section)))
        });
        let settings = Value::Array("{sa{sv}}".into(), settings.collect());
        bus.invoke(
            NM,
            NM_PATH,
            NM,
            "AddAndActivateConnection",
            vec![settings, device, point],
        );
    }
}

impl client::Service for Service {
    type Command = Command;

    fn start(&mut self, bus: &mut Client) -> io::Result<()> {
        self.objects = Objects::load(bus, NM, "/org/freedesktop")?;
        self.read_saved(bus);
        Ok(())
    }

    fn message(&mut self, _: &mut Client, message: Message) {
        if message.is_signal(SETTINGS, "Updated") {
            self.saved.remove(&message.path);
        }
        self.objects.apply(&message);
    }

    fn command(&mut self, bus: &mut Client, command: Command) {
        match command {
            Command::Wifi(on) => {
                let current = self
                    .objects
                    .get(NM_PATH, NM)
                    .is_some_and(|p| p.bool("WirelessEnabled"));
                let on = on.unwrap_or(!current);
                bus.set(NM, NM_PATH, NM, "WirelessEnabled", Value::Bool(on));
            }
            Command::Scan => {
                for device in self.wireless() {
                    let options = Value::dict([]);
                    bus.invoke(NM, device, WIRELESS, "RequestScan", vec![options]);
                }
            }
            Command::Connect(ssid, password) => self.connect(bus, &ssid, &password),
            Command::Disconnect => {
                let active = self.wireless().find(|d| {
                    let w = self.objects.get(d, WIRELESS);
                    w.is_some_and(|w| !matches!(w.str("ActiveAccessPoint"), "" | "/"))
                });
                if let Some(device) = active {
                    bus.invoke(NM, device, DEVICE, "Disconnect", vec![]);
                }
            }
        }
    }

    fn settle(&mut self, bus: &mut Client) {
        self.read_saved(bus);
        let state = self.state();
        if self.last.as_ref() != Some(&state) {
            (self.changed)(&state);
            self.last = Some(state);
        }
    }
}

/// The running network service.
pub struct Network(Handle<Command>);

impl Network {
    /// Turns Wi-Fi on or off; `None` flips it.
    pub fn set_wifi(&self, on: Option<bool>) {
        self.0.send(Command::Wifi(on));
    }

    /// Asks Wi-Fi devices to look for networks.
    pub fn scan(&self) {
        self.0.send(Command::Scan);
    }

    /// Connects to the Wi-Fi network `ssid`: through its saved connection
    /// when `password` is empty and there is one, else as a new connection
    /// (WPA with `password`, open without).
    pub fn connect(&self, ssid: &str, password: &str) {
        self.0
            .send(Command::Connect(ssid.to_owned(), password.to_owned()));
    }

    /// Disconnects Wi-Fi.
    pub fn disconnect(&self) {
        self.0.send(Command::Disconnect);
    }
}

impl System for Network {
    type State = NetworkState;
    const BUS: Bus = Bus::System;

    fn spawn(bus: &Bus, changed: Report<NetworkState>) -> io::Result<Self> {
        let service = Service {
            changed,
            objects: Objects::default(),
            saved: HashMap::new(),
            last: None,
        };
        client::spawn("network", bus, service).map(Self)
    }
}
