//! A notification server: owns `org.freedesktop.Notifications` (queued
//! behind another server until it leaves) and keeps the notifications until
//! they are dismissed or expire.

use crate::{
    Report, System,
    client::{self, BUS, Bus, Client, Handle},
    pixmap::{self, local_path},
};
use aegle_dbus::{Message, Value};
use std::{
    io,
    path::PathBuf,
    time::{Duration, Instant, SystemTime},
};

const NAME: &str = "org.freedesktop.Notifications";
const PATH: &str = "/org/freedesktop/Notifications";
/// How long a new notification shows as a popup, unless it is critical.
const POPUP: Duration = Duration::from_secs(5);
/// Notifications kept; older ones expire.
const LIMIT: usize = 100;

const INTROSPECTION: &str = r#"<node>
<interface name="org.freedesktop.Notifications">
<method name="GetCapabilities"><arg type="as" direction="out"/></method>
<method name="Notify"><arg type="s" direction="in"/><arg type="u" direction="in"/><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="as" direction="in"/><arg type="a{sv}" direction="in"/><arg type="i" direction="in"/><arg type="u" direction="out"/></method>
<method name="CloseNotification"><arg type="u" direction="in"/></method>
<method name="GetServerInformation"><arg type="s" direction="out"/><arg type="s" direction="out"/><arg type="s" direction="out"/><arg type="s" direction="out"/></method>
<signal name="NotificationClosed"><arg type="u"/><arg type="u"/></signal>
<signal name="ActionInvoked"><arg type="u"/><arg type="s"/></signal>
</interface>
</node>"#;

/// A notification.
#[derive(Clone, Debug, PartialEq)]
pub struct Notification {
    /// Its id, for dismissing it or invoking its actions.
    pub id: u32,
    /// The sending application's name.
    pub app: String,
    /// The application's icon: an icon name or a file.
    pub icon: String,
    /// An image sent with it, as a file; empty without one.
    pub image: String,
    /// The sender's desktop entry id, if it named one.
    pub desktop_entry: String,
    /// One line.
    pub summary: String,
    /// More text.
    pub body: String,
    /// 0 low, 1 normal, 2 critical.
    pub urgency: u8,
    /// Actions as `(key, label)`; the key `default` is a click on the
    /// notification itself.
    pub actions: Vec<(String, String)>,
    /// Shown as a popup: new, for five seconds unless critical.
    pub popup: bool,
    /// When it arrived.
    pub time: SystemTime,
}

/// The notifications, newest first.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NotificationsState {
    /// Whether this server owns the bus name; while another server does,
    /// nothing arrives here.
    pub serving: bool,
    /// The notifications, newest first.
    pub list: Vec<Notification>,
}

enum Command {
    Dismiss(u32),
    DismissAll,
    Invoke(u32, String),
}

/// Why a notification closed, as the specification numbers it.
#[derive(Clone, Copy)]
enum Reason {
    Expired = 1,
    Dismissed = 2,
    Closed = 3,
}

struct Kept {
    notification: Notification,
    expires: Option<Instant>,
    popup_ends: Option<Instant>,
    resident: bool,
    image: Option<PathBuf>,
}

struct Service {
    changed: Report<NotificationsState>,
    serving: bool,
    kept: Vec<Kept>,
    next: u32,
    dirty: bool,
}

fn text(body: &[Value], at: usize) -> String {
    body.get(at)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned()
}

/// RGBA rows from the `(iiibiiay)` image hint, without row padding.
fn image_data(value: &Value) -> Option<(u32, u32, Vec<u8>)> {
    let [w, h, stride, alpha, bits, channels, data] = value.items() else {
        return None;
    };
    let (w, h) = (w.as_u64()? as usize, h.as_u64()? as usize);
    let (stride, alpha, channels) = (
        stride.as_u64()? as usize,
        alpha.as_bool()?,
        channels.as_u64()?,
    );
    if bits.as_u64()? != 8 || channels != if alpha { 4 } else { 3 } || w == 0 || h == 0 {
        return None;
    }
    let bytes: Vec<u8> = (data.items().iter())
        .map(|b| match b {
            Value::Byte(b) => *b,
            _ => 0,
        })
        .collect();
    let channels = channels as usize;
    let mut rgba = Vec::with_capacity(w * h * 4);
    for row in 0..h {
        let line = bytes.get(row * stride..row * stride + w * channels)?;
        for p in line.chunks_exact(channels) {
            rgba.extend([p[0], p[1], p[2], if alpha { p[3] } else { 255 }]);
        }
    }
    Some((w as u32, h as u32, rgba))
}

