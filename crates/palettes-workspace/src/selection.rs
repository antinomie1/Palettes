//! The window each output has selected, as tiling window managers mark it:
//! the focused window on the focused output, elsewhere the window last
//! focused on the output's shown workspace.

use crate::{Desktop, Window};
use std::collections::HashMap;

/// Remembers the last focused window of every workspace.
#[derive(Clone, Debug, Default)]
pub struct Selection {
    last: HashMap<u64, u64>,
}

impl Selection {
    /// Takes in a new desktop: records its focused window and forgets
    /// workspaces that are gone.
    pub fn update(&mut self, desktop: &Desktop) {
        if let Some(window) = desktop.focused_window() {
            self.last.insert(window.workspace, window.id);
        }
        self.last
            .retain(|workspace, _| desktop.workspaces.iter().any(|w| w.id == *workspace));
    }

    /// The selected window of `output`: the one last focused on its shown
    /// workspace, else that workspace's first window. Where windows carry no
    /// workspace (the Wayland protocols), only the focused window is selected,
    /// on every output.
    pub fn on<'a>(&self, desktop: &'a Desktop, output: &str) -> Option<&'a Window> {
        if desktop.windows.iter().any(|w| w.workspace == 0) {
            return desktop.focused_window();
        }
        let shown = desktop.workspaces_on(output).find(|w| w.active)?;
        let mut windows = desktop.windows.iter().filter(|w| w.workspace == shown.id);
        let last = self.last.get(&shown.id);
        windows
            .clone()
            .find(|w| Some(&w.id) == last)
            .or_else(|| windows.next())
    }
}
