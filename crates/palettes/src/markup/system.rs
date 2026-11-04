//! System states and actions: battery, media, notifications, tray, network,
//! bluetooth, the backlight and processor and memory use, each service
//! started only for the states a document
//! declares or the actions it runs. Records carry icon files, looked up
//! here from the names services report.

use super::{
    actions::action,
    states::{declares, list, record, report, set, string},
};
use crate::services::{self, Services, follow_system};
use aegle_loader::{Data, View, markup::Type};
use aegle_ui::{Node, Result};
use palettes_shell::Shell;
use palettes_system::{
    Battery, BatteryState, Bluetooth, BluetoothState, Brightness, BrightnessState, Control, Load,
    LoadState, Media, MediaState, Network, NetworkState, Notifications, NotificationsState, Player,
    System, Tray, TrayState,
};
use std::{cell::RefCell, rc::Rc};

fn int(n: impl Into<i64>) -> Data {
    Data::Int(n.into())
}

/// An icon file: `file` if set, else `name` as a path or from the theme.
fn icon(file: &str, name: &str) -> Data {
    if !file.is_empty() {
        return string(file);
    }
    if name.starts_with('/') {
        return string(name);
    }
    let found = services::icons().lookup(name, 64);
    string(
        &found
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default(),
    )
}

fn app_icon(app: &str) -> Option<String> {
    let found = services::icons().for_app(app, 64)?;
    Some(found.to_string_lossy().into_owned())
}

const BATTERY: &[&str] = &["battery", "power_profile", "power_profiles"];
const MEDIA: &[&str] = &["media", "players", "media_position"];
const NOTIFICATIONS: &[&str] = &[
    "notifications",
    "notification_popups",
    "notification_actions",
];
const TRAY: &[&str] = &["tray", "tray_menu"];
const NETWORK: &[&str] = &["network", "wifi_networks"];
const BLUETOOTH: &[&str] = &["bluetooth", "bluetooth_devices"];
const BRIGHTNESS: &[&str] = &["brightness", "backlight"];
const LOAD: &[&str] = &["cpu", "memory"];

fn brightness(view: &View, b: &BrightnessState) {
    set(view, "brightness", Data::Float(b.level as f32));
    set(view, "backlight", Data::Bool(b.present));
}

fn load(view: &View, l: &LoadState) {
    const MIB: u64 = 1024 * 1024;
    set(view, "cpu", Data::Float(l.cpu as f32));
    let percent = l.memory_used as f64 / l.memory_total.max(1) as f64;
    let memory = record([
        int((l.memory_used / MIB) as i64),
        int((l.memory_total / MIB) as i64),
        Data::Float(percent as f32),
    ]);
    set(view, "memory", memory);
}

fn battery(view: &View, b: &BatteryState) {
    let battery = record([
        Data::Bool(b.present),
        Data::Float((b.percent / 100.0) as f32),
        string(b.charge.name()),
        Data::Bool(b.charge == palettes_system::Charge::Charging),
        int(b.time.min(i64::MAX as u64) as i64),
        icon("", &b.icon),
        Data::Bool(b.on_battery),
    ]);
    set(view, "battery", battery);
    set(view, "power_profile", string(&b.profile));
    set(
        view,
        "power_profiles",
        list(b.profiles.iter().map(|p| string(p))),
    );
}

fn player(p: &Player) -> Data {
    record([
        string(&p.name),
        string(&p.identity),
        string(&app_icon(&p.desktop_entry).unwrap_or_default()),
        string(&p.status),
        Data::Bool(p.status == "Playing"),
        string(&p.title),
        string(&p.artist),
        string(&p.album),
        string(&p.art),
        Data::Float(p.length as f32),
        Data::Bool(p.can_next),
        Data::Bool(p.can_previous),
        Data::Bool(p.can_play),
        Data::Bool(p.can_seek),
    ])
}

fn media(view: &View, m: &MediaState) {
    set(
        view,
        "media",
        player(&m.current().cloned().unwrap_or_default()),
    );
    if view.get("players").is_some() {
        set(view, "players", list(m.players.iter().map(player)));
    }
    let position = m.current().map_or(0.0, |p| p.position);
    set(view, "media_position", Data::Float(position as f32));
}

