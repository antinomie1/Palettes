//! The Palettes host: a [`Shell`] owns one Wayland connection
//! (`palettes-wayland`), and each of its [`Surface`]s (panels and popups) owns
//! an aegle [`Ui`](aegle_ui::Ui) tree drawn by aegle's Vulkan renderer on a
//! device all surfaces share.
//!
//! Panels are placed with [`PanelOptions`] and can follow outputs with
//! [`Shell::each_output`]; popups attach to any surface with
//! [`Surface::popup`]. Other threads reach the shell through
//! [`Shell::channel`]. Nothing runs while nothing changes: the loop blocks
//! until input, a frame callback, a channel value or a control's own timer.

mod channel;
mod input;
mod options;
mod runtime;
mod shape;
mod shell;
mod surface;

pub use aegle_platform::{Anchor, KeyboardInteractivity, Layer, OutputId, OutputInfo, Side};
pub use aegle_render_vulkan::SharedDevice;
pub use aegle_ui as ui;
pub use aegle_ui::Result;
pub use channel::Sender;
pub use options::{PanelOptions, PopupOptions, ShellOptions};
pub use shell::Shell;
pub use surface::Surface;
