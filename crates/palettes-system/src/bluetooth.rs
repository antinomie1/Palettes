//! Bluetooth from BlueZ: the first adapter and its devices, mirrored
//! through its object manager. Pairing this service starts goes through its
//! own agent: it confirms without asking (`NoInputNoOutput`) and refuses
//! what needs a code typed.

use crate::{
    Report, System,
    client::{self, Bus, Client, Handle},
    objects::Objects,
};
use aegle_dbus::{Message, Value};
use std::io;

const BLUEZ: &str = "org.bluez";
const ADAPTER: &str = "org.bluez.Adapter1";
const DEVICE: &str = "org.bluez.Device1";
const BATTERY: &str = "org.bluez.Battery1";
const AGENT: &str = "org.bluez.Agent1";
const AGENT_PATH: &str = "/org/palettes/BluetoothAgent";

/// A bluetooth device.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BluetoothDevice {
    /// Its address, for actions.
    pub address: String,
    /// Its name.
    pub name: String,
    /// Its icon name, such as `audio-headset`.
    pub icon: String,
    /// Whether it is paired.
    pub paired: bool,
    /// Whether it is connected.
    pub connected: bool,
    /// Its battery, 0–100, or `None` if it reports none.
    pub battery: Option<u8>,
}

/// The adapter and its devices.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BluetoothState {
    /// Whether there is an adapter.
    pub available: bool,
    /// Whether it is on.
    pub powered: bool,
    /// Whether it is looking for devices.
    pub scanning: bool,
    /// Known devices with a name: connected first, then paired, then by
    /// name.
    pub devices: Vec<BluetoothDevice>,
}

enum Command {
    Power(Option<bool>),
    Scan(bool),
    Connect(String, bool),
    Pair(String),
    Forget(String),
}

struct Service {
    changed: Report<BluetoothState>,
    objects: Objects,
    last: Option<BluetoothState>,
}

impl Service {
    fn adapter(&self) -> Option<&str> {
        self.objects.with(ADAPTER).next().map(|(path, _)| path)
    }

    fn device(&self, address: &str) -> Option<String> {
        let mut devices = self.objects.with(DEVICE);
        devices
            .find(|(_, d)| d.str("Address") == address)
            .map(|(path, _)| path.to_owned())
    }

    fn state(&self) -> BluetoothState {
        let adapter = self.adapter();
        let props = adapter.and_then(|a| self.objects.get(a, ADAPTER));
        let mut devices: Vec<BluetoothDevice> = (self.objects.with(DEVICE))
            .filter(|(_, d)| adapter.is_some_and(|a| d.str("Adapter") == a))
            .filter(|(_, d)| d.bool("Paired") || d.get("Name").is_some())
            .map(|(path, d)| BluetoothDevice {
                address: d.str("Address").to_owned(),
                name: match d.str("Alias") {
                    "" => d.str("Name").to_owned(),
                    alias => alias.to_owned(),
                },
                icon: d.str("Icon").to_owned(),
                paired: d.bool("Paired"),
                connected: d.bool("Connected"),
                battery: (self.objects.get(path, BATTERY))
                    .and_then(|b| b.get("Percentage")?.as_u64())
                    .map(|p| p.min(100) as u8),
            })
            .collect();
        devices.sort_by(|a, b| {
            (b.connected, b.paired, &a.name).cmp(&(a.connected, a.paired, &b.name))
        });
        BluetoothState {
            available: adapter.is_some(),
            powered: props.is_some_and(|p| p.bool("Powered")),
            scanning: props.is_some_and(|p| p.bool("Discovering")),
            devices,
        }
    }

    /// Answers BlueZ's calls to the agent: every pairing this connection
    /// asked for is confirmed.
    fn agent(&self, bus: &mut Client, call: &Message) {
        let reply = match call.member.as_str() {
            "RequestPinCode" | "RequestPasskey" => {
                call.error("org.bluez.Error.Rejected", "no way to type a code")
            }
            "Release"
            | "Cancel"
            | "DisplayPinCode"
            | "DisplayPasskey"
            | "RequestConfirmation"
            | "RequestAuthorization"
            | "AuthorizeService" => call.reply(vec![]),
            _ => call.error("org.freedesktop.DBus.Error.UnknownMethod", "no such method"),
        };
        let _ = bus.send(&reply);
    }
}

