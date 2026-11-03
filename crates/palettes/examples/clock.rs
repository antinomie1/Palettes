//! The smallest panel: a clock bar. `cargo run -p palettes --example clock`
use palettes::prelude::*;

fn main() -> Result<()> {
    let shell = Shell::new()?;
    let bar = shell.panel(PanelOptions::top(32))?;
    let time = bar.text("");
    shell.clock(&time, "%H:%M:%S", |time, now| time.set_text(now))?;
    shell.run()
}
