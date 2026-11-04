//! Luau scripts: `bar.luau` beside `bar.aegle` runs while the document is
//! open. Scripts share states by name with every document, define host
//! actions as top-level functions, and run commands, timers and file
//! watches that end when the script unloads. See `docs/scripts.md`.

mod api;
pub(crate) mod convert;
mod tasks;

use aegle_loader::{Data, markup::Type};
use aegle_ui::Result;
use mlua::{Function, Lua, MultiValue, Table, Value};
use palettes_shell::{Sender, Shell};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    path::{Path, PathBuf},
};
use tasks::{Event, Message, Task};

struct Callback {
    function: Function,
    /// Called per line; the final report only ends it.
    lines: bool,
    task: Task,
}

struct Script {
    id: u64,
    path: PathBuf,
    /// The script's globals: what it defines, over the built-ins.
    env: Table,
    callbacks: HashMap<u64, Callback>,
    watchers: Vec<(String, Function)>,
    files: Vec<(PathBuf, Function)>,
}

/// A state as scripts see it.
struct Shared {
    value: Value,
    /// The value last reported or assigned, to skip repeats.
    data: Option<Data>,
    /// Set by a script: documents opened later take it too.
    owned: bool,
}

struct Runtime {
    lua: Lua,
    sender: Sender<Message>,
    scripts: Vec<Script>,
    states: HashMap<String, Shared>,
}

thread_local! {
    static RUNTIME: RefCell<Option<Runtime>> = const { RefCell::new(None) };
    /// The state a script is assigning, whose reports from views are its own.
    static ASSIGNING: RefCell<Option<String>> = const { RefCell::new(None) };
    static NEXT: Cell<u64> = const { Cell::new(1) };
}

fn with<T>(f: impl FnOnce(&mut Runtime) -> T) -> Option<T> {
    RUNTIME.with_borrow_mut(|r| r.as_mut().map(f))
}

fn lua() -> Option<Lua> {
    with(|r| r.lua.clone())
}

fn next_id() -> u64 {
    NEXT.with(|n| {
        let id = n.get();
        n.set(id + 1);
        id
    })
}

/// Starts the runtime for this thread's documents.
pub(crate) fn start(shell: &Shell) -> Result<()> {
    if with(|_| ()).is_some() {
        return Ok(());
    }
    let lua = Lua::new();
    let sender = shell.channel(|_, message: Message| dispatch(message));
    RUNTIME.with_borrow_mut(|slot| {
        *slot = Some(Runtime {
            lua,
            sender,
            scripts: Vec::new(),
            states: HashMap::new(),
        })
    });
    Ok(())
}

/// The script that runs with the document at `document`, if there is one.
pub(crate) fn beside(document: &Path) -> Option<PathBuf> {
    Some(document.with_extension("luau")).filter(|p| p.is_file())
}

