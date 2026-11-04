//! Battery, media, network and bluetooth against fake services.

mod common;

use common::*;
use palettes_system::{Battery, Bluetooth, Charge, Control, Media, Network, System, Value};
use std::sync::{Arc, Mutex};

const PROPS: &str = "org.freedesktop.DBus.Properties";

fn dict(entries: Vec<(&str, Value)>) -> Value {
    Value::dict(entries)
}

fn bytes(text: &str) -> Value {
    Value::Array("y".into(), text.bytes().map(Value::Byte).collect())
}

fn path(p: &str) -> Value {
    Value::Path(p.into())
}

#[test]
fn battery_and_power_profiles() {
    let Some(daemon) = daemon("battery") else {
        return;
    };
    let display = "/org/freedesktop/UPower/devices/DisplayDevice";
    let profiles = "/net/hadess/PowerProfiles";
    let fake = Fake::serve(
        &daemon,
        &["org.freedesktop.UPower", "net.hadess.PowerProfiles"],
        move |call| {
            let profile = |name: &str| dict(vec![("Profile", Value::str(name))]);
            properties(
                call,
                &[
                    (
                        display,
                        "org.freedesktop.UPower.Device",
                        vec![
                            ("IsPresent", Value::Bool(true)),
                            ("Percentage", Value::F64(42.0)),
                            ("State", Value::U32(2)),
                            ("TimeToEmpty", Value::I64(3600)),
                            ("IconName", Value::str("battery-good-symbolic")),
                        ],
                    ),
                    (
                        "/org/freedesktop/UPower",
                        "org.freedesktop.UPower",
                        vec![("OnBattery", Value::Bool(true))],
                    ),
                    (
                        profiles,
                        "net.hadess.PowerProfiles",
                        vec![
                            ("ActiveProfile", Value::str("balanced")),
                            (
                                "Profiles",
                                Value::Array(
                                    "a{sv}".into(),
                                    vec![profile("power-saver"), profile("balanced")],
                                ),
                            ),
                        ],
                    ),
                ],
            )
            .or_else(|| (call.member == "Set").then(Vec::new))
        },
    );
    let (changed, states) = reports();
    let battery = Battery::spawn(&daemon.bus, changed).unwrap();
    let state = until(&states, |s| s.present);
    assert_eq!(
        (state.percent, state.charge, state.time),
        (42.0, Charge::Discharging, 3600)
    );
    assert!(state.on_battery);
    assert_eq!(state.profiles, ["power-saver", "balanced"]);
    assert_eq!(state.profile, "balanced");

    let body = props_changed(
        "org.freedesktop.UPower.Device",
        vec![
            ("Percentage", Value::F64(43.5)),
            ("State", Value::U32(1)),
            ("TimeToFull", Value::I64(60)),
        ],
    );
    fake.signal(display, PROPS, "PropertiesChanged", body);
    let state = until(&states, |s| s.percent == 43.5);
    assert_eq!((state.charge, state.time), (Charge::Charging, 60));

    battery.set_profile("power-saver");
    let set = fake.called("Set");
    assert_eq!(set.path, profiles);
    assert_eq!(set.body[2], Value::variant(Value::str("power-saver")));
}

#[test]
fn media_players() {
    let Some(daemon) = daemon("media") else {
        return;
    };
    let mpris = "/org/mpris/MediaPlayer2";
    let status = Arc::new(Mutex::new("Paused"));
    let shared = status.clone();
    let fake = Fake::serve(&daemon, &["org.mpris.MediaPlayer2.test"], move |call| {
        let meta = dict(vec![
            ("mpris:trackid", path("/track/1")),
            ("xesam:title", Value::str("Song")),
            ("xesam:artist", Value::strings(["A", "B"])),
            ("mpris:length", Value::I64(180_000_000)),
            ("mpris:artUrl", Value::str("file:///tmp/cover%20art.png")),
        ]);
        let status = *shared.lock().unwrap();
        properties(
            call,
            &[
                (
                    mpris,
                    "org.mpris.MediaPlayer2",
                    vec![("Identity", Value::str("Test Player"))],
                ),
                (
                    mpris,
                    "org.mpris.MediaPlayer2.Player",
                    vec![
                        ("PlaybackStatus", Value::str(status)),
                        ("Metadata", meta),
                        ("Position", Value::I64(30_000_000)),
                        ("CanGoNext", Value::Bool(true)),
                        ("CanPlay", Value::Bool(true)),
                    ],
                ),
            ],
        )
        .or_else(|| call.interface.ends_with("Player").then(Vec::new))
    });
    let (changed, states) = reports();
    let media = Media::spawn(&daemon.bus, changed).unwrap();
    let state = until(&states, |s| s.current().is_some());
    let player = state.current().unwrap();
    assert_eq!(
        (player.name.as_str(), player.identity.as_str()),
        ("test", "Test Player")
    );
    assert_eq!(
        (player.title.as_str(), player.artist.as_str()),
        ("Song", "A, B")
    );
    assert_eq!((player.length, player.position), (180.0, 30.0));
    assert_eq!(player.art, "/tmp/cover art.png");
    assert!(player.can_next && !player.can_previous);

    media.control(None, Control::PlayPause);
    fake.called("PlayPause");
    media.control(Some("test"), Control::Seek(12.0));
    let seek = fake.called("SetPosition");
    assert_eq!(seek.body, [path("/track/1"), Value::I64(12_000_000)]);

    *status.lock().unwrap() = "Playing";
    let body = props_changed(
        "org.mpris.MediaPlayer2.Player",
        vec![("PlaybackStatus", Value::str("Playing"))],
    );
    fake.signal(mpris, PROPS, "PropertiesChanged", body);
    until(&states, |s| {
        s.current().is_some_and(|p| p.status == "Playing")
    });

    fake.close();
    until(&states, |s| s.players.is_empty());
}

