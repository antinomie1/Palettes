//! `dbus` in scripts: proxies for objects on the session or system bus that
//! call methods, read and set properties, follow properties and signals.
//! One connection per bus, opened on first use; its messages come back to
//! the UI thread through the runtime's channel. See `docs/scripts.md`.

use super::{tasks, with};
use mlua::{Function, Lua, MultiValue, Table, Value};
use palettes_system::{Bus, Kind, Link, Message, Value as Dbus};
use std::{cell::RefCell, collections::HashMap};

const PROPERTIES: &str = "org.freedesktop.DBus.Properties";

/// What a reply answers.
enum Pending {
    /// `obj:call`: the callback gets the reply's values.
    Call(Function),
    /// `obj:get`: the callback gets the one value.
    Get(Function),
    /// `obj:properties`: every property for watch `id`, or one invalidated.
    Props(u64, Option<String>),
}

enum Follow {
    Signal(Function),
    Props(Function, Table),
}

struct Watch {
    id: u64,
    script: u64,
    bus: usize,
    rule: String,
    path: String,
    interface: String,
    member: String,
    /// For properties, the interface they belong to.
    arg0: Option<String>,
    follow: Follow,
}

#[derive(Default)]
struct State {
    links: [Option<Link>; 2],
    pending: HashMap<(usize, u32), (u64, Pending)>,
    watches: Vec<Watch>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::default();
}

fn error(e: impl std::fmt::Display) -> mlua::Error {
    mlua::Error::runtime(e.to_string())
}

/// The connection to bus `index` (0 session, 1 system), opened on first use.
fn link(index: usize) -> mlua::Result<Link> {
    if let Some(link) = STATE.with_borrow(|s| s.links[index].clone()) {
        return Ok(link);
    }
    let sender = with(|r| r.sender.clone()).ok_or_else(|| error("no script runtime"))?;
    let bus = if index == 0 {
        Bus::Session
    } else {
        Bus::System
    };
    let link = Link::open(&bus, move |message| {
        sender.send(tasks::Message {
            script: 0,
            callback: index as u64,
            event: tasks::Event::Bus(message),
        })
    })
    .map_err(error)?;
    STATE.with_borrow_mut(|s| s.links[index] = Some(link.clone()));
    Ok(link)
}

fn send(script: u64, bus: usize, message: Message, pending: Pending) -> mlua::Result<()> {
    let serial = link(bus)?.send(&message).map_err(error)?;
    STATE.with_borrow_mut(|s| s.pending.insert((bus, serial), (script, pending)));
    Ok(())
}

/// The first complete type of a signature and the rest.
fn split(signature: &str) -> Option<(&str, &str)> {
    let bytes = signature.as_bytes();
    let mut end = 0;
    while bytes.get(end) == Some(&b'a') {
        end += 1;
    }
    let (open, close) = match *bytes.get(end)? {
        b'(' => (b'(', b')'),
        b'{' => (b'{', b'}'),
        _ => return Some(signature.split_at(end + 1)),
    };
    let mut depth = 0;
    for (at, &b) in bytes.iter().enumerate().skip(end) {
        if b == open {
            depth += 1;
        } else if b == close {
            depth -= 1;
            if depth == 0 {
                return Some(signature.split_at(at + 1));
            }
        }
    }
    None
}

fn number(value: &Value) -> Option<f64> {
    match value {
        Value::Integer(n) => Some(*n as f64),
        Value::Number(n) => Some(*n),
        _ => None,
    }
}

