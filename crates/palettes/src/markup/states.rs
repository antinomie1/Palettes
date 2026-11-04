//! Shell states: a document declares a state with a known name and type, and
//! Palettes keeps it current. Services start only for states some document
//! declares. The record types are in the embedded `palettes.aegle`.

use crate::services::{self, Services};
use aegle_loader::{Data, View};
use aegle_ui::{Node, Result};
use palettes_audio::AudioState;
use palettes_shell::Shell;
use palettes_workspace::Desktop;
use std::{cell::RefCell, rc::Rc};

/// The record types and documentation of every shell state, imported with
/// `use "palettes.aegle"`.
pub const PRELUDE: &str = include_str!("prelude.aegle");

pub(super) fn string(text: &str) -> Data {
    Data::String(text.into())
}

pub(super) fn list(items: impl Iterator<Item = Data>) -> Data {
    Data::List(items.collect())
}

pub(super) fn record(fields: impl IntoIterator<Item = Data>) -> Data {
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

/// A window's icon file: its desktop entry's, else the icon it named itself.
fn icon_of(window: &palettes_workspace::Window) -> String {
    let icons = services::icons();
    icons
        .for_app(&window.app_id, 64)
        .or_else(|| icons.lookup(&window.icon, 64))
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn window(w: &palettes_workspace::Window) -> Data {
    record([
        Data::Int(w.id as i64),
        string(&w.title),
        string(&w.app_id),
        string(&icon_of(w)),
        Data::Int(w.workspace as i64),
        Data::Bool(w.focused),
    ])
}

/// Sets `name` if the view declares it; a type mismatch is reported once.
pub(super) fn set(view: &View, name: &str, value: Data) {
    if view.get(name).is_none() {
        return;
    }
    if let Err(e) = view.set(name, value) {
        eprintln!("palettes: state `{name}`: {e}; see palettes.aegle for its type");
    }
}

pub(super) fn declares(view: &View, names: &[&str]) -> bool {
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
    "keyboard_layout",
];
const AUDIO: &[&str] = &[
    "volume",
    "muted",
    "mic_volume",
    "mic_muted",
    "audio_device",
    "audio_sinks",
    "audio_sources",
    "audio_streams",
    "audio_peak",
];

fn device(e: &palettes_audio::Endpoint, default: Option<&palettes_audio::Endpoint>) -> Data {
    record([
        string(&e.name),
        string(&e.description),
        Data::Float(e.volume),
        Data::Bool(e.muted),
        Data::Bool(default.is_some_and(|d| d.name == e.name)),
    ])
}

fn stream(s: &palettes_audio::Stream) -> Data {
    let icons = services::icons();
    let icon = (icons.lookup(&s.icon, 64))
        .or_else(|| icons.for_app(&s.app, 64))
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    record([
        Data::Int(i64::from(s.id)),
        string(&s.app),
        string(&icon),
        string(&s.title),
        Data::Float(s.volume),
        Data::Bool(s.muted),
    ])
}

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
        set(view, "focused_icon", string(&field(icon_of)));
    }
    set(view, "keyboard_layout", string(&desktop.keyboard_layout));
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
    if view.get("audio_sinks").is_some() {
        set(
            view,
            "audio_sinks",
            list(audio.sinks.iter().map(|e| device(e, sink))),
        );
    }
    if view.get("audio_sources").is_some() {
        let sources = audio.sources.iter().map(|e| device(e, source));
        set(view, "audio_sources", list(sources));
    }
    if view.get("audio_streams").is_some() {
        set(
            view,
            "audio_streams",
            list(audio.streams.iter().map(stream)),
        );
    }
    set(view, "audio_peak", Data::Float(audio.peak));
}

/// Fills every shell state `view` declares and keeps it current while
/// `owner` (the view's root) lives. `output` names the panel's output.
pub fn bind(shell: &Shell, shared: &Rc<RefCell<View>>, owner: &Node, output: &str) -> Result<()> {
    let view = &*shared.borrow();
    set(view, "output", string(output));
    if view.get("user").is_some() {
        set(
            view,
            "user",
            string(&palettes_system::user_name().unwrap_or_default()),
        );
    }
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
        if view.get("audio_peak").is_some()
            && let Ok(mixer) = shell.mixer()
        {
            mixer.track_peak();
        }
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
    if view.get("palette").is_some() {
        let target = shared.clone();
        super::image::follow_palette(owner, move |colors| {
            set(
                &target.borrow(),
                "palette",
                list(colors.iter().map(|c| string(c))),
            )
        });
    }
    super::system::bind(shell, shared, owner)
}

/// A missing service leaves its states at their defaults; the panel still opens.
pub(super) fn report(followed: Result<()>, service: &str) {
    if let Err(e) = followed {
        eprintln!("palettes: {service}: {e}");
    }
}
