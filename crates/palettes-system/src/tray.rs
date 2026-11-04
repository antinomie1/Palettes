//! The system tray: StatusNotifierItems, through the
//! `org.kde.StatusNotifierWatcher` this service owns, or through another
//! one already running as a host; and an item's `com.canonical.dbusmenu`
//! menu, read when opened.

use crate::{
    Report, System,
    client::{self, BUS, Bus, Client, Handle, PROPERTIES, Props},
    pixmap,
};
use aegle_dbus::{Message, Value};
use std::{io, path::Path};

const WATCHER: &str = "org.kde.StatusNotifierWatcher";
const WATCHER_PATH: &str = "/StatusNotifierWatcher";
const ITEM: &str = "org.kde.StatusNotifierItem";
const ITEM_PATH: &str = "/StatusNotifierItem";
const MENU: &str = "com.canonical.dbusmenu";
/// Icons are written near this size.
const ICON_PIXELS: i64 = 64;

const INTROSPECTION: &str = r#"<node>
<interface name="org.kde.StatusNotifierWatcher">
<method name="RegisterStatusNotifierItem"><arg type="s" direction="in"/></method>
<method name="RegisterStatusNotifierHost"><arg type="s" direction="in"/></method>
<property name="RegisteredStatusNotifierItems" type="as" access="read"/>
<property name="IsStatusNotifierHostRegistered" type="b" access="read"/>
<property name="ProtocolVersion" type="i" access="read"/>
<signal name="StatusNotifierItemRegistered"><arg type="s"/></signal>
<signal name="StatusNotifierItemUnregistered"><arg type="s"/></signal>
<signal name="StatusNotifierHostRegistered"/>
</interface>
</node>"#;

/// A tray item.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TrayItem {
    /// Its address, for actions: bus name and object path.
    pub id: String,
    /// The application's own id for it.
    pub app_id: String,
    /// Its title.
    pub title: String,
    /// `Active`, `Passive` (may be hidden) or `NeedsAttention`.
    pub status: String,
    /// Its icon name, for the icon theme.
    pub icon: String,
    /// Its icon as a file, when it named one, ships its own theme or sent
    /// pixels; else empty and `icon` is looked up.
    pub icon_file: String,
    /// Its tooltip: title, then description.
    pub tooltip: String,
    /// Whether it has a menu.
    pub menu: bool,
}

/// An entry of an open menu.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MenuEntry {
    /// Its id, for [`Tray::select`].
    pub id: i32,
    /// Its label, without mnemonic underscores.
    pub label: String,
    /// Its icon name.
    pub icon: String,
    /// Its icon as a file, when it sent one.
    pub icon_file: String,
    /// Whether it can be chosen.
    pub enabled: bool,
    /// `checkmark`, `radio` or empty.
    pub toggle: String,
    /// Whether its toggle is on.
    pub checked: bool,
    /// Whether it is a separator.
    pub separator: bool,
    /// Submenu depth: entries of a submenu follow it one deeper.
    pub depth: u32,
    /// Whether entries of its submenu follow.
    pub submenu: bool,
}

/// The open menu.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Menu {
    /// The [`TrayItem::id`] it belongs to.
    pub item: String,
    /// Its entries, submenus flattened.
    pub entries: Vec<MenuEntry>,
}

/// The tray.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TrayState {
    /// Items, in the order they registered.
    pub items: Vec<TrayItem>,
    /// The menu [`Tray::open_menu`] opened, until chosen from or closed.
    pub menu: Option<Menu>,
}

enum Command {
    Activate(String, i32, i32, bool),
    Scroll(String, i32, bool),
    OpenMenu(String),
    Select(i32),
    CloseMenu,
}

struct Item {
    id: String,
    /// The name calls go to.
    bus: String,
    owner: String,
    path: String,
    props: Props,
    stale: bool,
}

struct Open {
    item: String,
    owner: String,
    bus: String,
    path: String,
    entries: Vec<MenuEntry>,
    stale: bool,
}

struct Service {
    changed: Report<TrayState>,
    items: Vec<Item>,
    /// Whether this connection is the watcher.
    watcher: bool,
    host: String,
    menu: Option<Open>,
    last: Option<TrayState>,
}

/// A label without mnemonic underscores: `_File` is `File`, `__` is `_`.
fn label(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match c {
            '_' => out.extend(chars.next()),
            c => out.push(c),
        }
    }
    out
}