impl Service {
    fn notify(&mut self, bus: &mut Client, call: &Message) {
        let b = &call.body;
        let replaces = b.get(1).and_then(Value::as_u64).unwrap_or(0) as u32;
        let hints = b
            .get(6)
            .cloned()
            .unwrap_or(Value::Array("{sv}".into(), vec![]));
        let timeout = b.get(7).and_then(Value::as_i64).unwrap_or(-1);
        let id = match self.kept.iter().any(|k| k.notification.id == replaces) && replaces != 0 {
            true => replaces,
            false => {
                self.next = self.next.checked_add(1).unwrap_or(1);
                self.next
            }
        };
        let actions: Vec<String> = (b.get(5).map(Value::items).unwrap_or(&[]).iter())
            .filter_map(|a| a.as_str().map(str::to_owned))
            .collect();
        let hint = |key: &str| hints.get(key);
        let urgency = hint("urgency").and_then(Value::as_u64).unwrap_or(1).min(2) as u8;
        let now = Instant::now();
        let popup_ends = (urgency < 2).then(|| now + POPUP);
        let expires = match timeout {
            t if t > 0 => Some(now + Duration::from_millis(t as u64)),
            _ if hint("transient").and_then(Value::as_bool) == Some(true) => popup_ends,
            _ => None,
        };
        let image = ["image-data", "image_data", "icon_data"]
            .iter()
            .find_map(|key| image_data(hint(key)?))
            .and_then(|(w, h, rgba)| {
                let written = pixmap::write_rgba(&format!("notification-{id}"), w, h, &rgba);
                written
                    .map_err(|e| eprintln!("palettes: notification image: {e}"))
                    .ok()
            });
        let path_hint = ["image-path", "image_path"]
            .iter()
            .find_map(|key| local_path(hint(key)?.as_str()?));
        let icon = text(b, 2);
        let notification = Notification {
            id,
            app: text(b, 0),
            icon: local_path(&icon).unwrap_or(icon),
            image: (image.as_ref().map(|p| p.to_string_lossy().into_owned()))
                .or(path_hint)
                .unwrap_or_default(),
            desktop_entry: hint("desktop-entry")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned(),
            summary: text(b, 3),
            body: text(b, 4),
            urgency,
            actions: (actions.as_chunks::<2>().0.iter())
                .map(|pair| (pair[0].clone(), pair[1].clone()))
                .collect(),
            popup: true,
            time: SystemTime::now(),
        };
        let kept = Kept {
            notification,
            expires,
            popup_ends,
            resident: hint("resident").and_then(Value::as_bool) == Some(true),
            image,
        };
        match self.kept.iter().position(|k| k.notification.id == id) {
            Some(at) => {
                let old = std::mem::replace(&mut self.kept[at], kept);
                if let Some(image) = old
                    .image
                    .filter(|i| Some(i) != self.kept[at].image.as_ref())
                {
                    let _ = std::fs::remove_file(image);
                }
            }
            None => self.kept.insert(0, kept),
        }
        while self.kept.len() > LIMIT {
            let oldest = self.kept[self.kept.len() - 1].notification.id;
            self.close(bus, oldest, Reason::Expired);
        }
        self.dirty = true;
        let _ = bus.send(&call.reply(vec![Value::U32(id)]));
    }

    fn close(&mut self, bus: &mut Client, id: u32, reason: Reason) {
        let Some(at) = self.kept.iter().position(|k| k.notification.id == id) else {
            return;
        };
        let kept = self.kept.remove(at);
        if let Some(image) = kept.image {
            let _ = std::fs::remove_file(image);
        }
        let body = vec![Value::U32(id), Value::U32(reason as u32)];
        let _ = bus.send(&Message::signal(PATH, NAME, "NotificationClosed", body));
        self.dirty = true;
    }

