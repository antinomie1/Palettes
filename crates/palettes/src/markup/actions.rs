//! Host actions: what `host.<name>(...)` in an `on` block does.

use crate::services::{self, Services, audio_target};
use aegle_loader::{Data, action, markup::Type};
use aegle_ui::Result;
use palettes_shell::Shell;

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
    action("toggle", &[Type::String], |a| super::toggle(text(a)));
    action("reload", &[], |_| super::reload_all());
    let quit = shell.clone();
    action("quit", &[], move |_| quit.quit());
}

/// Registers every action as doing nothing, so documents check without a shell.
pub(crate) fn declare() {
    for (name, params) in [
        ("spawn", &[Type::String][..]),
        ("launch", &[Type::String]),
        ("focus_workspace", &[Type::Int]),
        ("focus_window", &[Type::Int]),
        ("close_window", &[Type::Int]),
        ("compositor", &[Type::String]),
        ("set_volume", &[Type::Float]),
        ("toggle_mute", &[]),
        ("set_mic_volume", &[Type::Float]),
        ("toggle_mic_mute", &[]),
        ("toggle", &[Type::String]),
        ("reload", &[]),
        ("quit", &[]),
    ] {
        action(name, params, |_| ());
    }
}
