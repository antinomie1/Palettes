//! Host actions: what `host.<name>(...)` does in an `on` block, in a
//! script, and from `palettes msg`. Scripts add their own functions.

use crate::services::{self, Services, audio_target};
use aegle_loader::{Data, markup::Type};
use aegle_ui::Result;
use palettes_shell::Shell;
use std::{cell::RefCell, collections::HashMap, rc::Rc};

type Run = Rc<dyn Fn(&[Data]) -> Result<()>>;

thread_local! {
    static ACTIONS: RefCell<HashMap<String, (Vec<Type>, Run)>> = RefCell::default();
}

/// Registers a Palettes action for markup, scripts and `palettes msg`.
pub(crate) fn action(name: &str, params: &[Type], run: impl Fn(&[Data]) -> Result<()> + 'static) {
    let run: Run = Rc::new(run);
    let markup = run.clone();
    aegle_loader::action(name, params, move |a: &[Data]| markup(a));
    ACTIONS.with_borrow_mut(|actions| actions.insert(name.to_owned(), (params.to_vec(), run)));
}

/// The argument types of the Palettes action `name`.
pub(crate) fn params(name: &str) -> Option<Vec<Type>> {
    ACTIONS.with_borrow(|actions| actions.get(name).map(|(params, _)| params.clone()))
}

/// Runs the Palettes action `name`.
pub(crate) fn call(name: &str, arguments: &[Data]) -> Result<()> {
    let run = ACTIONS
        .with_borrow(|actions| actions.get(name).map(|(_, run)| run.clone()))
        .ok_or_else(|| format!("no action `{name}`"))?;
    run(arguments)
}

/// Runs the action `name` with command-line arguments: a Palettes action
/// parses them by its types, a script function gets numbers, booleans and
/// strings. Returns what a script function returned, as text.
pub(crate) fn call_text(name: &str, arguments: &[String]) -> Result<String> {
    let Some(params) = params(name) else {
        return crate::script::call_text(name, arguments);
    };
    if arguments.len() != params.len() {
        return Err(format!("`{name}` takes {} arguments", params.len()).into());
    }
    let data = arguments
        .iter()
        .zip(&params)
        .map(|(text, ty)| crate::script::convert::parse(text, ty))
        .collect::<std::result::Result<Vec<_>, _>>()?;
    call(name, &data)?;
    Ok(String::new())
}

/// Lets markup call the functions scripts define: each `host.name(...)` that
/// is no Palettes action calls the script function `name`, with the
/// argument types of the call. In `check`, every such call is accepted.
pub(crate) fn scripted(calls: &[aegle_loader::markup::HostCall], check: bool) {
    for call in calls {
        if params(&call.name).is_some() || !(check || crate::script::defines(&call.name)) {
            continue;
        }
        let (name, types) = (call.name.clone(), call.types.clone());
        aegle_loader::action(&call.name, &call.types, move |a: &[Data]| {
            if check {
                Ok(())
            } else {
                crate::script::call_data(&name, a, &types)
            }
        });
    }
}

fn text(arguments: &[Data]) -> &str {
    match arguments.first() {
        Some(Data::String(text)) => text,
        _ => "",
    }
}

fn int(arguments: &[Data]) -> u64 {
    match arguments.first() {
        Some(Data::Int(n)) => (*n).max(0) as u64,
        _ => 0,
    }
}

fn float(arguments: &[Data]) -> f32 {
    match arguments.first() {
        Some(Data::Float(n)) => *n,
        _ => 0.0,
    }
}

/// Registers every Palettes action for this thread's programs.
pub(crate) fn register(shell: &Shell) {
    action("spawn", &[Type::String], |a| services::spawn(text(a)));
    action("open", &[Type::String], |a| super::open_named(text(a)));
    action("close", &[Type::String], |a| super::close_named(text(a)));
    action("launch", &[Type::String], |a| services::launch(text(a)));
    let compositor = |shell: &Shell, f: fn(&palettes_workspace::Compositor, &[Data])| {
        let shell = shell.clone();
        move |a: &[Data]| -> Result<()> {
            f(&*shell.compositor()?, a);
            Ok(())
        }
    };
    action(
        "focus_workspace",
        &[Type::Int],
        compositor(shell, |c, a| c.focus_workspace(int(a))),
    );
    action(
        "focus_window",
        &[Type::Int],
        compositor(shell, |c, a| c.focus_window(int(a))),
    );
    action(
        "close_window",
        &[Type::Int],
        compositor(shell, |c, a| c.close_window(int(a))),
    );
    action(
        "compositor",
        &[Type::String],
        compositor(shell, |c, a| c.action(text(a))),
    );
    let mixer = |shell: &Shell, f: fn(&palettes_audio::Audio, &[Data])| {
        let shell = shell.clone();
        move |a: &[Data]| -> Result<()> {
            f(&*shell.mixer()?, a);
            Ok(())
        }
    };
    action(
        "set_volume",
        &[Type::Float],
        mixer(shell, |m, a| m.set_volume(audio_target(true), float(a))),
    );
    action(
        "toggle_mute",
        &[],
        mixer(shell, |m, _| m.toggle_mute(audio_target(true))),
    );
    action(
        "set_mic_volume",
        &[Type::Float],
        mixer(shell, |m, a| m.set_volume(audio_target(false), float(a))),
    );
    action(
        "toggle_mic_mute",
        &[],
        mixer(shell, |m, _| m.toggle_mute(audio_target(false))),
    );
    for (name, sink) in [("set_default_sink", true), ("set_default_source", false)] {
        let shell = shell.clone();
        action(name, &[Type::String], move |a| {
            shell.mixer()?.set_default(audio_target(sink), text(a));
            Ok(())
        });
    }
    action(
        "set_stream_volume",
        &[Type::Int, Type::Float],
        mixer(shell, |m, a| {
            let level = match a.get(1) {
                Some(Data::Float(n)) => *n,
                _ => 0.0,
            };
            m.set_stream_volume(int(a) as u32, level)
        }),
    );
    action(
        "toggle_stream_mute",
        &[Type::Int],
        mixer(shell, |m, a| m.set_stream_muted(int(a) as u32, None)),
    );
    super::system::actions(Some(shell));
    super::lock::register(Some(shell));
    action("toggle", &[Type::String], |a| super::toggle(text(a)));
    action("reload", &[], |_| super::reload_all());
    let quit = shell.clone();
    action("quit", &[], move |_| {
        quit.quit();
        Ok(())
    });
}

/// Registers every action as doing nothing, so documents check without a shell.
pub(crate) fn declare() {
    for (name, params) in [
        ("spawn", &[Type::String][..]),
        ("open", &[Type::String]),
        ("close", &[Type::String]),
        ("launch", &[Type::String]),
        ("focus_workspace", &[Type::Int]),
        ("focus_window", &[Type::Int]),
        ("close_window", &[Type::Int]),
        ("compositor", &[Type::String]),
        ("set_volume", &[Type::Float]),
        ("toggle_mute", &[]),
        ("set_mic_volume", &[Type::Float]),
        ("toggle_mic_mute", &[]),
        ("set_default_sink", &[Type::String]),
        ("set_default_source", &[Type::String]),
        ("set_stream_volume", &[Type::Int, Type::Float]),
        ("toggle_stream_mute", &[Type::Int]),
        ("toggle", &[Type::String]),
        ("reload", &[]),
        ("quit", &[]),
    ] {
        action(name, params, |_| Ok(()));
    }
    super::system::actions(None);
    super::lock::register(None);
}
