//! Cursor images from the xcursor theme, for compositors without
//! cursor-shape-v1. The theme is the desktop's
//! ([`palettes_assets::cursor_theme`]), loaded on first use, once per
//! integer scale. Animated cursors show their first frame: none of the
//! shapes aegle asks for (no busy or progress cursors) animate in common
//! themes, and a frame timer would wake an idle shell.

use aegle_platform::{Cursor, WindowId};
use wayland_client::{
    Connection, QueueHandle,
    protocol::{wl_pointer::WlPointer, wl_shm::WlShm, wl_surface::WlSurface},
};
use wayland_cursor::CursorTheme;

use crate::state::State;

/// Theme names for a cursor: the CSS name first, then older X11 names.
fn names(cursor: Cursor) -> &'static [&'static str] {
    match cursor {
        Cursor::Default => &["default", "left_ptr"],
        Cursor::Text => &["text", "xterm"],
        Cursor::Pointer => &["pointer", "hand2", "hand1"],
        Cursor::Crosshair => &["crosshair", "cross"],
        Cursor::Move => &["move", "fleur", "all-scroll"],
        Cursor::Grab => &["grab", "openhand", "hand1"],
        Cursor::Grabbing => &["grabbing", "closedhand", "fleur"],
        Cursor::NotAllowed => &["not-allowed", "crossed_circle"],
        Cursor::ResizeHorizontal => &["ew-resize", "sb_h_double_arrow", "h_double_arrow"],
        Cursor::ResizeVertical => &["ns-resize", "sb_v_double_arrow", "v_double_arrow"],
    }
}

pub(crate) struct Themes {
    connection: Connection,
    shm: WlShm,
    name: String,
    /// Logical size in pixels.
    size: u32,
    loaded: Vec<(u32, Option<CursorTheme>)>,
}

impl Themes {
    pub fn new(connection: Connection, shm: WlShm) -> Self {
        let (name, size) = palettes_assets::cursor_theme();
        Self {
            connection,
            shm,
            name,
            size,
            loaded: Vec::new(),
        }
    }

    fn theme(&mut self, scale: u32) -> Option<&mut CursorTheme> {
        let at = match self.loaded.iter().position(|(s, _)| *s == scale) {
            Some(at) => at,
            None => {
                let theme = CursorTheme::load_from_name(
                    &self.connection,
                    self.shm.clone(),
                    &self.name,
                    self.size * scale,
                )
                .ok();
                self.loaded.push((scale, theme));
                self.loaded.len() - 1
            }
        };
        self.loaded[at].1.as_mut()
    }

    /// Shows `cursor` on `surface` as the pointer's image, at `scale`, or
    /// at 1 where the theme has no image that divides by it.
    pub fn show(
        &mut self,
        pointer: &WlPointer,
        serial: u32,
        surface: &WlSurface,
        cursor: Cursor,
        scale: u32,
    ) {
        for scale in [scale, 1] {
            let Some(theme) = self.theme(scale) else {
                continue;
            };
            // Lookups load and cache the file, so the second one is free.
            let Some(name) = names(cursor).iter().find(|n| theme.get_cursor(n).is_some()) else {
                continue;
            };
            let image = &theme.get_cursor(name).expect("found above")[0];
            let ((width, height), (x, y)) = (image.dimensions(), image.hotspot());
            // A buffer must be a whole number of surface pixels.
            if width % scale != 0 || height % scale != 0 {
                continue;
            }
            surface.set_buffer_scale(scale as i32);
            surface.attach(Some(image), 0, 0);
            surface.damage_buffer(0, 0, width as i32, height as i32);
            surface.commit();
            pointer.set_cursor(serial, Some(surface), (x / scale) as i32, (y / scale) as i32);
            return;
        }
    }
}

impl State {
    /// A surface for a seat's cursor image, where images come from the theme.
    pub fn cursor_surface(&self, qh: &QueueHandle<State>) -> Option<WlSurface> {
        // Window ids start at 1, so this surface is no window's.
        self.themes
            .as_ref()
            .map(|_| self.globals.compositor.create_surface(qh, WindowId(0)))
    }

    /// Shows `cursor` to every seat whose pointer is over `window`.
    pub fn show_cursor(&mut self, window: WindowId, cursor: Cursor) {
        let scale = self
            .window(window)
            .map_or(1, |w| w.info.scale.ceil().max(1.0) as u32);
        for seat in &self.seats {
            if seat.pointer_focus != Some(window) {
                continue;
            }
            if let Some(device) = &seat.cursor {
                device.set_shape(seat.enter_serial, crate::pointer::shape(cursor));
            } else if let (Some(themes), Some(pointer), Some(surface)) =
                (self.themes.as_mut(), &seat.pointer, &seat.cursor_surface)
            {
                themes.show(pointer, seat.enter_serial, surface, cursor, scale);
            }
        }
    }
}
