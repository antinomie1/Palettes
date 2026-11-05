//! Pastel through `pastel-ipc`.

use crate::{Colors, Command, Desktop, Output, Window, Workspace};
use pastel_ipc::{Client, Reply, Request};
use std::{io, path::PathBuf, sync::mpsc};

fn socket() -> io::Result<PathBuf> {
    pastel_ipc::socket_path()
        .filter(|path| path.exists())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "no compositor IPC (Pastel) in this session",
            )
        })
}

/// The neutral model of Pastel's state.
pub fn desktop(state: pastel_ipc::State) -> Desktop {
    Desktop {
        outputs: state
            .outputs
            .into_iter()
            .map(|o| Output {
                name: o.name,
                focused: o.focused,
            })
            .collect(),
        workspaces: state
            .workspaces
            .into_iter()
            .map(|w| Workspace {
                id: w.id,
                name: w.index.to_string(),
                index: w.index,
                output: w.output,
                active: w.active,
                focused: w.focused,
                windows: w.windows,
                layout: w.layout,
                columns: w.columns,
                column: w.column,
                maximized: w.maximized,
            })
            .collect(),
        windows: state
            .windows
            .into_iter()
            .map(|w| Window {
                id: w.id,
                title: w.title,
                app_id: w.app_id,
                icon: w.icon,
                workspace: w.workspace,
                focused: w.focused,
                floating: w.floating,
                fullscreen: w.fullscreen,
            })
            .collect(),
        keyboard_layout: state.keyboard_layout,
        colors: Colors {
            background: state.colors.background,
            active: state.colors.active,
            inactive: state.colors.inactive,
            shadow: state.colors.shadow,
        },
    }
}

pub fn subscribe(mut changed: impl FnMut(Desktop) + Send + 'static) -> io::Result<()> {
    let states = Client::connect_to(&socket()?)?.subscribe()?;
    std::thread::Builder::new()
        .name("palettes-workspace".into())
        .spawn(move || {
            for state in states {
                match state {
                    Ok(state) => changed(desktop(state)),
                    Err(e) => {
                        eprintln!("palettes: compositor IPC: {e}");
                        break;
                    }
                }
            }
            changed(Desktop::default());
        })?;
    Ok(())
}

fn request(command: Command) -> Request {
    match command {
        Command::FocusWorkspace(id) => Request::FocusWorkspace(id),
        Command::FocusWindow(id) => Request::FocusWindow(id),
        Command::CloseWindow(id) => Request::CloseWindow(id),
        Command::Action(action) => Request::Action(action),
    }
}

/// A thread that sends queued commands over one connection, reconnecting once
/// if Pastel dropped it.
pub fn commands() -> io::Result<mpsc::Sender<Command>> {
    let path = socket()?;
    let (send, receive) = mpsc::channel::<Command>();
    std::thread::Builder::new()
        .name("palettes-commands".into())
        .spawn(move || {
            let mut client = None;
            for command in receive {
                let request = request(command);
                let mut reply = Err(io::Error::from(io::ErrorKind::NotConnected));
                for _ in 0..2 {
                    if client.is_none() {
                        client = Client::connect_to(&path).ok();
                    }
                    let Some(connected) = client.as_mut() else {
                        break;
                    };
                    reply = connected.request(&request);
                    if reply.is_ok() {
                        break;
                    }
                    client = None;
                }
                match reply {
                    Ok(Reply::Error(e)) => eprintln!("palettes: compositor: {e}"),
                    Err(e) => eprintln!("palettes: compositor IPC: {e}"),
                    Ok(_) => {}
                }
            }
        })?;
    Ok(send)
}