fn notification(n: &palettes_system::Notification) -> Data {
    let file = match n.image.is_empty() {
        true => None,
        false => Some(n.image.clone()),
    };
    let named = (!n.icon.is_empty()).then(|| icon("", &n.icon));
    let icon = match (file, named) {
        (Some(file), _) => string(&file),
        (None, Some(Data::String(found))) if !found.is_empty() => Data::String(found),
        _ => string(&app_icon(&n.desktop_entry).unwrap_or_default()),
    };
    let actions = n.actions.iter().filter(|(key, _)| key != "default").count();
    record([
        int(n.id),
        string(&n.app),
        icon,
        string(&n.summary),
        string(&n.body),
        int(n.urgency),
        Data::Bool(n.popup),
        int(actions as i64),
    ])
}

fn notifications(view: &View, n: &NotificationsState) {
    set(view, "notifications", list(n.list.iter().map(notification)));
    let popups = n.list.iter().filter(|n| n.popup);
    set(view, "notification_popups", list(popups.map(notification)));
    let actions = n.list.iter().flat_map(|n| {
        let shown = n.actions.iter().filter(|(key, _)| key != "default");
        shown.map(|(key, label)| record([int(n.id), string(key), string(label)]))
    });
    set(view, "notification_actions", list(actions));
}

fn tray(view: &View, t: &TrayState) {
    let items = t.items.iter().map(|i| {
        record([
            string(&i.id),
            string(if i.title.is_empty() {
                &i.app_id
            } else {
                &i.title
            }),
            icon(&i.icon_file, &i.icon),
            string(&i.status),
            string(&i.tooltip),
            Data::Bool(i.menu),
        ])
    });
    set(view, "tray", list(items));
    let entries = t.menu.iter().flat_map(|m| &m.entries).map(|e| {
        record([
            int(e.id),
            string(&e.label),
            icon(&e.icon_file, &e.icon),
            Data::Bool(e.enabled),
            string(&e.toggle),
            Data::Bool(e.checked),
            Data::Bool(e.separator),
            int(e.depth),
            Data::Bool(e.submenu),
        ])
    });
    set(view, "tray_menu", list(entries));
}

fn network(view: &View, n: &NetworkState) {
    let network = record([
        string(&n.kind),
        string(&n.name),
        Data::Bool(n.connected),
        int(n.strength),
        Data::Bool(n.wifi),
        Data::Bool(n.wifi_available),
    ]);
    set(view, "network", network);
    let networks = n.networks.iter().map(|w| {
        record([
            string(&w.ssid),
            int(w.strength),
            Data::Bool(w.secure),
            Data::Bool(w.active),
            Data::Bool(w.saved),
        ])
    });
    set(view, "wifi_networks", list(networks));
}

fn bluetooth(view: &View, b: &BluetoothState) {
    let adapter = record([
        Data::Bool(b.available),
        Data::Bool(b.powered),
        Data::Bool(b.scanning),
    ]);
    set(view, "bluetooth", adapter);
    let devices = b.devices.iter().map(|d| {
        record([
            string(&d.address),
            string(&d.name),
            icon("", &d.icon),
            Data::Bool(d.paired),
            Data::Bool(d.connected),
            int(d.battery.map_or(-1, i64::from)),
        ])
    });
    set(view, "bluetooth_devices", list(devices));
}

/// Follows `S` for the view if it declares any of `names`.
fn bind_one<S: System>(
    shell: &Shell,
    shared: &Rc<RefCell<View>>,
    owner: &Node,
    names: &[&str],
    fill: fn(&View, &S::State),
    service: &str,
) {
    if !declares(&shared.borrow(), names) {
        return;
    }
    let target = shared.clone();
    let followed = follow_system::<S>(shell, owner, move |s| fill(&target.borrow(), s));
    report(followed, service);
}

/// Fills the system states `shared` declares while `owner` lives.
pub(super) fn bind(shell: &Shell, shared: &Rc<RefCell<View>>, owner: &Node) -> Result<()> {
    bind_one::<Battery>(shell, shared, owner, BATTERY, battery, "battery");
    bind_one::<Media>(shell, shared, owner, MEDIA, media, "media");
    if shared.borrow().get("media_position").is_some()
        && let Ok(media) = shell.system::<Media>()
    {
        media.track_position();
    }
    let n = NOTIFICATIONS;
    bind_one::<Notifications>(shell, shared, owner, n, notifications, "notifications");
    bind_one::<Tray>(shell, shared, owner, TRAY, tray, "tray");
    bind_one::<Network>(shell, shared, owner, NETWORK, network, "network");
    bind_one::<Bluetooth>(shell, shared, owner, BLUETOOTH, bluetooth, "bluetooth");
    let b = BRIGHTNESS;
    bind_one::<Brightness>(shell, shared, owner, b, brightness, "brightness");
    bind_one::<Load>(shell, shared, owner, LOAD, load, "load");
    Ok(())
}

