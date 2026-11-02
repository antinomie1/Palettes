//! Shell surfaces: wlr layer surfaces and xdg popups, their configures and scale.

use crate::state::State;
use aegle_platform::{
    Anchor, Cursor, Error, Event, KeyboardInteractivity, Layer, LayerOptions, PixelSize,
    PopupOptions, Result, Side, WindowId, WindowInfo, WindowOptions,
};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, protocol::wl_surface::WlSurface};
use wayland_protocols::{
    wp::{
        fractional_scale::v1::client::wp_fractional_scale_v1::WpFractionalScaleV1,
        viewporter::client::wp_viewport::WpViewport,
    },
    xdg::shell::client::{
        xdg_popup::{self, XdgPopup},
        xdg_positioner::{Anchor as PAnchor, ConstraintAdjustment, Gravity},
        xdg_surface::{self, XdgSurface},
    },
};
use wayland_protocols_wlr::layer_shell::v1::client::{
    zwlr_layer_shell_v1,
    zwlr_layer_surface_v1::{self, ZwlrLayerSurfaceV1},
};

pub(crate) enum Role {
    Layer {
        surface: ZwlrLayerSurfaceV1,
        options: LayerOptions,
        /// The preferred logical size; zero on a stretched axis.
        size: PixelSize,
    },
    Popup {
        xdg: XdgSurface,
        popup: XdgPopup,
    },
}

pub(crate) struct Window {
    pub id: WindowId,
    pub surface: WlSurface,
    pub role: Role,
    viewport: Option<WpViewport>,
    fractional: Option<WpFractionalScaleV1>,
    pub info: WindowInfo,
    /// The scale and logical size last applied to the surface.
    applied: Option<(f32, PixelSize)>,
    pub cursor: Cursor,
    /// Content changed and a frame should be drawn.
    pub redraw_requested: bool,
    /// An [`Event::Redraw`] is queued and not yet answered.
    pub redraw_queued: bool,
    /// A frame callback was requested and will be committed with the next frame.
    pub frame_requested: bool,
    /// A frame was presented and the compositor has not asked for the next one.
    pub frame_pending: bool,
}

fn layer(layer: Layer) -> zwlr_layer_shell_v1::Layer {
    match layer {
        Layer::Background => zwlr_layer_shell_v1::Layer::Background,
        Layer::Bottom => zwlr_layer_shell_v1::Layer::Bottom,
        Layer::Top => zwlr_layer_shell_v1::Layer::Top,
        Layer::Overlay => zwlr_layer_shell_v1::Layer::Overlay,
    }
}

fn anchor(anchor: Anchor) -> zwlr_layer_surface_v1::Anchor {
    use zwlr_layer_surface_v1::Anchor as A;
    let mut out = A::empty();
    for (edge, flag) in [
        (Anchor::TOP, A::Top),
        (Anchor::BOTTOM, A::Bottom),
        (Anchor::LEFT, A::Left),
        (Anchor::RIGHT, A::Right),
    ] {
        if anchor.contains(edge) {
            out |= flag;
        }
    }
    out
}

/// The requested size: zero along an axis anchored on both sides, which the
/// compositor then stretches.
fn layer_size(options: &LayerOptions, size: PixelSize) -> PixelSize {
    let a = options.anchor;
    PixelSize {
        width: if a.contains(Anchor::LEFT | Anchor::RIGHT) {
            0
        } else {
            size.width
        },
        height: if a.contains(Anchor::TOP | Anchor::BOTTOM) {
            0
        } else {
            size.height
        },
    }
}

fn configure_layer(surface: &ZwlrLayerSurfaceV1, options: &LayerOptions, size: PixelSize) {
    surface.set_size(size.width, size.height);
    surface.set_anchor(anchor(options.anchor));
    surface.set_exclusive_zone(options.exclusive_zone);
    let [top, right, bottom, left] = options.margin;
    surface.set_margin(top, right, bottom, left);
    surface.set_keyboard_interactivity(match options.keyboard {
        KeyboardInteractivity::None => zwlr_layer_surface_v1::KeyboardInteractivity::None,
        KeyboardInteractivity::Exclusive => zwlr_layer_surface_v1::KeyboardInteractivity::Exclusive,
        // OnDemand arrived in version 4; Exclusive would steal the keyboard.
        KeyboardInteractivity::OnDemand if surface.version() >= 4 => {
            zwlr_layer_surface_v1::KeyboardInteractivity::OnDemand
        }
        KeyboardInteractivity::OnDemand => zwlr_layer_surface_v1::KeyboardInteractivity::None,
    });
}