/// Text without markup tags, as some tooltips are HTML.
fn plain(text: &str) -> String {
    let mut out = String::new();
    let mut inside = false;
    for c in text.chars() {
        match c {
            '<' => inside = true,
            '>' => inside = false,
            c if !inside => out.push(c),
            _ => {}
        }
    }
    out.trim().to_owned()
}

/// The icon file named `name` under an item's own theme directory.
fn themed(dir: &Path, name: &str, depth: u32) -> Option<String> {
    let mut found: Option<(bool, u64, String)> = None;
    let mut dirs = vec![(dir.to_owned(), 0)];
    while let Some((dir, level)) = dirs.pop() {
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() && level < depth {
                dirs.push((path, level + 1));
                continue;
            }
            let (Some(stem), Some(ext)) = (path.file_stem(), path.extension()) else {
                continue;
            };
            if stem != name || !(ext == "png" || ext == "svg") {
                continue;
            }
            // SVG first, then the largest bitmap.
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            let rank = (ext == "svg", size, path.to_string_lossy().into_owned());
            if found.as_ref().is_none_or(|f| (f.0, f.1) < (rank.0, rank.1)) {
                found = Some(rank);
            }
        }
    }
    found.map(|f| f.2)
}

/// The pixmap of an `a(iiay)` property closest to the icon size, written.
fn pixmap_file(pixmaps: &[Value], name: &str) -> Option<String> {
    let sized = pixmaps.iter().filter_map(|p| {
        let [w, h, data] = p.items() else { return None };
        Some((w.as_i64()?, h.as_i64()?, data))
    });
    let (w, h, data) = sized.min_by_key(|(w, _, _)| match *w >= ICON_PIXELS {
        true => (0, *w),
        false => (1, -*w),
    })?;
    let argb: Vec<u8> = (data.items().iter())
        .map(|b| if let Value::Byte(b) = b { *b } else { 0 })
        .collect();
    let rgba = pixmap::argb_to_rgba(&argb);
    let written = pixmap::write_rgba(name, w as u32, h as u32, &rgba);
    written.ok().map(|p| p.to_string_lossy().into_owned())
}

fn report(item: &Item) -> TrayItem {
    let p = &item.props;
    let attention = p.str("Status") == "NeedsAttention";
    let pick = |key: &str| match attention && !p.str(&format!("Attention{key}")).is_empty() {
        true => format!("Attention{key}"),
        false => key.to_owned(),
    };
    let icon = p.str(&pick("IconName")).to_owned();
    let theme = p.str("IconThemePath");
    let pixmaps = p.items(&pick("IconPixmap"));
    let name = format!("tray-{}", pixmap::file_key(&item.id));
    let icon_file = match icon.starts_with('/') {
        true => Some(icon.clone()),
        false => (!theme.is_empty() && !icon.is_empty())
            .then(|| themed(Path::new(theme), &icon, 3))
            .flatten(),
    };
    let icon_file = icon_file
        .or_else(|| pixmap_file(pixmaps, &name))
        .unwrap_or_default();
    let tooltip = match p.get("ToolTip").map(Value::items) {
        Some([_, _, title, text]) => {
            let (title, text) = (
                title.as_str().unwrap_or(""),
                plain(text.as_str().unwrap_or("")),
            );
            [title, &text]
                .into_iter()
                .filter(|t| !t.is_empty())
                .collect::<Vec<_>>()
                .join("\n")
        }
        _ => String::new(),
    };
    TrayItem {
        id: item.id.clone(),
        app_id: p.str("Id").to_owned(),
        title: p.str("Title").to_owned(),
        status: p.str("Status").to_owned(),
        icon,
        icon_file,
        tooltip,
        menu: !matches!(p.str("Menu"), "" | "/"),
    }
}