#[test]
fn network_from_network_manager() {
    let Some(daemon) = daemon("network") else {
        return;
    };
    let nm = "org.freedesktop.NetworkManager";
    let object = |p: &str, interfaces: Vec<(&str, Vec<(&str, Value)>)>| {
        let interfaces = interfaces
            .into_iter()
            .map(|(name, props)| Value::Entry(Box::new((Value::str(name), dict(props)))));
        Value::Entry(Box::new((
            path(p),
            Value::Array("{sa{sv}}".into(), interfaces.collect()),
        )))
    };
    let device = "/org/freedesktop/NetworkManager/Devices/2";
    let (home, cafe) = (
        "/org/freedesktop/NetworkManager/AccessPoint/1",
        "/org/freedesktop/NetworkManager/AccessPoint/2",
    );
    let active = "/org/freedesktop/NetworkManager/ActiveConnection/1";
    let saved = "/org/freedesktop/NetworkManager/Settings/1";
    let objects = vec![
        object(
            "/org/freedesktop/NetworkManager",
            vec![(
                nm,
                vec![
                    ("WirelessEnabled", Value::Bool(true)),
                    ("PrimaryConnection", path(active)),
                    ("Connectivity", Value::U32(4)),
                ],
            )],
        ),
        object(
            device,
            vec![
                (
                    "org.freedesktop.NetworkManager.Device",
                    vec![("DeviceType", Value::U32(2))],
                ),
                (
                    "org.freedesktop.NetworkManager.Device.Wireless",
                    vec![
                        (
                            "AccessPoints",
                            Value::Array("o".into(), vec![path(home), path(cafe)]),
                        ),
                        ("ActiveAccessPoint", path(home)),
                    ],
                ),
            ],
        ),
        object(
            home,
            vec![(
                "org.freedesktop.NetworkManager.AccessPoint",
                vec![
                    ("Ssid", bytes("home")),
                    ("Strength", Value::Byte(70)),
                    ("RsnFlags", Value::U32(0x188)),
                ],
            )],
        ),
        object(
            cafe,
            vec![(
                "org.freedesktop.NetworkManager.AccessPoint",
                vec![("Ssid", bytes("cafe")), ("Strength", Value::Byte(90))],
            )],
        ),
        object(
            active,
            vec![(
                "org.freedesktop.NetworkManager.Connection.Active",
                vec![
                    ("Id", Value::str("home")),
                    ("Type", Value::str("802-11-wireless")),
                    ("SpecificObject", path(home)),
                ],
            )],
        ),
        object(
            saved,
            vec![("org.freedesktop.NetworkManager.Settings.Connection", vec![])],
        ),
    ];
    let fake = Fake::serve(&daemon, &[nm], move |call| match call.member.as_str() {
        "GetManagedObjects" => Some(vec![Value::Array("{oa{sa{sv}}}".into(), objects.clone())]),
        "GetSettings" => {
            let wireless = dict(vec![("ssid", bytes("home"))]);
            let section = Value::Entry(Box::new((Value::str("802-11-wireless"), wireless)));
            Some(vec![Value::Array("{sa{sv}}".into(), vec![section])])
        }
        "Set" | "ActivateConnection" | "AddAndActivateConnection" | "RequestScan" => Some(vec![]),
        _ => None,
    });
    let (changed, states) = reports();
    let network = Network::spawn(&daemon.bus, changed).unwrap();
    let state = until(&states, |s| s.connected);
    assert_eq!(
        (state.kind.as_str(), state.name.as_str(), state.strength),
        ("wifi", "home", 70)
    );
    assert!(state.wifi && state.wifi_available);
    let names: Vec<_> = state
        .networks
        .iter()
        .map(|n| (n.ssid.as_str(), n.active, n.secure, n.saved))
        .collect();
    assert_eq!(
        names,
        [("home", true, true, true), ("cafe", false, false, false)]
    );

    let body = props_changed(
        "org.freedesktop.NetworkManager.AccessPoint",
        vec![("Strength", Value::Byte(55))],
    );
    fake.signal(home, PROPS, "PropertiesChanged", body);
    until(&states, |s| s.strength == 55);

    network.connect("home", "");
    let activate = fake.called("ActivateConnection");
    assert_eq!(activate.body, [path(saved), path(device), path(home)]);
    network.connect("cafe", "secret");
    let add = fake.called("AddAndActivateConnection");
    let security = add.body[0].get("802-11-wireless-security").unwrap();
    assert_eq!(security.get("psk").and_then(Value::as_str), Some("secret"));
    network.set_wifi(None);
    let set = fake.called("Set");
    assert_eq!(set.body[2], Value::variant(Value::Bool(false)));
}

