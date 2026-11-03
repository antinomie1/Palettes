//! Shell states: a document declares a state with a known name and type, and
//! Palettes keeps it current. Services start only for states some document
//! declares. The record types are in the embedded `palettes.aegle`.

use crate::services;
use aegle_loader::{Data, View};
use aegle_ui::{Node, Result};
use palettes_audio::AudioState;
use palettes_shell::Shell;
use palettes_workspace::Desktop;
use std::{cell::RefCell, rc::Rc};

/// The record types and documentation of every shell state, imported with
/// `use "palettes.aegle"`.
pub const PRELUDE: &str = include_str!("prelude.aegle");

fn string(text: &str) -> Data {
    Data::String(text.into())
}

fn list(items: impl Iterator<Item = Data>) -> Data {
    Data::List(items.collect())
}

fn record(fields: impl IntoIterator<Item = Data>) -> Data {
    Data::Record(fields.into_iter().collect())
}

fn workspace(w: &palettes_workspace::Workspace) -> Data {
    record([
        Data::Int(w.id as i64),
        string(&w.name),
        Data::Int(i64::from(w.index)),
        string(&w.output),
        Data::Bool(w.active),
        Data::Bool(w.focused),
        Data::Int(i64::from(w.windows)),
    ])
}

fn icon_of(app_id: &str) -> String {
    services::icons()
        .for_app(app_id, 64)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn window(w: &palettes_workspace::Window) -> Data {
    record([
        Data::Int(w.id as i64),
        string(&w.title),
        string(&w.app_id),
        string(&icon_of(&w.app_id)),
        Data::Int(w.workspace as i64),
        Data::Bool(w.focused),
    ])
}

/// Sets `name` if the view declares it; a type mismatch is reported once.
fn set(view: &View, name: &str, value: Data) {
    if view.get(name).is_none() {
        return;
    }
    if let Err(e) = view.set(name, value) {
        eprintln!("palettes: state `{name}`: {e}; see palettes.aegle for its type");
    }
}

fn declares(view: &View, names: &[&str]) -> bool {
    names.iter().any(|name| view.get(name).is_some())
}

fn text_state(view: &View, name: &str, default: &str) -> String {
    match view.get(name) {
        Some(Data::String(text)) if !text.is_empty() => text.to_string(),
        _ => default.to_owned(),
    }
}

const DESKTOP: &[&str] = &[
    "workspaces",
    "output_workspaces",
    "windows",
    "focused_title",
    "focused_app",
    "focused_icon",
];
const AUDIO: &[&str] = &["volume", "muted", "mic_volume", "mic_muted", "audio_device"];

fn desktop(view: &View, output: &str, desktop: &Desktop) {
    set(
        view,
        "workspaces",
        list(desktop.workspaces.iter().map(workspace)),
    );
    set(
        view,
        "output_workspaces",
        list(desktop.workspaces_on(output).map(workspace)),
    );
    if view.get("windows").is_some() {
        set(view, "windows", list(desktop.windows.iter().map(window)));
    }
    let focused = desktop.focused_window();
    let field = |f: fn(&palettes_workspace::Window) -> String| focused.map(f).unwrap_or_default();
    set(view, "focused_title", string(&field(|w| w.title.clone())));
    set(view, "focused_app", string(&field(|w| w.app_id.clone())));
    if view.get("focused_icon").is_some() {
        set(view, "focused_icon", string(&field(|w| icon_of(&w.app_id))));
    }
}

fn audio(view: &View, audio: &AudioState) {
    let sink = audio.sink.as_ref();
    let source = audio.source.as_ref();
    set(view, "volume", Data::Float(sink.map_or(0.0, |e| e.volume)));
    set(view, "muted", Data::Bool(sink.is_some_and(|e| e.muted)));
    set(
        view,
        "mic_volume",
        Data::Float(source.map_or(0.0, |e| e.volume)),
    );
    set(
        view,
        "mic_muted",
        Data::Bool(source.is_some_and(|e| e.muted)),
    );
    set(
        view,
        "audio_device",
        string(sink.map_or("", |e| &e.description)),
    );
}

/// Fills every shell state `view` declares and keeps it current while
/// `owner` (the view's root) lives. `output` names the panel's output.
pub fn bind(shell: &Shell, shared: &Rc<RefCell<View>>, owner: &Node, output: &str) -> Result<()> {
    let view = &*shared.borrow();
    set(view, "output", string(output));
    for (name, default_format) in [("time", "%H:%M"), ("date", "%a %d %b")] {
        if view.get(name).is_none() {
            continue;
        }
        let format = text_state(view, &format!("{name}_format"), default_format);
        let target = shared.clone();
        services::follow_clock(shell, owner, &format, move |text| {
            set(&target.borrow(), name, string(text))
        })?;
    }
    if declares(view, DESKTOP) {
        let (target, output) = (shared.clone(), output.to_owned());
        let followed =
            services::follow_desktop(shell, owner, move |d| desktop(&target.borrow(), &output, d));
        report(followed, "compositor");
    }
    if declares(view, AUDIO) {
        let target = shared.clone();
        report(
            services::follow_audio(shell, owner, move |a| audio(&target.borrow(), a)),
            "audio",
        );
    }
    if view.get("applications").is_some() {
        let apps = services::applications();
        let items = apps.launchable().into_iter().map(|e| {
            let icon = services::icons().lookup(&e.icon, 64);
            record([
                string(&e.id),
                string(&e.name),
                string(
                    &icon
                        .map(|p| p.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                ),
            ])
        });
        set(view, "applications", list(items));
    }
    Ok(())
}

/// A missing service leaves its states at their defaults; the panel still opens.
fn report(followed: Result<()>, service: &str) {
    if let Err(e) = followed {
        eprintln!("palettes: {service}: {e}");
    }
}