/// Appends the visible children of a `(ia{sv}av)` layout node.
fn flatten(node: &Value, depth: u32, key: &str, out: &mut Vec<MenuEntry>) {
    let children = node.items().get(2).map(Value::items).unwrap_or(&[]);
    for child in children {
        let [id, props, grandchildren] = child.items() else {
            continue;
        };
        let props = Props::from_dict(props);
        if props.get("visible").and_then(Value::as_bool) == Some(false) {
            continue;
        }
        let id = id.as_i64().unwrap_or(0) as i32;
        let icon_data: Vec<u8> = (props.items("icon-data").iter())
            .map(|b| if let Value::Byte(b) = b { *b } else { 0 })
            .collect();
        let icon_file = match icon_data.is_empty() {
            true => String::new(),
            false => pixmap::write_file(&format!("menu-{key}-{id}.png"), &icon_data)
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default(),
        };
        let submenu =
            props.str("children-display") == "submenu" || !grandchildren.items().is_empty();
        out.push(MenuEntry {
            id,
            label: label(props.str("label")),
            icon: props.str("icon-name").to_owned(),
            icon_file,
            enabled: props
                .get("enabled")
                .and_then(Value::as_bool)
                .unwrap_or(true),
            toggle: props.str("toggle-type").to_owned(),
            checked: props.i64("toggle-state") == 1,
            separator: props.str("type") == "separator",
            depth,
            submenu,
        });
        if submenu {
            flatten(child, depth + 1, key, out);
        }
    }
}

impl Service {
    fn register(&mut self, bus: &mut Client, service: &str, sender: &str) {
        let (name, path) = match service.find('/') {
            Some(0) => (sender, service),
            Some(at) => service.split_at(at),
            None => (service, ITEM_PATH),
        };
        let id = format!("{name}{path}");
        if self.items.iter().any(|i| i.id == id) {
            return;
        }
        let Some(owner) = bus.owner(name) else {
            return;
        };
        self.items.push(Item {
            id: id.clone(),
            bus: name.to_owned(),
            owner,
            path: path.to_owned(),
            props: Props::default(),
            stale: true,
        });
        if self.watcher {
            let body = vec![Value::str(id)];
            let signal =
                Message::signal(WATCHER_PATH, WATCHER, "StatusNotifierItemRegistered", body);
            let _ = bus.send(&signal);
        }
    }

    fn remove(&mut self, bus: &mut Client, gone: impl Fn(&Item) -> bool) {
        let (removed, kept) = std::mem::take(&mut self.items).into_iter().partition(&gone);
        self.items = kept;
        for item in removed {
            if self.menu.as_ref().is_some_and(|m| m.item == item.id) {
                self.menu = None;
            }
            if self.watcher {
                let body = vec![Value::str(item.id)];
                let signal = Message::signal(
                    WATCHER_PATH,
                    WATCHER,
                    "StatusNotifierItemUnregistered",
                    body,
                );
                let _ = bus.send(&signal);
            }
        }
    }

    /// Registers as a host with another watcher and takes its items.
    fn join(&mut self, bus: &mut Client) -> io::Result<()> {
        let host = vec![Value::str(&self.host)];
        bus.method(
            WATCHER,
            WATCHER_PATH,
            WATCHER,
            "RegisterStatusNotifierHost",
            host,
        )?;
        let body = vec![
            Value::str(WATCHER),
            Value::str("RegisteredStatusNotifierItems"),
        ];
        let reply = bus.method(WATCHER, WATCHER_PATH, PROPERTIES, "Get", body)?;
        let items = reply.first().map(Value::items).unwrap_or(&[]);
        let services: Vec<String> = (items.iter())
            .filter_map(|s| Some(s.as_str()?.to_owned()))
            .collect();
        for service in services {
            self.register(bus, &service, "");
        }
        Ok(())
    }

    fn watcher_property(&self, name: &str) -> Option<Value> {
        Some(match name {
            "RegisteredStatusNotifierItems" => {
                Value::strings(self.items.iter().map(|i| i.id.as_str()))
            }
            "IsStatusNotifierHostRegistered" => Value::Bool(true),
            "ProtocolVersion" => Value::I32(0),
            _ => return None,
        })
    }

