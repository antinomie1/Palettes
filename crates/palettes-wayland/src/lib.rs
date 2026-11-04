//! A Wayland client written for desktop shells.
//!
//! [`Wayland`] implements aegle's [`Platform`](aegle_platform::Platform) and
//! [`SurfacePlatform`](aegle_platform::SurfacePlatform) for the surfaces a shell
//! needs: wlr layer surfaces on chosen outputs, xdg popups attached to them or
//! to each other, outputs with names and fractional scales, and seat input
//! (pointer, keyboard through xkbcommon with compose sequences and key repeat,
//! touch), input methods through text-input-v3 and the clipboard as text. There are no
//! ordinary toplevel windows and no software buffers: frames are presented by
//! a GPU renderer through [`WindowSurface`] leases, paced by frame callbacks.
//! [`Wayland::set_input`] limits input to a region of a surface, such as its
//! rounded shape; where the compositor offers ext-background-effect-v1,
//! [`Wayland::set_blur`] has it blur what lies behind a region.
//!
//! The loop blocks in `poll` on the Wayland socket and a wake `eventfd`; it
//! wakes on its own only to repeat a held key. Without cursor-shape-v1,
//! cursors come from the xcursor theme (`XCURSOR_THEME`, `XCURSOR_SIZE`).

mod clipboard;
mod cursor;
mod gpu;
mod ime;
mod keyboard;
mod output;
mod platform;
mod pointer;
mod region;
mod state;
mod window;

pub use gpu::WindowSurface;
pub use keyboard::logical_key;
pub use platform::Wayland;
pub use region::rounded_rects;
