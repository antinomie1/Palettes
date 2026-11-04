//! The notification server: notify, replace, expire, invoke and dismiss.

mod common;

use common::*;
use palettes_system::{Message, Notifications, System, Value};
use std::time::Duration;

const NAME: &str = "org.freedesktop.Notifications";
const PATH: &str = "/org/freedesktop/Notifications";

fn notify(
    replaces: u32,
    summary: &str,
    actions: &[&str],
    hints: Vec<(&str, Value)>,
    timeout: i32,
) -> Message {
    let body = vec![
        Value::str("app"),
        Value::U32(replaces),
        Value::str("dialog-information"),
        Value::str(summary),
        Value::str("body"),
        Value::strings(actions.iter().copied()),
        Value::dict(hints),
        Value::I32(timeout),
    ];
    Message::call(NAME, PATH, NAME, "Notify", body)
}

fn id(reply: &Message) -> u32 {
    reply.body[0].as_u64().unwrap() as u32
}

#[test]
fn notifications_arrive_expire_and_close() {
    let Some(daemon) = daemon("notifications") else {
        return;
    };
    let (changed, states) = reports();
    let server = Notifications::spawn(&daemon.bus, changed).unwrap();
    until(&states, |s| s.serving);
    let mut app = Peer::new(&daemon);
    app.connection
        .add_match(&format!("type='signal',interface='{NAME}'"))
        .unwrap();

    let info = app.call(Message::call(
        NAME,
        PATH,
        NAME,
        "GetServerInformation",
        vec![],
    ));
    assert_eq!(info.body[0].as_str(), Some("Palettes"));

    let image = Value::Struct(vec![
        Value::I32(1),
        Value::I32(1),
        Value::I32(3),
        Value::Bool(false),
        Value::I32(8),
        Value::I32(3),
        Value::Array(
            "y".into(),
            vec![Value::Byte(255), Value::Byte(0), Value::Byte(0)],
        ),
    ]);
    let hints = vec![("urgency", Value::Byte(2)), ("image-data", image)];
    let first = id(&app.call(notify(
        0,
        "first",
        &["default", "Open", "later", "Later"],
        hints,
        0,
    )));
    let state = until(&states, |s| s.list.len() == 1);
    let n = &state.list[0];
    assert_eq!((n.summary.as_str(), n.urgency, n.popup), ("first", 2, true));
    assert_eq!(n.actions[1], ("later".to_owned(), "Later".to_owned()));
    assert_eq!(n.icon, "dialog-information");
    let image = n.image.clone();
    assert!(std::path::Path::new(&image).is_file(), "image written");

    // Replacing keeps the id; a short timeout expires it.
    let again = id(&app.call(notify(first, "replaced", &[], vec![], 0)));
    assert_eq!(again, first);
    until(&states, |s| {
        s.list.len() == 1 && s.list[0].summary == "replaced"
    });
    let brief = id(&app.call(notify(0, "brief", &[], vec![], 50)));
    until(&states, |s| s.list.len() == 2);
    let state = until(&states, |s| s.list.len() == 1);
    assert_eq!(state.list[0].id, first);
    let closed = app.signal("NotificationClosed");
    assert_eq!(closed.body, [Value::U32(brief), Value::U32(1)]);

    // Invoking an action signals it and dismisses the notification.
    let third = id(&app.call(notify(0, "third", &["default", "Open"], vec![], 0)));
    until(&states, |s| s.list.len() == 2);
    server.invoke(third, "default");
    let invoked = app.signal("ActionInvoked");
    assert_eq!(invoked.body, [Value::U32(third), Value::str("default")]);
    until(&states, |s| s.list.len() == 1);

    server.dismiss_all();
    until(&states, |s| s.list.is_empty());
    std::thread::sleep(Duration::from_millis(50));
    assert!(!std::path::Path::new(&image).exists(), "image removed");
}