impl client::Service for Service {
    type Command = Command;

    fn start(&mut self, bus: &mut Client) -> io::Result<()> {
        self.objects = Objects::load(bus, BLUEZ, "/")?;
        let agent = vec![
            Value::Path(AGENT_PATH.into()),
            Value::str("NoInputNoOutput"),
        ];
        bus.invoke(
            BLUEZ,
            "/org/bluez",
            "org.bluez.AgentManager1",
            "RegisterAgent",
            agent,
        );
        Ok(())
    }

    fn message(&mut self, bus: &mut Client, message: Message) {
        if message.kind == aegle_dbus::Kind::Call && message.interface == AGENT {
            return self.agent(bus, &message);
        }
        self.objects.apply(&message);
    }

    fn command(&mut self, bus: &mut Client, command: Command) {
        let adapter = self.adapter().map(str::to_owned);
        match command {
            Command::Power(on) => {
                let Some(adapter) = adapter else { return };
                let current = self
                    .objects
                    .get(&adapter, ADAPTER)
                    .is_some_and(|p| p.bool("Powered"));
                bus.set(
                    BLUEZ,
                    &adapter,
                    ADAPTER,
                    "Powered",
                    Value::Bool(on.unwrap_or(!current)),
                );
            }
            Command::Scan(on) => {
                let Some(adapter) = adapter else { return };
                let member = if on {
                    "StartDiscovery"
                } else {
                    "StopDiscovery"
                };
                bus.invoke(BLUEZ, &adapter, ADAPTER, member, vec![]);
            }
            Command::Connect(address, on) => {
                let Some(device) = self.device(&address) else {
                    return;
                };
                let member = if on { "Connect" } else { "Disconnect" };
                bus.invoke(BLUEZ, &device, DEVICE, member, vec![]);
            }
            Command::Pair(address) => {
                let Some(device) = self.device(&address) else {
                    return;
                };
                bus.set(BLUEZ, &device, DEVICE, "Trusted", Value::Bool(true));
                bus.invoke(BLUEZ, &device, DEVICE, "Pair", vec![]);
            }
            Command::Forget(address) => {
                let (Some(adapter), Some(device)) = (adapter, self.device(&address)) else {
                    return;
                };
                bus.invoke(
                    BLUEZ,
                    &adapter,
                    ADAPTER,
                    "RemoveDevice",
                    vec![Value::Path(device)],
                );
            }
        }
    }

    fn settle(&mut self, _: &mut Client) {
        let state = self.state();
        if self.last.as_ref() != Some(&state) {
            (self.changed)(&state);
            self.last = Some(state);
        }
    }
}

/// The running bluetooth service.
pub struct Bluetooth(Handle<Command>);

impl Bluetooth {
    /// Turns the adapter on or off; `None` flips it.
    pub fn set_powered(&self, on: Option<bool>) {
        self.0.send(Command::Power(on));
    }

    /// Starts or stops looking for devices; looking stops by itself when
    /// the shell exits.
    pub fn scan(&self, on: bool) {
        self.0.send(Command::Scan(on));
    }

    /// Connects to a device by address.
    pub fn connect(&self, address: &str) {
        self.0.send(Command::Connect(address.to_owned(), true));
    }

    /// Disconnects a device.
    pub fn disconnect(&self, address: &str) {
        self.0.send(Command::Connect(address.to_owned(), false));
    }

    /// Pairs with and trusts a device.
    pub fn pair(&self, address: &str) {
        self.0.send(Command::Pair(address.to_owned()));
    }

    /// Removes a device and its pairing.
    pub fn forget(&self, address: &str) {
        self.0.send(Command::Forget(address.to_owned()));
    }
}

impl System for Bluetooth {
    type State = BluetoothState;
    const BUS: Bus = Bus::System;

    fn spawn(bus: &Bus, changed: Report<BluetoothState>) -> io::Result<Self> {
        let service = Service {
            changed,
            objects: Objects::default(),
            last: None,
        };
        client::spawn("bluetooth", bus, service).map(Self)
    }
}
