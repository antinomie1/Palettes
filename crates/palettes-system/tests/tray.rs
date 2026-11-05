//! The tray as watcher: an item registers, shows a pixmap icon, opens its
//! menu and is chosen from; and as a host of another watcher.

mod common;

use common::*;
use palettes_system::{Message, System, Tray, Value};

const WATCHER: &str = "org.kde.StatusNotifierWatcher";

fn node(id: i32, props: Vec<(&str, Value)>, children: Vec<Value>) -> Value {
    let children = children.into_iter().map(Value::variant).collect();
    Value::Struct(vec![
        Value::I32(id),
        Value::dict(props),
        Value::Array("v".into(), children),
    ])
}

fn item(daemon: &Daemon, name: &str) -> Fake {
    Fake::serve(daemon, &[name], |call| {
        let red = [255u8, 255, 0, 0].repeat(4);
        let pixmap = Value::Struct(vec![
            Value::I32(2),
            Value::I32(2),
            Value::Array("y".into(), red.into_iter().map(Value::Byte).collect()),
        ]);
        let tooltip = Value::Struct(vec![
            Value::str(""),
            Value::Array("(iiay)".into(), vec![]),
            Value::str("Tip"),
            Value::str("<b>more</b>"),
        ]);
        let found = properties(
            call,
            &[(
                "/StatusNotifierItem",
                "org.kde.StatusNotifierItem",
                vec![
                    ("Id", Value::str("app")),
                    ("Title", Value::str("App")),
                    ("Status", Value::str("Active")),
                    ("IconName", Value::str("")),
                    ("IconPixmap", Value::Array("(iiay)".into(), vec![pixmap])),
                    ("ToolTip", tooltip),
                    ("Menu", Value::Path("/Menu".into())),
                ],
            )],
        );
        found.or_else(|| match call.member.as_str() {
            "AboutToShow" => Some(vec![Value::Bool(false)]),
            "GetLayout" => {
                let inner = node(
                    4,
                    vec![
                        ("label", Value::str("Inner")),
                        ("toggle-type", Value::str("checkmark")),
                        ("toggle-state", Value::I32(1)),
                    ],
                    vec![],
                );
                let root = node(
                    0,
                    vec![],
                    vec![
                        node(1, vec![("label", Value::str("_Quit"))], vec![]),
                        node(2, vec![("type", Value::str("separator"))], vec![]),
                        node(
                            3,
                            vec![
                                ("label", Value::str("Sub")),
                                ("children-display", Value::str("submenu")),
                            ],
                            vec![inner],
                        ),
                        node(
                            5,
                            vec![
                                ("label", Value::str("Hidden")),
                                ("visible", Value::Bool(false)),
                            ],
                            vec![],
                        ),
                    ],
                );
                Some(vec![Value::U32(1), root])
            }
            "Event" | "Activate" => Some(vec![]),
            _ => None,
        })
    })
}

#[test]
fn items_register_and_menus_open() {
    let Some(daemon) = daemon("tray") else { return };
    let (changed, states) = reports();
    let tray = Tray::spawn(&daemon.bus, changed).unwrap();
    until(&states, |s| s.items.is_empty());
    let mut peer = Peer::new(&daemon);
    // Wait until the tray owns the watcher name.
    loop {
        let bus = "org.freedesktop.DBus";
        let has = Message::call(
            bus,
            "/org/freedesktop/DBus",
            bus,
            "NameHasOwner",
            vec![Value::str(WATCHER)],
        );
        if peer.call(has).body[0].as_bool() == Some(true) {
            break;
        }
    }
    let fake = item(&daemon, "org.example.App");
    let register = Message::call(
        WATCHER,
        "/StatusNotifierWatcher",
        WATCHER,
        "RegisterStatusNotifierItem",
        vec![Value::str("org.example.App")],
    );
    fake.connection.send(&register).unwrap();
    let state = until(&states, |s| {
        s.items.len() == 1 && !s.items[0].icon_file.is_empty()
    });
    let shown = &state.items[0];
    assert_eq!(
        (shown.app_id.as_str(), shown.title.as_str()),
        ("app", "App")
    );
    assert_eq!(shown.tooltip, "Tip\nmore");
    assert!(shown.menu);
    let icon = std::fs::read(&shown.icon_file).unwrap();
    assert_eq!(&icon[1..4], b"PNG");

    let get = Message::call(
        WATCHER,
        "/StatusNotifierWatcher",
        "org.freedesktop.DBus.Properties",
        "Get",
        vec![
            Value::str(WATCHER),
            Value::str("RegisteredStatusNotifierItems"),
        ],
    );
    let items = peer.call(get);
    assert_eq!(
        items.body[0].items()[0].as_str(),
        Some("org.example.App/StatusNotifierItem")
    );

    tray.open_menu(&shown.id);
    let state = until(&states, |s| s.menu.is_some());
    let entries = &state.menu.as_ref().unwrap().entries;
    let summary: Vec<_> = entries
        .iter()
        .map(|e| (e.id, e.label.as_str(), e.separator, e.depth, e.checked))
        .collect();
    assert_eq!(
        summary,
        [
            (1, "Quit", false, 0, false),
            (2, "", true, 0, false),
            (3, "Sub", false, 0, false),
            (4, "Inner", false, 1, true)
        ]
    );
    let event = |id: i32, kind: &str| {
        let event = fake.called("Event");
        assert_eq!(
            (event.path.as_str(), &event.body[..2]),
            ("/Menu", &[Value::I32(id), Value::str(kind)][..])
        );
    };
    event(0, "opened");
    tray.select(1);
    event(1, "clicked");
    until(&states, |s| s.menu.is_none());

    // Dismissed without a choice.
    tray.open_menu(&shown.id);
    event(0, "opened");
    until(&states, |s| s.menu.is_some());
    tray.close_menu();
    event(0, "closed");
    until(&states, |s| s.menu.is_none());

    tray.activate(&shown.id, 3, 4);
    let activate = fake.called("Activate");
    assert_eq!(activate.body, [Value::I32(3), Value::I32(4)]);

    fake.close();
    until(&states, |s| s.items.is_empty());
}

#[test]
fn hosts_behind_another_watcher() {
    let Some(daemon) = daemon("tray-host") else {
        return;
    };
    let fake_item = item(&daemon, "org.example.Other");
    let watcher = Fake::serve(&daemon, &[WATCHER], |call| match call.member.as_str() {
        "RegisterStatusNotifierHost" => Some(vec![]),
        "Get" => Some(vec![Value::variant(Value::strings([
            "org.example.Other/StatusNotifierItem",
        ]))]),
        _ => None,
    });
    let (changed, states) = reports();
    let _tray = Tray::spawn(&daemon.bus, changed).unwrap();
    let host = watcher.called("RegisterStatusNotifierHost");
    assert!(
        host.body[0]
            .as_str()
            .unwrap()
            .starts_with("org.kde.StatusNotifierHost-")
    );
    let state = until(&states, |s| s.items.len() == 1);
    assert_eq!(state.items[0].title, "App");
    drop(fake_item);
}