    fn serve(&mut self, bus: &mut Client, call: &Message) {
        let arg = |i: usize| {
            call.body
                .get(i)
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned()
        };
        let reply = match (call.interface.as_str(), call.member.as_str()) {
            (WATCHER, "RegisterStatusNotifierItem") if self.watcher => {
                self.register(bus, &arg(0), &call.sender);
                call.reply(vec![])
            }
            (WATCHER, "RegisterStatusNotifierHost") if self.watcher => call.reply(vec![]),
            (PROPERTIES, "Get") if self.watcher && arg(0) == WATCHER => {
                match self.watcher_property(&arg(1)) {
                    Some(value) => call.reply(vec![Value::variant(value)]),
                    None => call.error(
                        "org.freedesktop.DBus.Error.UnknownProperty",
                        "no such property",
                    ),
                }
            }
            (PROPERTIES, "GetAll") if self.watcher => {
                let names = [
                    "RegisteredStatusNotifierItems",
                    "IsStatusNotifierHostRegistered",
                    "ProtocolVersion",
                ];
                let all = names.map(|n| (n, self.watcher_property(n).expect("known")));
                call.reply(vec![Value::dict(all)])
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

    fn read_menu(&mut self, bus: &mut Client) {
        let Some(open) = &mut self.menu else { return };
        open.stale = false;
        let about = vec![Value::I32(0)];
        let _ = bus.method(&open.bus, &open.path, MENU, "AboutToShow", about);
        let body = vec![Value::I32(0), Value::I32(-1), Value::strings([])];
        match bus.method(&open.bus, &open.path, MENU, "GetLayout", body) {
            Ok(reply) => {
                let mut entries = Vec::new();
                if let Some(layout) = reply.get(1) {
                    flatten(layout, 0, &pixmap::file_key(&open.item), &mut entries);
                }
                open.entries = entries;
            }
            Err(e) => {
                eprintln!("palettes: tray menu: {e}");
                self.menu = None;
            }
        }
    }

    fn item(&self, id: &str) -> Option<&Item> {
        self.items.iter().find(|i| i.id == id)
    }
}

impl client::Service for Service {
    type Command = Command;

    fn start(&mut self, bus: &mut Client) -> io::Result<()> {
        bus.add_match(&format!(
            "type='signal',sender='{BUS}',member='NameOwnerChanged'"
        ))?;
        bus.add_match(&format!("type='signal',interface='{ITEM}'"))?;
        bus.add_match(&format!("type='signal',interface='{MENU}'"))?;
        bus.add_match(&format!(
            "type='signal',sender='{WATCHER}',interface='{WATCHER}'"
        ))?;
        bus.request_name(&self.host, false)?;
        self.watcher = bus.request_name(WATCHER, false)?;
        match self.watcher {
            true => {
                let signal = Message::signal(
                    WATCHER_PATH,
                    WATCHER,
                    "StatusNotifierHostRegistered",
                    vec![],
                );
                bus.send(&signal)?;
            }
            false => self.join(bus)?,
        }
        Ok(())
    }

    fn message(&mut self, bus: &mut Client, message: Message) {
        if message.kind == aegle_dbus::Kind::Call {
            return self.serve(bus, &message);
        }
        if message.is_signal(BUS, "NameOwnerChanged") {
            let text = |i: usize| {
                message
                    .body
                    .get(i)
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned()
            };
            let (name, old, new) = (text(0), text(1), text(2));
            if new.is_empty() {
                self.remove(bus, |i| i.owner == old || i.bus == name);
            }
            if name == WATCHER && !self.watcher {
                match new.is_empty() {
                    // The other watcher left: take its place.
                    true => {
                        self.watcher = bus.request_name(WATCHER, false).unwrap_or(false);
                        if self.watcher {
                            let s = Message::signal(
                                WATCHER_PATH,
                                WATCHER,
                                "StatusNotifierHostRegistered",
                                vec![],
                            );
                            let _ = bus.send(&s);
                        }
                    }
                    false if new != bus.name() => {
                        if let Err(e) = self.join(bus) {
                            eprintln!("palettes: tray: {e}");
                        }
                    }
                    false => {}
                }
            }
        } else if message.is_signal(WATCHER, "StatusNotifierItemRegistered") && !self.watcher {
            if let Some(service) = message.body.first().and_then(Value::as_str) {
                self.register(bus, service, "");
            }
        } else if message.interface == ITEM {
            for item in &mut self.items {
                if item.owner == message.sender && item.path == message.path {
                    item.stale = true;
                }
            }
        } else if message.interface == MENU
            && let Some(open) = &mut self.menu
            && open.owner == message.sender
            && open.path == message.path
        {
            open.stale = true;
        }
    }

    fn command(&mut self, bus: &mut Client, command: Command) {
        match command {
            Command::Activate(id, x, y, secondary) => {
                let Some(item) = self.item(&id) else { return };
                let menu_only = item.props.bool("ItemIsMenu");
                if menu_only && !secondary {
                    return self.command(bus, Command::OpenMenu(id));
                }
                let member = if secondary {
                    "SecondaryActivate"
                } else {
                    "Activate"
                };
                bus.invoke(
                    &item.bus,
                    &item.path,
                    ITEM,
                    member,
                    vec![Value::I32(x), Value::I32(y)],
                );
            }
            Command::Scroll(id, delta, horizontal) => {
                let Some(item) = self.item(&id) else { return };
                let orientation = if horizontal { "horizontal" } else { "vertical" };
                let body = vec![Value::I32(delta), Value::str(orientation)];
                bus.invoke(&item.bus, &item.path, ITEM, "Scroll", body);
            }
            Command::OpenMenu(id) => {
                let Some(item) = self.item(&id) else { return };
                let path = item.props.str("Menu");
                if matches!(path, "" | "/") {
                    // No exported menu: the item shows its own.
                    let body = vec![Value::I32(0), Value::I32(0)];
                    return bus.invoke(&item.bus, &item.path, ITEM, "ContextMenu", body);
                }
                self.menu = Some(Open {
                    item: id,
                    owner: item.owner.clone(),
                    bus: item.bus.clone(),
                    path: path.to_owned(),
                    entries: Vec::new(),
                    stale: false,
                });
                self.read_menu(bus);
            }
            Command::Select(entry) => {
                let Some(open) = self.menu.take() else { return };
                let body = vec![
                    Value::I32(entry),
                    Value::str("clicked"),
                    Value::variant(Value::I32(0)),
                    Value::U32(0),
                ];
                bus.invoke(&open.bus, &open.path, MENU, "Event", body);
            }
            Command::CloseMenu => {
                if let Some(open) = self.menu.take() {
                    let body = vec![
                        Value::I32(0),
                        Value::str("closed"),
                        Value::variant(Value::I32(0)),
                        Value::U32(0),
                    ];
                    bus.invoke(&open.bus, &open.path, MENU, "Event", body);
                }
            }
        }
    }

    fn settle(&mut self, bus: &mut Client) {
        let mut gone = Vec::new();
        for item in self.items.iter_mut().filter(|i| i.stale) {
            item.stale = false;
            match bus.get_all(&item.bus, &item.path, ITEM) {
                Ok(props) => item.props = props,
                Err(_) => gone.push(item.id.clone()),
            }
        }
        if !gone.is_empty() {
            self.remove(bus, |i| gone.contains(&i.id));
        }
        if self.menu.as_ref().is_some_and(|m| m.stale) {
            self.read_menu(bus);
        }
        let state = TrayState {
            items: self.items.iter().map(report).collect(),
            menu: (self.menu.as_ref()).map(|m| Menu {
                item: m.item.clone(),
                entries: m.entries.clone(),
            }),
        };
        if self.last.as_ref() != Some(&state) {
            (self.changed)(&state);
            self.last = Some(state);
        }
    }
}

/// The running tray.
pub struct Tray(Handle<Command>);

impl Tray {
    /// Activates an item, as a click does; an item that is only a menu
    /// opens it. `x` and `y` are where, on the screen, if known.
    pub fn activate(&self, item: &str, x: i32, y: i32) {
        self.0.send(Command::Activate(item.to_owned(), x, y, false));
    }

    /// The secondary action, as a middle click does.
    pub fn secondary_activate(&self, item: &str, x: i32, y: i32) {
        self.0.send(Command::Activate(item.to_owned(), x, y, true));
    }

    /// Scrolls over an item by `delta` steps.
    pub fn scroll(&self, item: &str, delta: i32, horizontal: bool) {
        self.0
            .send(Command::Scroll(item.to_owned(), delta, horizontal));
    }

    /// Reads an item's menu into [`TrayState::menu`].
    pub fn open_menu(&self, item: &str) {
        self.0.send(Command::OpenMenu(item.to_owned()));
    }

    /// Chooses an entry of the open menu, which closes it.
    pub fn select(&self, entry: i32) {
        self.0.send(Command::Select(entry));
    }

    /// Closes the open menu.
    pub fn close_menu(&self) {
        self.0.send(Command::CloseMenu);
    }
}

impl System for Tray {
    type State = TrayState;
    const BUS: Bus = Bus::Session;

    fn spawn(bus: &Bus, changed: Report<TrayState>) -> io::Result<Self> {
        let service = Service {
            changed,
            items: Vec::new(),
            watcher: false,
            host: format!("org.kde.StatusNotifierHost-{}", std::process::id()),
            menu: None,
            last: None,
        };
        client::spawn("tray", bus, service).map(Self)
    }
}
