//! The globals a script sees: `state`, `host`, `watch`, `exec`, `lines`,
//! `read`, `file`, `json`, `after`, `every` and `print`.

use super::{Callback, Message, Task, convert, tasks, with};
use mlua::{Function, Lua, LuaSerdeExt, MultiValue, Table, Value};
use palettes_shell::Sender;
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

fn runtime_error(e: impl std::fmt::Display) -> mlua::Error {
    mlua::Error::runtime(e.to_string())
}

/// A table whose fields are computed by `index` and assigned by `assign`.
fn proxy(lua: &Lua, index: Function, assign: Option<Function>) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    let meta = lua.create_table()?;
    meta.set("__index", index)?;
    if let Some(assign) = assign {
        meta.set("__newindex", assign)?;
    }
    table.set_metatable(Some(meta))?;
    Ok(table)
}

/// The globals of the script `id` at `path`: its own definitions go in the
/// returned table, over built-ins, over Luau's libraries.
pub(super) fn environment(lua: &Lua, id: u64, path: &Path) -> mlua::Result<Table> {
    let dir = path.parent().unwrap_or(Path::new(".")).to_owned();
    let base = lua.create_table()?;
    let state = proxy(
        lua,
        lua.create_function(|_, (_, name): (Value, String)| Ok(super::get(&name)))?,
        Some(
            lua.create_function(|_, (_, name, value): (Value, String, Value)| {
                super::set(&name, value)
            })?,
        ),
    )?;
    base.set("state", state)?;
    let host = proxy(
        lua,
        lua.create_function(|lua, (_, name): (Value, String)| {
            lua.create_function(move |lua, arguments: MultiValue| host(lua, &name, arguments))
        })?,
        None,
    )?;
    base.set("host", host)?;
    base.set(
        "watch",
        lua.create_function(move |_, (name, function): (String, Function)| {
            with(|r| {
                if let Some(s) = r.scripts.iter_mut().find(|s| s.id == id) {
                    s.watchers.push((name, function));
                }
            });
            Ok(())
        })?,
    )?;
    base.set(
        "exec",
        lua.create_function(
            move |lua, (command, function): (String, Option<Function>)| {
                let function = match function {
                    Some(f) => f,
                    None => lua.create_function(|_, _: MultiValue| Ok(()))?,
                };
                start(id, function, false, |sender, callback| {
                    tasks::command(&command, false, sender, id, callback)
                })?;
                Ok(())
            },
        )?,
    )?;
    base.set(
        "lines",
        lua.create_function(move |lua, (command, function): (String, Function)| {
            let callback = start(id, function, true, |sender, callback| {
                tasks::command(&command, true, sender, id, callback)
            })?;
            handle(lua, id, callback)
        })?,
    )?;
    for (name, repeat) in [("after", false), ("every", true)] {
        base.set(
            name,
            lua.create_function(move |lua, (seconds, function): (f64, Function)| {
                if !(seconds.is_finite() && seconds > 0.0) {
                    return Err(runtime_error("seconds must be positive"));
                }
                let period = Duration::from_secs_f64(seconds);
                let callback = start(id, function, false, |sender, callback| {
                    tasks::timer(period, repeat, sender, id, callback)
                })?;
                handle(lua, id, callback)
            })?,
        )?;
    }
    let read_dir = dir.clone();
    base.set(
        "read",
        lua.create_function(move |_, path: String| {
            Ok(std::fs::read_to_string(resolve(&read_dir, &path)).ok())
        })?,
    )?;
    base.set(
        "file",
        lua.create_function(move |_, (path, function): (String, Function)| {
            let path = resolve(&dir, &path);
            crate::markup::watch(&path);
            with(|r| {
                if let Some(s) = r.scripts.iter_mut().find(|s| s.id == id) {
                    s.files.push((path.clone(), function.clone()));
                }
            });
            function.call::<()>(std::fs::read_to_string(&path).ok())
        })?,
    )?;
    base.set("json", json(lua)?)?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    base.set(
        "print",
        lua.create_function(move |lua, values: MultiValue| {
            let words = values
                .into_iter()
                .map(|v| text(lua, v))
                .collect::<mlua::Result<Vec<_>>>()?;
            eprintln!("{name}: {}", words.join("\t"));
            Ok(())
        })?,
    )?;
    let globals = lua.globals();
    let meta = lua.create_table()?;
    meta.set("__index", globals)?;
    base.set_metatable(Some(meta))?;
    let env = lua.create_table()?;
    let meta = lua.create_table()?;
    meta.set("__index", base)?;
    env.set_metatable(Some(meta))?;
    Ok(env)
}

