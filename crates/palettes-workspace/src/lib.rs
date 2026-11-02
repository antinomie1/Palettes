//! Workspaces and windows as a shell shows them, independent of the compositor.
//!
//! [`Compositor::connect`] finds the running compositor's IPC; today that is
//! Pastel's socket. [`Compositor::subscribe`] delivers the whole [`Desktop`] on a
//! background thread at once and after every change; requests such as
//! [`Compositor::focus_workspace`] are queued to another thread and never block
//! the caller.

mod pastel;

use std::{io, sync::mpsc};

/// Everything a shell shows about the desktop.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Desktop {
    /// Monitors.
    pub outputs: Vec<Output>,
    /// Workspaces, grouped by output, in display order.
    pub workspaces: Vec<Workspace>,
    /// Mapped windows, grouped by workspace.
    pub windows: Vec<Window>,
}

/// A monitor.
#[derive(Clone, Debug, PartialEq)]
pub struct Output {
    /// Connector name, such as `DP-1`, matching the `wl_output` name.
    pub name: String,
    /// It has the keyboard.
    pub focused: bool,
}

/// A workspace.
#[derive(Clone, Debug, PartialEq)]
pub struct Workspace {
    /// Stable for the workspace's life.
    pub id: u64,
    /// Display name; its index where the compositor has no names.
    pub name: String,
    /// Position on its output, from 1.
    pub index: u32,
    /// The output it is on.
    pub output: String,
    /// Shown on its output.
    pub active: bool,
    /// Shown on the focused output.
    pub focused: bool,
    /// How many windows it holds.
    pub windows: u32,
}

/// A toplevel window.
#[derive(Clone, Debug, PartialEq)]
pub struct Window {
    /// Stable for the window's life.
    pub id: u64,
    /// Title.
    pub title: String,
    /// Application id, which names its desktop entry and icon.
    pub app_id: String,
    /// The workspace it is on.
    pub workspace: u64,
    /// It has the keyboard.
    pub focused: bool,
    /// Out of the tiling layout.
    pub floating: bool,
    /// Covers its output.
    pub fullscreen: bool,
}

impl Desktop {
    /// The window with the keyboard.
    pub fn focused_window(&self) -> Option<&Window> {
        self.windows.iter().find(|w| w.focused)
    }

    /// The workspace shown on the focused output.
    pub fn focused_workspace(&self) -> Option<&Workspace> {
        self.workspaces.iter().find(|w| w.focused)
    }

    /// The workspaces of one output.
    pub fn workspaces_on<'a>(&'a self, output: &'a str) -> impl Iterator<Item = &'a Workspace> {
        self.workspaces.iter().filter(move |w| w.output == output)
    }
}

/// A request to the compositor.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    /// Show a workspace and focus its output.
    FocusWorkspace(u64),
    /// Focus a window, switching to its workspace.
    FocusWindow(u64),
    /// Ask a window to close.
    CloseWindow(u64),
    /// A compositor-specific action, such as Pastel's `"focus-workspace 2"`.
    Action(String),
}

/// The running compositor's IPC.
pub struct Compositor {
    commands: mpsc::Sender<Command>,
}

impl Compositor {
    /// Connects to the compositor of this session. Fails when it has no IPC
    /// Palettes knows.
    pub fn connect() -> io::Result<Self> {
        let commands = pastel::commands()?;
        Ok(Self { commands })
    }

    /// Calls `changed` on a background thread with the desktop now and after
    /// every change. When the compositor goes away it is called once more with
    /// an empty desktop, and the thread ends.
    pub fn subscribe(&self, changed: impl FnMut(Desktop) + Send + 'static) -> io::Result<()> {
        pastel::subscribe(changed)
    }

    /// Queues a request; failures are reported on stderr.
    pub fn send(&self, command: Command) {
        let _ = self.commands.send(command);
    }

    /// Shows a workspace and focuses its output.
    pub fn focus_workspace(&self, id: u64) {
        self.send(Command::FocusWorkspace(id));
    }

    /// Focuses a window.
    pub fn focus_window(&self, id: u64) {
        self.send(Command::FocusWindow(id));
    }

    /// Asks a window to close.
    pub fn close_window(&self, id: u64) {
        self.send(Command::CloseWindow(id));
    }

    /// Runs a compositor action.
    pub fn action(&self, action: &str) {
        self.send(Command::Action(action.into()));
    }
}
