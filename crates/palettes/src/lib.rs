//! Palettes: a modular Wayland desktop shell and widget library on aegle,
//! drawn with Vulkan.
//!
//! In Rust, open panels on a [`Shell`] and follow services through
//! [`Services`]:
//!
//! ```no_run
//! use palettes::prelude::*;
//!
//! fn main() -> Result<()> {
//!     let shell = Shell::new()?;
//!     let bar = shell.panel(PanelOptions::top(32))?;
//!     let time = bar.text("");
//!     shell.clock(&time, "%H:%M", |time, now| time.set_text(now))?;
//!     shell.run()
//! }
//! ```
//!
//! In markup, a document whose root is a `Panel` declares the shell states it
//! shows; [`run`] opens it on every output and reloads it on save:
//!
//! ```text
//! use "palettes.aegle"
//! Panel {
//!     edge: top
//!     size: 32dp
//!     state time: string = ""
//!     Text { text: time }
//! }
//! ```

pub mod markup;
mod services;

pub use aegle_loader as loader;
pub use aegle_ui as ui;
pub use aegle_widgets as widgets;
pub use palettes_assets as assets;
pub use palettes_audio::{AudioState, Endpoint, Target};
pub use palettes_shell::{
    Anchor, KeyboardInteractivity, Layer, OutputId, OutputInfo, PanelOptions, PopupOptions, Result,
    Sender, Shell, ShellOptions, Side, Surface,
};
pub use palettes_workspace::{Desktop, Window, Workspace};
pub use services::{Services, applications, icons, launch, spawn};

/// Everything a shell usually needs, in one `use`.
pub mod prelude {
    pub use crate::{
        Desktop, PanelOptions, PopupOptions, Result, Services, Shell, Surface,
        ui::{Container, Node},
        widgets::Widgets,
    };
}

/// The local time now, formatted with a `strftime` pattern.
pub fn format_time(pattern: &str) -> String {
    palettes_clock::format(pattern, std::time::SystemTime::now())
}

/// Runs a panel document on every output until it quits; the whole of the
/// `palettes` command for one file.
pub fn run(path: impl AsRef<std::path::Path>) -> Result<()> {
    markup::Documents::new()?.open(path)?.run()
}
