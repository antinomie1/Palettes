//! Prints the desktop after each change.
//! `cargo run -p palettes-workspace --example desktop [--ext] [--focus N]`:
//! `--ext` uses ext-workspace-v1 even under Pastel; `--focus N` then shows the
//! workspace at position N of the first output.
use palettes_workspace::Compositor;
use std::{sync::mpsc, time::Duration};

fn main() -> std::io::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let compositor = if args.iter().any(|a| a == "--ext") {
        Compositor::connect_ext_workspace()?
    } else {
        Compositor::connect()?
    };
    println!("source: {:?}", compositor.source());
    let focus: Option<u32> = args
        .iter()
        .position(|a| a == "--focus")
        .and_then(|i| args.get(i + 1)?.parse().ok());
    let (send, desktops) = mpsc::channel();
    compositor.subscribe(move |desktop| drop(send.send(desktop)))?;
    let mut focused = false;
    while let Ok(desktop) = desktops.recv_timeout(Duration::from_millis(800)) {
        for ws in &desktop.workspaces {
            println!(
                "  {} {} on {:?}{}",
                ws.id,
                ws.name,
                ws.output,
                if ws.active { " (active)" } else { "" }
            );
        }
        println!("--");
        if let (Some(index), false) = (focus, focused)
            && let Some(ws) = desktop.workspaces.iter().find(|w| w.index == index)
        {
            compositor.focus_workspace(ws.id);
            focused = true;
        }
    }
    Ok(())
}
