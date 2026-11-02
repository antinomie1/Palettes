//! A bar on every output with a button that opens a popup.
use aegle_widgets::Widgets;
use palettes_shell::{PanelOptions, Shell};

fn main() -> palettes_shell::Result<()> {
    let shell = Shell::new()?;
    shell.each_output(|shell, output| {
        let bar = shell.panel(PanelOptions::top(32).on(output.id))?;
        let row = bar.row();
        row.text(&format!("Palettes on {} ({}×)", output.name, output.scale));
        let open = row.button("Menu");
        let surface = bar.clone();
        open.on_click(move |button| {
            let menu = surface.popup_below(&button, 200, 120)?;
            menu.text("A popup");
            Ok(())
        });
        Ok(bar)
    })?;
    if let Ok(ms) = std::env::var("PALETTES_EXIT_MS") {
        let shell2 = shell.clone();
        let quit = shell.channel(move |_, ()| shell2.quit());
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(ms.parse().unwrap_or(1000)));
            quit.send(());
        });
    }
    shell.run()
}