/// A Luau value as D-Bus type `ty`.
fn typed(value: &Value, ty: &str) -> mlua::Result<Dbus> {
    let bad = || error(format!("cannot send {} as `{ty}`", value.type_name()));
    let int = || number(value).filter(|n| n.fract() == 0.0).ok_or_else(bad);
    let text = || match value {
        Value::String(s) => Ok(s.to_string_lossy()),
        _ => Err(bad()),
    };
    Ok(match ty.as_bytes().first().ok_or_else(bad)? {
        b'y' => Dbus::Byte(int()? as u8),
        b'b' => Dbus::Bool(value.as_boolean().ok_or_else(bad)?),
        b'n' => Dbus::I16(int()? as i16),
        b'q' => Dbus::U16(int()? as u16),
        b'i' => Dbus::I32(int()? as i32),
        b'u' => Dbus::U32(int()? as u32),
        b'x' => Dbus::I64(int()? as i64),
        b't' => Dbus::U64(int()? as u64),
        b'd' => Dbus::F64(number(value).ok_or_else(bad)?),
        b's' => Dbus::Str(text()?),
        b'o' => Dbus::Path(text()?),
        b'g' => Dbus::Signature(text()?),
        b'v' => Dbus::variant(guess(value)?),
        b'a' if ty == "ay" && value.is_string() => {
            let bytes = value.as_string().expect("string").as_bytes().to_vec();
            Dbus::Array("y".into(), bytes.into_iter().map(Dbus::Byte).collect())
        }
        b'a' => {
            let element = &ty[1..];
            let table = value.as_table().ok_or_else(bad)?;
            let items = match element.strip_prefix('{').and_then(|e| e.strip_suffix('}')) {
                Some(entry) => {
                    let (key, rest) = split(entry).ok_or_else(bad)?;
                    let pairs = table.pairs::<Value, Value>();
                    pairs
                        .map(|pair| {
                            let (k, v) = pair?;
                            Ok(Dbus::Entry(Box::new((typed(&k, key)?, typed(&v, rest)?))))
                        })
                        .collect::<mlua::Result<_>>()?
                }
                None => (table.sequence_values::<Value>())
                    .map(|v| typed(&v?, element))
                    .collect::<mlua::Result<_>>()?,
            };
            Dbus::Array(element.into(), items)
        }
        b'(' => {
            let table = value.as_table().ok_or_else(bad)?;
            let mut rest = &ty[1..ty.len() - 1];
            let mut fields = Vec::new();
            while !rest.is_empty() {
                let (field, tail) = split(rest).ok_or_else(bad)?;
                fields.push(typed(&table.raw_get(fields.len() + 1)?, field)?);
                rest = tail;
            }
            Dbus::Struct(fields)
        }
        _ => return Err(bad()),
    })
}

/// A Luau value as the D-Bus type it reads as: whole numbers `i` (or `x`
/// beyond 32 bits), other numbers `d`, sequences of strings `as`, other
/// sequences `av`, keyed tables `a{sv}`.
fn guess(value: &Value) -> mlua::Result<Dbus> {
    Ok(match value {
        Value::Boolean(b) => Dbus::Bool(*b),
        Value::String(s) => Dbus::Str(s.to_string_lossy()),
        Value::Integer(_) | Value::Number(_) => {
            let n = number(value).expect("number");
            match n.fract() == 0.0 && n.abs() < 9e15 {
                true if i32::try_from(n as i64).is_ok() => Dbus::I32(n as i32),
                true => Dbus::I64(n as i64),
                false => Dbus::F64(n),
            }
        }
        Value::Table(t) if t.raw_len() > 0 || t.pairs::<Value, Value>().next().is_none() => {
            let items: Vec<Value> = t.sequence_values().collect::<mlua::Result<_>>()?;
            match items.iter().all(Value::is_string) {
                true => typed(value, "as")?,
                false => Dbus::Array(
                    "v".into(),
                    (items.iter())
                        .map(|v| Ok(Dbus::variant(guess(v)?)))
                        .collect::<mlua::Result<_>>()?,
                ),
            }
        }
        Value::Table(_) => typed(value, "a{sv}")?,
        other => return Err(error(format!("cannot send {}", other.type_name()))),
    })
}

/// Arguments from a script's list, typed by its `signature` field if set.
fn arguments(args: Option<Table>) -> mlua::Result<Vec<Dbus>> {
    let Some(args) = args else {
        return Ok(Vec::new());
    };
    let values: Vec<Value> = args.sequence_values().collect::<mlua::Result<_>>()?;
    let Some(signature) = args.raw_get::<Option<String>>("signature")? else {
        return values.iter().map(guess).collect();
    };
    let mut rest = signature.as_str();
    let mut out = Vec::new();
    for value in &values {
        let (ty, tail) =
            split(rest).ok_or_else(|| error("signature has fewer types than arguments"))?;
        out.push(typed(value, ty)?);
        rest = tail;
    }
    Ok(out)
}