/// Compiles a script without running it.
pub(crate) fn check(path: &Path) -> Result<()> {
    let source = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    Lua::new()
        .load(source)
        .set_name(format!("@{}", path.display()))
        .into_function()
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Runs the script at `path`, replacing a running copy. A script that does
/// not compile leaves the running copy alone.
pub(crate) fn load(path: &Path) -> Result<()> {
    let lua = lua().ok_or("no script runtime")?;
    let source = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let id = next_id();
    let env = api::environment(&lua, id, path).map_err(|e| e.to_string())?;
    let chunk = lua
        .load(source)
        .set_name(format!("@{}", path.display()))
        .set_environment(env.clone())
        .into_function()
        .map_err(|e| e.to_string())?;
    unload(path);
    with(|r| {
        r.scripts.push(Script {
            id,
            path: path.to_owned(),
            env,
            callbacks: HashMap::new(),
            watchers: Vec::new(),
            files: Vec::new(),
        })
    });
    // An error in the top-level code keeps what it set up so far.
    chunk.call::<()>(()).map_err(|e| e.to_string())?;
    Ok(())
}

/// Stops the script at `path`: its commands, timers and watches end.
pub(crate) fn unload(path: &Path) {
    let removed = with(|r| {
        let index = r.scripts.iter().position(|s| s.path == path)?;
        Some(r.scripts.remove(index))
    });
    // Dropped outside the borrow: tasks stop as they drop.
    drop(removed);
}

/// Whether `path` is a running script.
pub(crate) fn is_script(path: &Path) -> bool {
    with(|r| r.scripts.iter().any(|s| s.path == path)).unwrap_or(false)
}

fn report(path: &Path, error: &mlua::Error) {
    eprintln!("palettes: {}: {error}", path.display());
}

fn script_path(id: u64) -> PathBuf {
    with(|r| {
        r.scripts
            .iter()
            .find(|s| s.id == id)
            .map(|s| s.path.clone())
    })
    .flatten()
    .unwrap_or_default()
}

fn dispatch(message: Message) {
    let Some(Some((function, lines))) = with(|r| {
        let script = r.scripts.iter_mut().find(|s| s.id == message.script)?;
        let callback = script.callbacks.get(&message.callback)?;
        let done = match message.event {
            Event::Output(..) => true,
            Event::Line(_) => false,
            Event::Tick => !callback.task.running(),
        };
        let found = (callback.function.clone(), callback.lines);
        if done {
            script.callbacks.remove(&message.callback);
        }
        Some(found)
    }) else {
        return;
    };
    let result = match message.event {
        Event::Output(_, _) if lines => Ok(()),
        Event::Output(text, status) => function.call::<()>((text, status)),
        Event::Line(line) => function.call::<()>(line),
        Event::Tick => function.call::<()>(()),
    };
    if let Err(e) = result {
        report(&script_path(message.script), &e);
    }
}

/// Calls each script's watch of a changed file with its new text.
pub(crate) fn file_changed(path: &Path) {
    let watching: Vec<(PathBuf, Function)> = with(|r| {
        r.scripts
            .iter()
            .flat_map(|s| {
                s.files
                    .iter()
                    .filter(|(p, _)| p == path)
                    .map(|(_, f)| (s.path.clone(), f.clone()))
            })
            .collect()
    })
    .unwrap_or_default();
    if watching.is_empty() {
        return;
    }
    let text = std::fs::read_to_string(path).ok();
    for (script, function) in watching {
        if let Err(e) = function.call::<()>(text.clone()) {
            report(&script, &e);
        }
    }
}

/// The function a script defines as `name`, the first script first.
fn function(name: &str) -> Option<(PathBuf, Function)> {
    with(|r| {
        r.scripts
            .iter()
            .find_map(|s| match s.env.raw_get::<Value>(name) {
                Ok(Value::Function(f)) => Some((s.path.clone(), f)),
                _ => None,
            })
    })
    .flatten()
}

/// Whether a running script defines a host action `name`.
pub(crate) fn defines(name: &str) -> bool {
    function(name).is_some()
}

/// Runs the script function `name` with arguments from markup.
pub(crate) fn call_data(name: &str, arguments: &[Data], types: &[Type]) -> Result<()> {
    let (path, function) = function(name).ok_or_else(|| format!("no script defines `{name}`"))?;
    let lua = lua().ok_or("no script runtime")?;
    let values = arguments
        .iter()
        .zip(types)
        .map(|(data, ty)| convert::to_lua(&lua, data, ty))
        .collect::<mlua::Result<MultiValue>>()
        .map_err(|e| e.to_string())?;
    function
        .call::<()>(values)
        .map_err(|e| format!("{}: {e}", path.display()).into())
}

/// Runs the script function `name` with command-line arguments; its result
/// as text.
pub(crate) fn call_text(name: &str, arguments: &[String]) -> Result<String> {
    let (path, function) = function(name).ok_or_else(|| format!("no action `{name}`"))?;
    let lua = lua().ok_or("no script runtime")?;
    let values = arguments
        .iter()
        .map(|a| convert::guess(&lua, a))
        .collect::<mlua::Result<MultiValue>>()
        .map_err(|e| e.to_string())?;
    let result: Value = function
        .call(values)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    api::text(&lua, result).map_err(|e| e.to_string().into())
}

/// A script-set value for a state of type `ty` in a view being linked.
pub(crate) fn shared(name: &str, ty: &Type) -> Option<Data> {
    let value = with(|r| {
        let shared = r.states.get(name).filter(|s| s.owned)?;
        Some(shared.value.clone())
    })??;
    convert::to_data(&value, ty).ok()
}

/// A state a view declares, so scripts can read it before it changes.
pub(crate) fn initial(name: &str, ty: &Type, data: &Data) {
    let Some(lua) = lua() else { return };
    with(|r| {
        if r.states.contains_key(name) {
            return;
        }
        if let Ok(value) = convert::to_lua(&lua, data, ty) {
            let data = Some(data.clone());
            let owned = false;
            r.states
                .insert(name.to_owned(), Shared { value, data, owned });
        }
    });
}

/// A view's state changed; scripts see the value and their watches run.
pub(crate) fn changed(name: &str, ty: &Type, data: &Data) {
    let own = ASSIGNING.with_borrow(|a| a.as_deref() == Some(name));
    let Some(lua) = lua() else { return };
    let Some(watchers) = with(|r| {
        if r.states.get(name).and_then(|s| s.data.as_ref()) == Some(data) {
            return None;
        }
        let value = convert::to_lua(&lua, data, ty).ok()?;
        let owned = r.states.get(name).is_some_and(|s| s.owned);
        r.states.insert(
            name.to_owned(),
            Shared {
                value: value.clone(),
                data: Some(data.clone()),
                owned: owned && own,
            },
        );
        (!own).then(|| watchers(r, name, value))
    })
    .flatten() else {
        return;
    };
    notify(watchers);
}

type Watchers = Vec<(PathBuf, Function, Value)>;

fn watchers(r: &Runtime, name: &str, value: Value) -> Watchers {
    r.scripts
        .iter()
        .flat_map(|s| {
            s.watchers
                .iter()
                .filter(|(n, _)| n == name)
                .map(|(_, f)| (s.path.clone(), f.clone(), value.clone()))
        })
        .collect()
}

fn notify(watchers: Watchers) {
    for (path, function, value) in watchers {
        if let Err(e) = function.call::<()>(value) {
            report(&path, &e);
        }
    }
}

/// `state.name` in a script.
fn get(name: &str) -> Value {
    with(|r| r.states.get(name).map(|s| s.value.clone()))
        .flatten()
        .unwrap_or(Value::Nil)
}

/// `state.name = value` in a script: every document declaring the state
/// shows it, then watches run.
fn set(name: &str, value: Value) -> mlua::Result<()> {
    with(|r| {
        r.states.insert(
            name.to_owned(),
            Shared {
                value: value.clone(),
                data: None,
                owned: true,
            },
        )
    });
    ASSIGNING.with_borrow_mut(|a| *a = Some(name.to_owned()));
    let assigned = crate::markup::link::assign(name, |ty| convert::to_data(&value, ty));
    ASSIGNING.with_borrow_mut(|a| *a = None);
    // Reports during the assignment recorded the value but not ownership.
    with(|r| {
        if let Some(shared) = r.states.get_mut(name) {
            shared.owned = true;
            shared.value = value.clone();
        }
    });
    assigned.map_err(|e| mlua::Error::runtime(format!("state `{name}`: {e}")))?;
    if let Some(watchers) = with(|r| watchers(r, name, value)) {
        notify(watchers);
    }
    Ok(())
}