impl State {
    pub fn create_window(&mut self, options: WindowOptions<'_>) -> Result<WindowId> {
        if options.layer.is_some() && options.popup.is_some() {
            return Err(Error::InvalidRole("a window is either a layer or a popup"));
        }
        let id = WindowId(self.next_window);
        let qh = self.qh.clone();
        let surface = self.globals.compositor.create_surface(&qh, id);
        let role = match (options.layer, options.popup) {
            (Some(layer), None) => self.layer_role(&surface, layer, options.size, options.app_id),
            (None, Some(popup)) => self.popup_role(&surface, popup, options.size, id),
            _ => Err(Error::Unsupported(
                "toplevel windows; a shell uses layers and popups",
            )),
        };
        let role = match role {
            Ok(role) => role,
            Err(error) => {
                surface.destroy();
                return Err(error);
            }
        };
        self.next_window += 1;
        let viewport = self
            .globals
            .viewporter
            .as_ref()
            .map(|v| v.get_viewport(&surface, &qh, ()));
        let fractional = self
            .globals
            .fractional
            .as_ref()
            .map(|f| f.get_fractional_scale(&surface, &qh, id));
        surface.commit();
        self.windows.push(Window {
            id,
            surface,
            role,
            viewport,
            fractional,
            info: WindowInfo {
                size: options.size,
                buffer: options.size,
                scale: 1.0,
                active: true,
                configured: false,
            },
            applied: None,
            cursor: Cursor::Default,
            redraw_requested: true,
            redraw_queued: false,
            frame_requested: false,
            frame_pending: false,
        });
        Ok(id)
    }

    fn layer_role(
        &mut self,
        surface: &WlSurface,
        options: LayerOptions,
        size: PixelSize,
        namespace: &str,
    ) -> Result<Role> {
        let shell = self.globals.layer_shell.as_ref().ok_or(Error::Unsupported(
            "layer surfaces: the compositor has no wlr-layer-shell",
        ))?;
        let output = match options.output {
            Some(id) => Some(
                &self
                    .output(id)
                    .ok_or(Error::InvalidRole("the output is not connected"))?
                    .output,
            ),
            None => None,
        };
        let id = surface
            .data::<WindowId>()
            .copied()
            .expect("surfaces carry their window");
        let layer_surface = shell.get_layer_surface(
            surface,
            output,
            layer(options.layer),
            namespace.into(),
            &self.qh,
            id,
        );
        let size = layer_size(&options, size);
        configure_layer(&layer_surface, &options, size);
        Ok(Role::Layer {
            surface: layer_surface,
            options,
            size,
        })
    }

    fn popup_role(
        &mut self,
        surface: &WlSurface,
        options: PopupOptions,
        size: PixelSize,
        id: WindowId,
    ) -> Result<Role> {
        let wm_base = self.globals.wm_base.as_ref().ok_or(Error::Unsupported(
            "popups: the compositor has no xdg-shell",
        ))?;
        let parent = self.window(options.parent).ok_or(Error::InvalidWindow)?;
        let rect = options.anchor;
        if rect.size.width <= 0.0 || rect.size.height <= 0.0 {
            return Err(Error::InvalidSize);
        }
        let positioner = wm_base.create_positioner(&self.qh, ());
        positioner.set_size(size.width.max(1) as i32, size.height.max(1) as i32);
        positioner.set_anchor_rect(
            rect.origin.x.floor() as i32,
            rect.origin.y.floor() as i32,
            rect.size.width.ceil().max(1.0) as i32,
            rect.size.height.ceil().max(1.0) as i32,
        );
        let (anchor, gravity) = match options.side {
            Side::Below => (PAnchor::BottomLeft, Gravity::BottomRight),
            Side::Above => (PAnchor::TopLeft, Gravity::TopRight),
            Side::Left => (PAnchor::TopLeft, Gravity::BottomLeft),
            Side::Right => (PAnchor::TopRight, Gravity::BottomRight),
        };
        positioner.set_anchor(anchor);
        positioner.set_gravity(gravity);
        positioner.set_constraint_adjustment(
            ConstraintAdjustment::FlipX
                | ConstraintAdjustment::FlipY
                | ConstraintAdjustment::SlideX
                | ConstraintAdjustment::SlideY,
        );
        let xdg = wm_base.get_xdg_surface(surface, &self.qh, id);
        let popup = match &parent.role {
            Role::Popup { xdg: parent, .. } => {
                xdg.get_popup(Some(parent), &positioner, &self.qh, id)
            }
            Role::Layer { surface, .. } => {
                let popup = xdg.get_popup(None, &positioner, &self.qh, id);
                surface.get_popup(&popup);
                popup
            }
        };
        positioner.destroy();
        if let Some(seat) = options.grab {
            // A grab needs the serial of the press that opened the popup.
            if let Some(seat) = self.seats.iter().find(|s| s.id == seat) {
                popup.grab(&seat.seat, seat.last_serial);
            }
        }
        Ok(Role::Popup { xdg, popup })
    }