    fn serve(&mut self, bus: &mut Client, call: &Message) {
        let reply = match (call.interface.as_str(), call.member.as_str()) {
            (NAME, "Notify") => return self.notify(bus, call),
            (NAME, "CloseNotification") => {
                if let Some(id) = call.body.first().and_then(Value::as_u64) {
                    self.close(bus, id as u32, Reason::Closed);
                }
                call.reply(vec![])
            }
            (NAME, "GetCapabilities") => {
                let caps = ["actions", "body", "icon-static", "persistence"];
                call.reply(vec![Value::strings(caps)])
            }
            (NAME, "GetServerInformation") => {
                let info = ["Palettes", "Palettes", env!("CARGO_PKG_VERSION"), "1.2"];
                call.reply(info.into_iter().map(Value::str).collect())
            }
            ("org.freedesktop.DBus.Introspectable", "Introspect") => {
                call.reply(vec![Value::str(INTROSPECTION)])
            }
            ("org.freedesktop.DBus.Peer", "Ping") => call.reply(vec![]),
            _ => call.error("org.freedesktop.DBus.Error.UnknownMethod", "no such method"),
        };
        if !call.no_reply {
            let _ = bus.send(&reply);
        }
    }
}

impl client::Service for Service {
    type Command = Command;

    fn start(&mut self, bus: &mut Client) -> io::Result<()> {
        self.serving = bus.request_name(NAME, true)?;
        if !self.serving {
            eprintln!("palettes: another notification server runs; waiting for it to leave");
        }
        Ok(())
    }

    fn message(&mut self, bus: &mut Client, message: Message) {
        if message.kind == aegle_dbus::Kind::Call {
            return self.serve(bus, &message);
        }
        let name = message.body.first().and_then(Value::as_str) == Some(NAME);
        if message.is_signal(BUS, "NameAcquired") && name {
            self.serving = true;
            self.dirty = true;
        } else if message.is_signal(BUS, "NameLost") && name {
            self.serving = false;
            self.dirty = true;
        }
    }

    fn command(&mut self, bus: &mut Client, command: Command) {
        match command {
            Command::Dismiss(id) => self.close(bus, id, Reason::Dismissed),
            Command::DismissAll => {
                let ids: Vec<u32> = self.kept.iter().map(|k| k.notification.id).collect();
                ids.into_iter()
                    .for_each(|id| self.close(bus, id, Reason::Dismissed));
            }
            Command::Invoke(id, key) => {
                let Some(kept) = self.kept.iter().find(|k| k.notification.id == id) else {
                    return;
                };
                let resident = kept.resident;
                let body = vec![Value::U32(id), Value::str(key)];
                let _ = bus.send(&Message::signal(PATH, NAME, "ActionInvoked", body));
                if !resident {
                    self.close(bus, id, Reason::Dismissed);
                }
            }
        }
    }

    fn deadline(&self) -> Option<Instant> {
        let ends = self.kept.iter().flat_map(|k| {
            let popup = k.popup_ends.filter(|_| k.notification.popup);
            [k.expires, popup]
        });
        ends.flatten().min()
    }

    fn expired(&mut self, bus: &mut Client) {
        let now = Instant::now();
        let expired: Vec<u32> = (self.kept.iter())
            .filter(|k| k.expires.is_some_and(|e| e <= now))
            .map(|k| k.notification.id)
            .collect();
        expired
            .into_iter()
            .for_each(|id| self.close(bus, id, Reason::Expired));
        for k in &mut self.kept {
            if k.notification.popup && k.popup_ends.is_some_and(|e| e <= now) {
                k.notification.popup = false;
                self.dirty = true;
            }
        }
    }

    fn settle(&mut self, _: &mut Client) {
        if std::mem::take(&mut self.dirty) {
            let state = NotificationsState {
                serving: self.serving,
                list: self.kept.iter().map(|k| k.notification.clone()).collect(),
            };
            (self.changed)(&state);
        }
    }
}

/// The running notification server.
pub struct Notifications(Handle<Command>);

impl Notifications {
    /// Dismisses a notification.
    pub fn dismiss(&self, id: u32) {
        self.0.send(Command::Dismiss(id));
    }

    /// Dismisses every notification.
    pub fn dismiss_all(&self) {
        self.0.send(Command::DismissAll);
    }

    /// Invokes a notification's action `key` (`default` for the
    /// notification itself), then dismisses it unless it is resident.
    pub fn invoke(&self, id: u32, key: &str) {
        self.0.send(Command::Invoke(id, key.to_owned()));
    }
}

impl System for Notifications {
    type State = NotificationsState;
    const BUS: Bus = Bus::Session;

    fn spawn(bus: &Bus, changed: Report<NotificationsState>) -> io::Result<Self> {
        let service = Service {
            changed,
            serving: false,
            kept: Vec::new(),
            next: 0,
            dirty: true,
        };
        client::spawn("notifications", bus, service).map(Self)
    }
}
