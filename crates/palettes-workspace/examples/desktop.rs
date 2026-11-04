//! Prints the desktop after each change.
//! `cargo run -p palettes-workspace --example desktop [--wayland] [--focus N]`:
//! `--wayland` uses the Wayland protocols even under Pastel; `--focus N` then shows the
//! workspace at position N of the first output.
use palettes_workspace::Compositor;
use std::{sync::mpsc, time::Duration};

fn main() -> std::io::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let compositor = if args.iter().any(|a| a == "--wayland") {
        Compositor::connect_wayland()?
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
        for w in &desktop.windows {
            let focused = if w.focused { " (focused)" } else { "" };
            println!("  window {} {:?} {:?}{focused}", w.id, w.app_id, w.title);
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
