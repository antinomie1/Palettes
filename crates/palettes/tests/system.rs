//! System states and actions in documents, and `dbus` in scripts: checked
//! without a session, and live against a private session bus where a
//! Wayland session exists.

use palettes::{Shell, ipc, markup};
use std::{
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

fn dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("palettes-system-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(dir: &Path, name: &str, text: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, text).unwrap();
    path
}

const STATUS: &str = r#"use "palettes.aegle"
Panel {
    state battery: Battery = Battery(false, 0.0, "", false, 0, "", false)
    state power_profiles: list<string> = []
    state media: Player = Player("", "", "", "", false, "", "", "", "", 0.0, false, false, false, false)
    state notifications: list<Notification> = []
    state notification_actions: list<NotificationAction> = []
    state tray: list<TrayItem> = []
    state tray_menu: list<MenuEntry> = []
    state network: Network = Network("", "", false, 0, false, false)
    state wifi_networks: list<WifiNetwork> = []
    state bluetooth: Bluetooth = Bluetooth(false, false, false)
    state bluetooth_devices: list<BluetoothDevice> = []
    state brightness: float = 0.0
    state backlight: bool = false
    state cpu: float = 0.0
    state memory: Memory = Memory(0, 0, 0.0)
    state palette: list<string> = []
    Row {
        Text { text: str(len(notifications)) + " " + str(len(tray)) }
        Icon { name: "x"; on context_menu { host.tray_menu("a") } }
        Button { text: "x"; on clicked {
            host.set_power_profile("balanced")
            host.media_play_pause()
            host.media_seek(1.5)
            host.dismiss_notification(1)
            host.notification_action(1, "default")
            host.tray_activate("a")
            host.tray_menu("a")
            host.tray_select(2)
            host.toggle_wifi()
            host.wifi_connect("home", "")
            host.toggle_bluetooth()
            host.bluetooth_scan(true)
            host.bluetooth_pair("AA")
            host.set_brightness(0.5)
        } }
    }
}
"#;

#[test]
fn system_states_and_actions_check() {
    let dir = dir("check");
    let doc = write(&dir, "status.aegle", STATUS);
    markup::check(&doc).unwrap();
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    for example in ["status.aegle", "traymenu.aegle", "notifications.aegle"] {
        markup::check(&examples.join(example)).unwrap();
    }
    let wrong = write(
        &dir,
        "wrong.aegle",
        "use \"palettes.aegle\"\nPanel { Button { text: \"x\"; on clicked { host.wifi_connect(1) } } }\n",
    );
    assert!(markup::check(&wrong).is_err());
}

const SCRIPT: &str = r#"
local server = dbus.session("org.freedesktop.Notifications", "/org/freedesktop/Notifications")
local watcher = dbus.session("org.kde.StatusNotifierWatcher", "/StatusNotifierWatcher")
local id, closed, host, items = "", "", "", ""
server:signal("org.freedesktop.Notifications", "NotificationClosed", function(which, reason)
    closed = `{which}:{reason}`
end)
function follow()
    watcher:properties("org.kde.StatusNotifierWatcher", function(props)
        items = `{props.ProtocolVersion}/{#props.RegisteredStatusNotifierItems}`
    end)
end
function notify()
    local args = { "app", 0, "", "hello", "body", {}, {}, -1, signature = "susssasa{sv}i" }
    server:call("org.freedesktop.Notifications", "Notify", args, function(reply, err)
        id = reply and tostring(reply[1]) or err
    end)
    watcher:get("org.kde.StatusNotifierWatcher", "IsStatusNotifierHostRegistered", function(value)
        host = tostring(value)
    end)
end
function bad()
    server:call("org.freedesktop.Notifications", "Nothing", {}, function(reply, err) id = err end)
end
function report()
    return `{id}|{closed}|{host}|{items}|{#state.notifications}`
end
"#;

#[test]
fn scripts_talk_to_services_over_dbus() {
    if std::env::var_os("WAYLAND_DISPLAY").is_none() {
        eprintln!("skipped: no Wayland session");
        return;
    }
    let dir = dir("live");
    let Ok(mut daemon) = Command::new("dbus-daemon")
        .args(["--session", "--nofork", "--print-address"])
        .arg(format!("--address=unix:path={}/bus", dir.display()))
        .stdout(Stdio::piped())
        .spawn()
    else {
        eprintln!("skipped: no dbus-daemon");
        return;
    };
    let mut address = String::new();
    BufReader::new(daemon.stdout.take().unwrap())
        .read_line(&mut address)
        .unwrap();
    // SAFETY: the only test in this binary that reads the environment.
    unsafe {
        std::env::set_var("DBUS_SESSION_BUS_ADDRESS", address.trim());
        std::env::set_var("PALETTES_SOCKET", dir.join("ipc.sock"));
    }
    let doc = write(&dir, "status.aegle", STATUS);
    write(&dir, "status.luau", SCRIPT);
    let shell = Shell::new().unwrap();
    let documents = markup::Documents::with(shell.clone(), markup::elements::elements())
        .unwrap()
        .open(&doc)
        .unwrap();
    let ask = |request: &[&str]| {
        let request: Vec<String> = request.iter().map(|s| s.to_string()).collect();
        let (shell, deadline) = (shell.clone(), Instant::now() + Duration::from_secs(5));
        let asking = std::thread::spawn(move || ipc::send(&request).map_err(|e| e.to_string()));
        while !asking.is_finished() && Instant::now() < deadline {
            shell.dispatch(Some(Duration::from_millis(10))).unwrap();
        }
        asking.join().unwrap()
    };
    let until = |want: &dyn Fn(&str) -> bool| {
        let end = Instant::now() + Duration::from_secs(5);
        loop {
            let report = ask(&["report"]).unwrap();
            if want(&report) || Instant::now() > end {
                return report;
            }
            shell.dispatch(Some(Duration::from_millis(50))).unwrap();
        }
    };
    // Give the tray time to own the watcher name, then follow it.
    let end = Instant::now() + Duration::from_millis(500);
    while Instant::now() < end {
        shell.dispatch(Some(Duration::from_millis(10))).unwrap();
    }
    ask(&["follow"]).unwrap();
    until(&|r| r.contains("|0/0|"));
    ask(&["notify"]).unwrap();
    let report = until(&|r| r.ends_with("|1"));
    assert_eq!(report, "1||true|0/0|1");
    assert_eq!(ask(&["dismiss_notification", "1"]), Ok(String::new()));
    let report = until(&|r| r.ends_with("|0") && r.contains("1:2"));
    assert_eq!(report, "1|1:2|true|0/0|0");
    ask(&["bad"]).unwrap();
    let report = until(&|r| r.contains("UnknownMethod"));
    assert!(
        report.starts_with("org.freedesktop.DBus.Error.UnknownMethod"),
        "{report}"
    );
    drop(documents);
    let _ = daemon.kill();
    let _ = std::fs::remove_dir_all(&dir);
}
