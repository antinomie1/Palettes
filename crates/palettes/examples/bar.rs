//! The markup example bar in Rust: on every output, its workspaces, the focused
//! window, volume and a clock, with a popup menu.
//! `cargo run -p palettes --example bar`
use palettes::{prelude::*, widgets::Button};

fn main() -> Result<()> {
    let shell = Shell::new()?;
    shell.each_output(|shell, output| {
        let bar = shell.panel(PanelOptions::top(34).on(output.id))?;
        let row = bar.row();
        row.set_gap(8.0, 8.0);

        // Workspaces of this output, rebuilt when they change.
        let workspaces = row.row();
        let (name, s) = (output.name.clone(), shell.clone());
        let followed = shell.desktop(&workspaces, move |workspaces, desktop| {
            workspaces
                .children()
                .iter()
                .for_each(|child| child.remove());
            for ws in desktop.workspaces_on(&name) {
                let label = if ws.active {
                    format!("● {}", ws.name)
                } else {
                    ws.name.clone()
                };
                let (id, s) = (ws.id, s.clone());
                workspaces
                    .button(&label)
                    .on_click(move |_: Button| s.compositor().map(|c| c.focus_workspace(id)));
            }
        });
        if let Err(e) = followed {
            eprintln!("no workspaces: {e}");
        }

        let title = row.text("");
        title.set_grow(1.0);
        let _ = shell.desktop(&title, |title, desktop| {
            title.set_text(desktop.focused_window().map_or("", |w| &w.title));
        });

        let volume = row.button("");
        let _ = shell.audio(&volume, |volume, audio| {
            let text = audio
                .sink
                .as_ref()
                .map(|s| format!("{:.0}%", s.volume * 100.0));
            volume.set_text(&text.unwrap_or_default());
        });
        let mixer = shell.clone();
        volume.on_click(move |_| mixer.mixer().map(|m| m.toggle_mute(palettes::Target::Sink)));

        let clock = row.button("");
        shell.clock(&clock, "%a %d %b  %H:%M", |clock, now| clock.set_text(now))?;
        let surface = bar.clone();
        clock.on_click(move |clock| {
            let calendar = surface.popup_below(&clock, 220, 80)?;
            calendar.text(&palettes::format_time("%A, %d %B %Y"));
            Ok(())
        });
        Ok(bar)
    })?;
    shell.run()
}