/// A D-Bus value in Luau: byte arrays are strings, dictionaries tables by
/// key, other arrays and structs sequences, variants their value.
fn to_lua(lua: &Lua, value: &Dbus) -> mlua::Result<Value> {
    Ok(match value {
        Dbus::Bool(b) => Value::Boolean(*b),
        Dbus::Byte(_)
        | Dbus::I16(_)
        | Dbus::U16(_)
        | Dbus::I32(_)
        | Dbus::U32(_)
        | Dbus::I64(_)
        | Dbus::U64(_)
        | Dbus::F64(_) => Value::Number(value.as_f64().unwrap_or(0.0)),
        Dbus::Str(s) | Dbus::Path(s) | Dbus::Signature(s) => Value::String(lua.create_string(s)?),
        Dbus::Variant(inner) => to_lua(lua, inner)?,
        Dbus::Array(element, items) if element == "y" => {
            let bytes: Vec<u8> = items
                .iter()
                .filter_map(|b| b.as_u64().map(|b| b as u8))
                .collect();
            Value::String(lua.create_string(bytes)?)
        }
        Dbus::Array(element, items) if element.starts_with('{') => {
            let table = lua.create_table()?;
            for item in items {
                if let Dbus::Entry(entry) = item {
                    table.raw_set(to_lua(lua, &entry.0)?, to_lua(lua, &entry.1)?)?;
                }
            }
            Value::Table(table)
        }
        Dbus::Array(_, items) | Dbus::Struct(items) => {
            let table = lua.create_table_with_capacity(items.len(), 0)?;
            for item in items {
                table.raw_push(to_lua(lua, item)?)?;
            }
            Value::Table(table)
        }
        Dbus::Entry(entry) => {
            let table = lua.create_table()?;
            table.raw_push(to_lua(lua, &entry.0)?)?;
            table.raw_push(to_lua(lua, &entry.1)?)?;
            Value::Table(table)
        }
    })
}

fn report(script: u64, result: mlua::Result<()>) {
    if let Err(e) = result {
        super::report(&super::script_path(script), &e);
    }
}

fn alive(script: u64) -> bool {
    with(|r| r.scripts.iter().any(|s| s.id == script)).unwrap_or(false)
}

/// A message from bus `bus`, on the UI thread.
pub(super) fn received(bus: usize, message: Message) {
    let Some(lua) = super::lua() else { return };
    if message.kind == Kind::Signal {
        return signal(&lua, bus, &message);
    }
    let Some((script, pending)) =
        STATE.with_borrow_mut(|s| s.pending.remove(&(bus, message.reply_serial)))
    else {
        return;
    };
    if !alive(script) {
        return;
    }
    let failed = (message.kind == Kind::Error).then(|| {
        let text = message.body.first().and_then(Dbus::as_str).unwrap_or("");
        format!("{}: {text}", message.error)
    });
    let result = (|| match pending {
        Pending::Call(function) => match failed {
            Some(e) => function.call::<()>((Value::Nil, e)),
            None => {
                let values = lua.create_table()?;
                for value in &message.body {
                    values.raw_push(to_lua(&lua, value)?)?;
                }
                function.call::<()>(values)
            }
        },
        Pending::Get(function) => match failed {
            Some(e) => function.call::<()>((Value::Nil, e)),
            None => function.call::<()>(match message.body.first() {
                Some(value) => to_lua(&lua, value)?,
                None => Value::Nil,
            }),
        },
        Pending::Props(id, name) => {
            if let Some(e) = failed {
                return Err(error(e));
            }
            let Some((function, table)) = props_of(id) else {
                return Ok(());
            };
            let value = match message.body.first() {
                Some(value) => to_lua(&lua, value)?,
                None => Value::Nil,
            };
            match (name, value) {
                (Some(name), value) => table.raw_set(name, value)?,
                (None, Value::Table(all)) => {
                    for pair in all.pairs::<Value, Value>() {
                        let (k, v) = pair?;
                        table.raw_set(k, v)?;
                    }
                }
                _ => {}
            }
            function.call::<()>(table)
        }
    })();
    report(script, result);
}

