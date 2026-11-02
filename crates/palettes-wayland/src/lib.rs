//! A Wayland client written for desktop shells.
//!
//! [`Wayland`] implements aegle's [`Platform`](aegle_platform::Platform) and
//! [`SurfacePlatform`](aegle_platform::SurfacePlatform) for the surfaces a shell
//! needs: wlr layer surfaces on chosen outputs, xdg popups attached to them or
//! to each other, outputs with names and fractional scales, and seat input
//! (pointer, keyboard through xkbcommon with key repeat, touch). There are no
//! ordinary toplevel windows and no software buffers: frames are presented by
//! a GPU renderer through [`WindowSurface`] leases, paced by frame callbacks.
//!
//! The loop blocks in `poll` on the Wayland socket and a wake `eventfd`; it
//! wakes on its own only to repeat a held key.

mod gpu;
mod keyboard;
mod output;
mod platform;
mod pointer;
mod state;
mod window;

pub use gpu::WindowSurface;
pub use keyboard::logical_key;
pub use platform::Wayland;