fn text(a: &[Data], at: usize) -> &str {
    match a.get(at) {
        Some(Data::String(text)) => text,
        _ => "",
    }
}

fn number(a: &[Data], at: usize) -> f64 {
    match a.get(at) {
        Some(Data::Int(n)) => *n as f64,
        Some(Data::Float(n)) => f64::from(*n),
        _ => 0.0,
    }
}

fn flag(a: &[Data]) -> bool {
    matches!(a.first(), Some(Data::Bool(true)))
}

/// Registers an action on service `S`; without a shell it does nothing,
/// for checking documents.
fn on<S: System>(
    shell: Option<&Shell>,
    name: &str,
    params: &[Type],
    run: impl Fn(&S, &[Data]) + 'static,
) {
    let Some(shell) = shell.cloned() else {
        return action(name, params, |_| Ok(()));
    };
    action(name, params, move |a| {
        run(&*shell.system::<S>()?, a);
        Ok(())
    });
}

/// Registers the system actions; see `palettes.aegle`.
pub(super) fn actions(shell: Option<&Shell>) {
    use Type::{Bool, Float, Int, String};
    on::<Battery>(shell, "set_power_profile", &[String], |b, a| {
        b.set_profile(text(a, 0))
    });
    let media =
        |control: fn(&[Data]) -> Control| move |m: &Media, a: &[Data]| m.control(None, control(a));
    on::<Media>(
        shell,
        "media_play_pause",
        &[],
        media(|_| Control::PlayPause),
    );
    on::<Media>(shell, "media_next", &[], media(|_| Control::Next));
    on::<Media>(shell, "media_previous", &[], media(|_| Control::Previous));
    on::<Media>(
        shell,
        "media_seek",
        &[Float],
        media(|a| Control::Seek(number(a, 0))),
    );
    let id = |a: &[Data]| number(a, 0) as u32;
    on::<Notifications>(shell, "dismiss_notification", &[Int], move |n, a| {
        n.dismiss(id(a))
    });
    on::<Notifications>(shell, "dismiss_notifications", &[], |n, _| n.dismiss_all());
    on::<Notifications>(shell, "notification_action", &[Int, String], move |n, a| {
        n.invoke(id(a), text(a, 1))
    });
    on::<Tray>(shell, "tray_activate", &[String], |t, a| {
        t.activate(text(a, 0), 0, 0)
    });
    on::<Tray>(shell, "tray_secondary", &[String], |t, a| {
        t.secondary_activate(text(a, 0), 0, 0)
    });
    on::<Tray>(shell, "tray_scroll", &[String, Int], |t, a| {
        t.scroll(text(a, 0), number(a, 1) as i32, false)
    });
    on::<Tray>(shell, "tray_menu", &[String], |t, a| {
        t.open_menu(text(a, 0))
    });
    on::<Tray>(shell, "tray_select", &[Int], |t, a| {
        t.select(number(a, 0) as i32)
    });
    on::<Network>(shell, "toggle_wifi", &[], |n, _| n.set_wifi(None));
    on::<Network>(shell, "wifi_scan", &[], |n, _| n.scan());
    on::<Network>(shell, "wifi_connect", &[String, String], |n, a| {
        n.connect(text(a, 0), text(a, 1))
    });
    on::<Network>(shell, "wifi_disconnect", &[], |n, _| n.disconnect());
    on::<Bluetooth>(shell, "toggle_bluetooth", &[], |b, _| b.set_powered(None));
    on::<Bluetooth>(shell, "bluetooth_scan", &[Bool], |b, a| b.scan(flag(a)));
    on::<Bluetooth>(shell, "bluetooth_connect", &[String], |b, a| {
        b.connect(text(a, 0))
    });
    on::<Bluetooth>(shell, "bluetooth_disconnect", &[String], |b, a| {
        b.disconnect(text(a, 0))
    });
    on::<Bluetooth>(shell, "bluetooth_pair", &[String], |b, a| {
        b.pair(text(a, 0))
    });
    on::<Bluetooth>(shell, "bluetooth_forget", &[String], |b, a| {
        b.forget(text(a, 0))
    });
    on::<Brightness>(shell, "set_brightness", &[Float], |b, a| {
        b.set(number(a, 0))
    });
}