fn props_of(id: u64) -> Option<(Function, Table)> {
    STATE.with_borrow(|s| {
        s.watches
            .iter()
            .find(|w| w.id == id)
            .and_then(|w| match &w.follow {
                Follow::Props(f, t) => Some((f.clone(), t.clone())),
                Follow::Signal(_) => None,
            })
    })
}

fn signal(lua: &Lua, bus: usize, message: &Message) {
    let arg0 = message.body.first().and_then(Dbus::as_str);
    let matching: Vec<(u64, u64, Option<Function>)> = STATE.with_borrow(|s| {
        (s.watches.iter())
            .filter(|w| w.bus == bus && w.path == message.path && w.interface == message.interface)
            .filter(|w| w.member == message.member)
            .filter(|w| w.arg0.is_none() || w.arg0.as_deref() == arg0)
            .map(|w| match &w.follow {
                Follow::Signal(f) => (w.script, w.id, Some(f.clone())),
                Follow::Props(..) => (w.script, w.id, None),
            })
            .collect()
    });
    for (script, id, function) in matching {
        let result = (|| match function {
            Some(function) => {
                let args = (message.body.iter())
                    .map(|v| to_lua(lua, v))
                    .collect::<mlua::Result<MultiValue>>()?;
                function.call::<()>(args)
            }
            None => changed(lua, script, bus, id, message),
        })();
        report(script, result);
    }
}

/// Merges a `PropertiesChanged` signal into watch `id`'s table and asks
/// for the values it invalidated.
fn changed(lua: &Lua, script: u64, bus: usize, id: u64, message: &Message) -> mlua::Result<()> {
    let Some((function, table)) = props_of(id) else {
        return Ok(());
    };
    if let Some(Value::Table(changed)) = message.body.get(1).map(|v| to_lua(lua, v)).transpose()? {
        for pair in changed.pairs::<Value, Value>() {
            let (k, v) = pair?;
            table.raw_set(k, v)?;
        }
    }
    let invalidated = message.body.get(2).map(Dbus::items).unwrap_or(&[]);
    let (destination, path, interface) = STATE
        .with_borrow(|s| {
            let w = s.watches.iter().find(|w| w.id == id);
            w.map(|w| {
                (
                    destination_of(&w.rule),
                    w.path.clone(),
                    w.arg0.clone().unwrap_or_default(),
                )
            })
        })
        .unwrap_or_default();
    for name in invalidated.iter().filter_map(Dbus::as_str) {
        table.raw_set(name, Value::Nil)?;
        let body = vec![Dbus::str(&interface), Dbus::str(name)];
        let get = Message::call(&destination, &path, PROPERTIES, "Get", body);
        send(script, bus, get, Pending::Props(id, Some(name.to_owned())))?;
    }
    function.call::<()>(table)
}

/// The `sender` of a match rule.
fn destination_of(rule: &str) -> String {
    let start = rule.find("sender='").map_or(rule.len(), |at| at + 8);
    rule[start..].split('\'').next().unwrap_or("").to_owned()
}

/// Ends the watches of a script that unloads.
pub(super) fn unload(script: u64) {
    let ended: Vec<Watch> = STATE.with_borrow_mut(|s| {
        s.pending.retain(|_, (owner, _)| *owner != script);
        let (ended, kept) = std::mem::take(&mut s.watches)
            .into_iter()
            .partition(|w| w.script == script);
        s.watches = kept;
        ended
    });
    ended.into_iter().for_each(end);
}

fn end(watch: Watch) {
    if let Some(link) = STATE.with_borrow(|s| s.links[watch.bus].clone()) {
        link.remove_match(&watch.rule);
    }
}