/// Starts a task reporting to `function`, owned by script `id`.
fn start(
    id: u64,
    function: Function,
    lines: bool,
    task: impl FnOnce(Sender<Message>, u64) -> std::io::Result<Task>,
) -> mlua::Result<u64> {
    let callback = super::next_id();
    let sender = with(|r| r.sender.clone()).ok_or_else(|| runtime_error("no script runtime"))?;
    let task = task(sender, callback).map_err(runtime_error)?;
    with(|r| {
        if let Some(s) = r.scripts.iter_mut().find(|s| s.id == id) {
            s.callbacks.insert(
                callback,
                Callback {
                    function,
                    lines,
                    task,
                },
            );
        }
    });
    Ok(callback)
}

/// What `lines`, `after` and `every` return: `handle:stop()` ends the task.
fn handle(lua: &Lua, id: u64, callback: u64) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    table.set(
        "stop",
        lua.create_function(move |_, _: MultiValue| {
            let removed = with(|r| {
                r.scripts
                    .iter_mut()
                    .find(|s| s.id == id)
                    .and_then(|s| s.callbacks.remove(&callback))
            });
            drop(removed);
            Ok(())
        })?,
    )?;
    Ok(table)
}

/// `host.name(...)`: a Palettes action, or a script's function.
fn host(_: &Lua, name: &str, arguments: MultiValue) -> mlua::Result<MultiValue> {
    if let Some(params) = crate::markup::actions::params(name) {
        let values: Vec<Value> = arguments.into_iter().collect();
        let data = params
            .iter()
            .enumerate()
            .map(|(i, ty)| convert::to_data(values.get(i).unwrap_or(&Value::Nil), ty))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| runtime_error(format!("host.{name}: {e}")))?;
        crate::markup::actions::call(name, &data).map_err(runtime_error)?;
        return Ok(MultiValue::new());
    }
    let (_, function) =
        super::function(name).ok_or_else(|| runtime_error(format!("no host action `{name}`")))?;
    function.call(arguments)
}

fn json(lua: &Lua) -> mlua::Result<Table> {
    let json = lua.create_table()?;
    json.set(
        "decode",
        lua.create_function(|lua, text: String| {
            let value: serde_json::Value = serde_json::from_str(&text).map_err(runtime_error)?;
            let options = mlua::serde::SerializeOptions::new()
                .serialize_none_to_null(false)
                .serialize_unit_to_null(false);
            lua.to_value_with(&value, options)
        })?,
    )?;
    json.set(
        "encode",
        lua.create_function(|lua, value: Value| {
            let value: serde_json::Value = lua.from_value(value)?;
            Ok(value.to_string())
        })?,
    )?;
    Ok(json)
}

/// A value as text: strings as they are, tables as JSON.
pub(super) fn text(lua: &Lua, value: Value) -> mlua::Result<String> {
    Ok(match value {
        Value::Nil => String::new(),
        Value::String(s) => s.to_string_lossy(),
        Value::Table(_) => {
            let value: serde_json::Value = lua.from_value(value)?;
            value.to_string()
        }
        other => other.to_string()?,
    })
}

/// A script's path: `~/` is the home directory, relative paths start at the
/// script's directory.
fn resolve(dir: &Path, path: &str) -> PathBuf {
    let path = match path.strip_prefix("~/") {
        Some(rest) => std::env::var_os("HOME")
            .map(|home| PathBuf::from(home).join(rest))
            .unwrap_or_else(|| PathBuf::from(path)),
        None => dir.join(path),
    };
    // The watcher reports paths under canonical directories.
    match (
        path.parent().and_then(|d| d.canonicalize().ok()),
        path.file_name(),
    ) {
        (Some(dir), Some(name)) => dir.join(name),
        _ => path,
    }
}