    pub fn set_layer(
        &mut self,
        id: WindowId,
        options: LayerOptions,
        size: PixelSize,
    ) -> Result<()> {
        let window = self.window_mut(id).ok_or(Error::InvalidWindow)?;
        let Role::Layer {
            surface,
            options: current,
            size: current_size,
        } = &mut window.role
        else {
            return Err(Error::InvalidRole("not a layer surface"));
        };
        if options.output != current.output {
            return Err(Error::InvalidRole(
                "a layer surface cannot change its output",
            ));
        }
        if options.layer != current.layer {
            if surface.version() < 2 {
                return Err(Error::Unsupported("changing the layer of a layer surface"));
            }
            surface.set_layer(layer(options.layer));
        }
        let size = layer_size(&options, size);
        configure_layer(surface, &options, size);
        *current = options;
        *current_size = size;
        window.surface.commit();
        Ok(())
    }

    pub fn remove_window(&mut self, id: WindowId) -> Result<()> {
        let index = self
            .windows
            .iter()
            .position(|w| w.id == id)
            .ok_or(Error::InvalidWindow)?;
        let window = self.windows.remove(index);
        match window.role {
            Role::Layer { surface, .. } => surface.destroy(),
            Role::Popup { xdg, popup } => {
                popup.destroy();
                xdg.destroy();
            }
        }
        if let Some(viewport) = window.viewport {
            viewport.destroy();
        }
        if let Some(fractional) = window.fractional {
            fractional.destroy();
        }
        window.surface.destroy();
        self.events.retain(|event| event.window() != Some(id));
        for seat in &mut self.seats {
            seat.forget(id);
        }
        Ok(())
    }

    /// The compositor fixed a size: `width`/`height` zero keep the requested one.
    fn configured(&mut self, id: WindowId, width: u32, height: u32) {
        let Some(window) = self.window_mut(id) else {
            return;
        };
        let requested = match &window.role {
            Role::Layer { size, .. } => *size,
            Role::Popup { .. } => window.info.size,
        };
        let size = PixelSize {
            width: if width == 0 { requested.width } else { width },
            height: if height == 0 {
                requested.height
            } else {
                height
            },
        };
        window.info.size = size;
        window.info.configured = size.width > 0 && size.height > 0;
        window.redraw_requested = true;
        self.update_buffer(id);
    }

    pub fn rescale(&mut self, id: WindowId, scale: f32) {
        let Some(window) = self.window_mut(id) else {
            return;
        };
        if window.info.scale == scale {
            return;
        }
        window.info.scale = scale;
        window.redraw_requested = true;
        self.update_buffer(id);
    }

    /// Derives the buffer extent and reports the new geometry.
    fn update_buffer(&mut self, id: WindowId) {
        let viewporter = self.globals.viewporter.is_some();
        let Some(window) = self.window_mut(id) else {
            return;
        };
        // Without a viewport only whole buffer scales exist.
        if !viewporter {
            window.info.scale = window.info.scale.ceil();
        }
        let scale = window.info.scale;
        let size = window.info.size;
        window.info.buffer = PixelSize {
            width: (size.width as f32 * scale).round() as u32,
            height: (size.height as f32 * scale).round() as u32,
        };
        if window.info.configured {
            let info = window.info;
            self.events.push_back(Event::Configure { window: id, info });
        }
    }
}

impl Window {
    /// Applies scale and logical size to the surface; the next commit, made
    /// by the presenter, carries them with the frame drawn for them.
    pub fn apply_scale(&mut self) {
        let wanted = (self.info.scale, self.info.size);
        if self.applied == Some(wanted) {
            return;
        }
        match &self.viewport {
            Some(viewport) => {
                viewport.set_destination(self.info.size.width as i32, self.info.size.height as i32);
            }
            None => self.surface.set_buffer_scale(self.info.scale as i32),
        }
        self.applied = Some(wanted);
    }
}

impl Dispatch<ZwlrLayerSurfaceV1, WindowId> for State {
    fn event(
        state: &mut Self,
        surface: &ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        id: &WindowId,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_layer_surface_v1::Event::Configure {
                serial,
                width,
                height,
            } => {
                surface.ack_configure(serial);
                state.configured(*id, width, height);
            }
            zwlr_layer_surface_v1::Event::Closed => {
                state.events.push_back(Event::Close { window: *id });
            }
            _ => {}
        }
    }
}

impl Dispatch<XdgSurface, WindowId> for State {
    fn event(
        _: &mut Self,
        xdg: &XdgSurface,
        event: xdg_surface::Event,
        _: &WindowId,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            xdg.ack_configure(serial);
        }
    }
}

impl Dispatch<XdgPopup, WindowId> for State {
    fn event(
        state: &mut Self,
        _: &XdgPopup,
        event: xdg_popup::Event,
        id: &WindowId,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            xdg_popup::Event::Configure { width, height, .. } => {
                state.configured(*id, width.max(0) as u32, height.max(0) as u32);
            }
            xdg_popup::Event::PopupDone => state.events.push_back(Event::Close { window: *id }),
            _ => {}
        }
    }
}