/// Starts following a signal or properties; returns its handle.
#[allow(clippy::too_many_arguments)]
fn watch(
    lua: &Lua,
    script: u64,
    bus: usize,
    destination: &str,
    path: &str,
    interface: &str,
    member: &str,
    arg0: Option<String>,
    follow: Follow,
) -> mlua::Result<(u64, Table)> {
    let mut rule = format!(
        "type='signal',sender='{destination}',path='{path}',interface='{interface}',member='{member}'"
    );
    if let Some(arg0) = &arg0 {
        rule.push_str(&format!(",arg0='{arg0}'"));
    }
    link(bus)?.add_match(&rule).map_err(error)?;
    let id = super::next_id();
    STATE.with_borrow_mut(|s| {
        s.watches.push(Watch {
            id,
            script,
            bus,
            rule,
            path: path.to_owned(),
            interface: interface.to_owned(),
            member: member.to_owned(),
            arg0,
            follow,
        })
    });
    let handle = lua.create_table()?;
    handle.set(
        "stop",
        lua.create_function(move |_, _: MultiValue| {
            let found = STATE.with_borrow_mut(|s| {
                let at = s.watches.iter().position(|w| w.id == id)?;
                Some(s.watches.remove(at))
            });
            found.into_iter().for_each(end);
            Ok(())
        })?,
    )?;
    Ok((id, handle))
}

/// An object proxy: `dbus.system(destination, path)`.
fn object(
    lua: &Lua,
    script: u64,
    bus: usize,
    destination: String,
    path: String,
) -> mlua::Result<Table> {
    let object = lua.create_table()?;
    let (d, p) = (destination.clone(), path.clone());
    object.set(
        "call",
        lua.create_function(
            move |_,
                  (_, interface, method, args, callback): (
                Value,
                String,
                String,
                Option<Table>,
                Option<Function>,
            )| {
                let mut message = Message::call(&d, &p, &interface, &method, arguments(args)?);
                match callback {
                    Some(f) => send(script, bus, message, Pending::Call(f)),
                    None => {
                        message.no_reply = true;
                        link(bus)?.send(&message).map(drop).map_err(error)
                    }
                }
            },
        )?,
    )?;
    let (d, p) = (destination.clone(), path.clone());
    object.set(
        "get",
        lua.create_function(
            move |_, (_, interface, name, callback): (Value, String, String, Function)| {
                let body = vec![Dbus::str(interface), Dbus::str(name)];
                send(
                    script,
                    bus,
                    Message::call(&d, &p, PROPERTIES, "Get", body),
                    Pending::Get(callback),
                )
            },
        )?,
    )?;
    let (d, p) = (destination.clone(), path.clone());
    object.set(
        "set",
        lua.create_function(
            move |_,
                  (_, interface, name, value, signature): (
                Value,
                String,
                String,
                Value,
                Option<String>,
            )| {
                let value = match signature {
                    Some(ty) => typed(&value, &ty)?,
                    None => guess(&value)?,
                };
                let body = vec![Dbus::str(interface), Dbus::str(name), Dbus::variant(value)];
                let mut message = Message::call(&d, &p, PROPERTIES, "Set", body);
                message.no_reply = true;
                link(bus)?.send(&message).map(drop).map_err(error)
            },
        )?,
    )?;
    let (d, p) = (destination.clone(), path.clone());
    object.set(
        "properties",
        lua.create_function(
            move |lua, (_, interface, callback): (Value, String, Function)| {
                let follow = Follow::Props(callback, lua.create_table()?);
                let arg0 = Some(interface.clone());
                let (id, handle) = watch(
                    lua,
                    script,
                    bus,
                    &d,
                    &p,
                    PROPERTIES,
                    "PropertiesChanged",
                    arg0,
                    follow,
                )?;
                let all = Message::call(&d, &p, PROPERTIES, "GetAll", vec![Dbus::str(interface)]);
                send(script, bus, all, Pending::Props(id, None))?;
                Ok(handle)
            },
        )?,
    )?;
    object.set(
        "signal",
        lua.create_function(
            move |lua, (_, interface, member, callback): (Value, String, String, Function)| {
                let follow = Follow::Signal(callback);
                let (_, handle) = watch(
                    lua,
                    script,
                    bus,
                    &destination,
                    &path,
                    &interface,
                    &member,
                    None,
                    follow,
                )?;
                Ok(handle)
            },
        )?,
    )?;
    Ok(object)
}

/// The `dbus` table of script `script`.
pub(super) fn table(lua: &Lua, script: u64) -> mlua::Result<Table> {
    let dbus = lua.create_table()?;
    for (name, bus) in [("session", 0), ("system", 1)] {
        dbus.set(
            name,
            lua.create_function(move |lua, (destination, path): (String, String)| {
                object(lua, script, bus, destination, path)
            })?,
        )?;
    }
    Ok(dbus)
}