#[test]
fn bluetooth_from_bluez() {
    let Some(daemon) = daemon("bluetooth") else {
        return;
    };
    let object = |p: &str, interfaces: Vec<(&str, Vec<(&str, Value)>)>| {
        let interfaces = interfaces
            .into_iter()
            .map(|(name, props)| Value::Entry(Box::new((Value::str(name), dict(props)))));
        Value::Entry(Box::new((
            path(p),
            Value::Array("{sa{sv}}".into(), interfaces.collect()),
        )))
    };
    let adapter = "/org/bluez/hci0";
    let headset = "/org/bluez/hci0/dev_AA";
    let objects = vec![
        object(
            adapter,
            vec![(
                "org.bluez.Adapter1",
                vec![
                    ("Powered", Value::Bool(true)),
                    ("Discovering", Value::Bool(false)),
                ],
            )],
        ),
        object(
            headset,
            vec![
                (
                    "org.bluez.Device1",
                    vec![
                        ("Address", Value::str("AA")),
                        ("Name", Value::str("Headset")),
                        ("Alias", Value::str("Headset")),
                        ("Icon", Value::str("audio-headset")),
                        ("Paired", Value::Bool(true)),
                        ("Connected", Value::Bool(true)),
                        ("Adapter", path(adapter)),
                        ("RSSI", Value::I16(-60)),
                        ("Appearance", Value::U16(0x941)),
                    ],
                ),
                ("org.bluez.Battery1", vec![("Percentage", Value::Byte(80))]),
            ],
        ),
        object(
            "/org/bluez/hci0/dev_BB",
            vec![(
                "org.bluez.Device1",
                vec![
                    ("Address", Value::str("BB")),
                    ("Alias", Value::str("BB")),
                    ("Adapter", path(adapter)),
                ],
            )],
        ),
    ];
    let fake = Fake::serve(&daemon, &["org.bluez"], move |call| {
        match call.member.as_str() {
            "GetManagedObjects" => Some(vec![Value::Array("{oa{sa{sv}}}".into(), objects.clone())]),
            _ => Some(vec![]),
        }
    });
    let (changed, states) = reports();
    let bluetooth = Bluetooth::spawn(&daemon.bus, changed).unwrap();
    let state = until(&states, |s| s.available);
    assert!(state.powered && !state.scanning);
    assert_eq!(
        state.devices.len(),
        1,
        "unnamed, unpaired devices are left out"
    );
    let headset_state = &state.devices[0];
    assert_eq!(
        (headset_state.name.as_str(), headset_state.battery),
        ("Headset", Some(80))
    );
    fake.called("RegisterAgent");

    let speaker = object(
        "/org/bluez/hci0/dev_CC",
        vec![(
            "org.bluez.Device1",
            vec![
                ("Address", Value::str("CC")),
                ("Name", Value::str("Speaker")),
                ("Alias", Value::str("Speaker")),
                ("Adapter", path(adapter)),
            ],
        )],
    );
    let Value::Entry(entry) = speaker else {
        unreachable!()
    };
    fake.signal(
        "/",
        "org.freedesktop.DBus.ObjectManager",
        "InterfacesAdded",
        vec![entry.0, entry.1],
    );
    let state = until(&states, |s| s.devices.len() == 2);
    assert_eq!(state.devices[1].name, "Speaker");

    bluetooth.connect("CC");
    let connect = fake.called("Connect");
    assert_eq!(connect.path, "/org/bluez/hci0/dev_CC");
    bluetooth.scan(true);
    fake.called("StartDiscovery");
}
