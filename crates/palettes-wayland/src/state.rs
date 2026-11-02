//! Globals, the registry and the small protocol objects every surface shares.

use crate::{output::Output, pointer::Seat, window::Window};
use aegle_platform::{Event, OutputId, WindowId};
use std::collections::VecDeque;
use wayland_client::{
    Connection, Dispatch, Proxy, QueueHandle, delegate_noop,
    globals::{GlobalList, GlobalListContents},
    protocol::{
        wl_callback::{self, WlCallback},
        wl_compositor::WlCompositor,
        wl_output::WlOutput,
        wl_registry::{self, WlRegistry},
        wl_seat::WlSeat,
        wl_surface::{self, WlSurface},
    },
};
use wayland_protocols::{
    wp::{
        cursor_shape::v1::client::wp_cursor_shape_manager_v1::WpCursorShapeManagerV1,
        fractional_scale::v1::client::{
            wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1,
            wp_fractional_scale_v1::{self, WpFractionalScaleV1},
        },
        viewporter::client::{wp_viewport::WpViewport, wp_viewporter::WpViewporter},
    },
    xdg::{
        shell::client::{
            xdg_positioner::XdgPositioner,
            xdg_wm_base::{self, XdgWmBase},
        },
        xdg_output::zv1::client::zxdg_output_manager_v1::ZxdgOutputManagerV1,
    },
};
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_shell_v1::ZwlrLayerShellV1;

/// Globals bound once at connection.
pub(crate) struct Globals {
    pub compositor: WlCompositor,
    pub layer_shell: Option<ZwlrLayerShellV1>,
    pub wm_base: Option<XdgWmBase>,
    pub viewporter: Option<WpViewporter>,
    pub fractional: Option<WpFractionalScaleManagerV1>,
    pub cursor_shape: Option<WpCursorShapeManagerV1>,
    pub xdg_output: Option<ZxdgOutputManagerV1>,
}

pub(crate) struct State {
    pub globals: Globals,
    pub qh: QueueHandle<State>,
    pub outputs: Vec<Output>,
    pub seats: Vec<Seat>,
    pub windows: Vec<Window>,
    pub events: VecDeque<Event>,
    pub next_window: u64,
    pub next_seat: u32,
}

impl State {
    pub fn new(
        globals: &GlobalList,
        qh: QueueHandle<State>,
    ) -> Result<Self, aegle_platform::Error> {
        let compositor = globals
            .bind::<WlCompositor, _, _>(&qh, 4..=6, ())
            .map_err(|e| aegle_platform::Error::backend(format!("wl_compositor: {e}")))?;
        let globals_bound = Globals {
            compositor,
            layer_shell: globals.bind(&qh, 1..=4, ()).ok(),
            wm_base: globals.bind(&qh, 1..=6, ()).ok(),
            viewporter: globals.bind(&qh, 1..=1, ()).ok(),
            fractional: globals.bind(&qh, 1..=1, ()).ok(),
            cursor_shape: globals.bind(&qh, 1..=1, ()).ok(),
            xdg_output: globals.bind(&qh, 2..=3, ()).ok(),
        };
        let mut state = Self {
            globals: globals_bound,
            qh,
            outputs: Vec::new(),
            seats: Vec::new(),
            windows: Vec::new(),
            events: VecDeque::new(),
            next_window: 1,
            next_seat: 0,
        };
        let registry = globals.registry();
        for global in globals.contents().clone_list() {
            state.global(registry, global.name, &global.interface, global.version);
        }
        Ok(state)
    }

    /// Binds outputs and seats as they appear.
    fn global(&mut self, registry: &WlRegistry, name: u32, interface: &str, version: u32) {
        if interface == WlOutput::interface().name {
            let output = registry.bind::<WlOutput, _, _>(name, version.min(4), &self.qh, name);
            self.outputs
                .push(Output::new(name, output, &self.globals, &self.qh));
        } else if interface == WlSeat::interface().name {
            let seat = registry.bind::<WlSeat, _, _>(name, version.min(7), &self.qh, name);
            let id = aegle_platform::SeatId(self.next_seat);
            self.next_seat += 1;
            self.seats.push(Seat::new(name, id, seat));
        }
    }

    fn global_remove(&mut self, name: u32) {
        if let Some(index) = self.outputs.iter().position(|o| o.global == name) {
            self.outputs.remove(index).destroy();
            self.events.push_back(Event::Outputs);
        }
        if let Some(index) = self.seats.iter().position(|s| s.global == name) {
            self.seats.remove(index).release();
        }
    }

    pub fn window(&self, id: WindowId) -> Option<&Window> {
        self.windows.iter().find(|w| w.id == id)
    }

    pub fn window_mut(&mut self, id: WindowId) -> Option<&mut Window> {
        self.windows.iter_mut().find(|w| w.id == id)
    }

    pub fn output(&self, id: OutputId) -> Option<&Output> {
        self.outputs.iter().find(|o| u64::from(o.global) == id.0)
    }

    /// Queues one [`Event::Redraw`] for every window that wants a frame and can
    /// present one now.
    pub fn queue_redraws(&mut self) {
        for window in &mut self.windows {
            if window.redraw_requested
                && window.info.configured
                && !window.frame_pending
                && !window.redraw_queued
            {
                window.redraw_queued = true;
                self.events.push_back(Event::Redraw { window: window.id });
            }
        }
    }
}

impl Dispatch<WlRegistry, GlobalListContents> for State {
    fn event(
        state: &mut Self,
        registry: &WlRegistry,
        event: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_registry::Event::Global {
                name,
                interface,
                version,
            } => state.global(registry, name, &interface, version),
            wl_registry::Event::GlobalRemove { name } => state.global_remove(name),
            _ => {}
        }
    }
}

impl Dispatch<XdgWmBase, ()> for State {
    fn event(
        _: &mut Self,
        wm_base: &XdgWmBase,
        event: xdg_wm_base::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            wm_base.pong(serial);
        }
    }
}

/// A frame callback: the compositor shows the window's last frame, so it may
/// draw the next.
impl Dispatch<WlCallback, WindowId> for State {
    fn event(
        state: &mut Self,
        _: &WlCallback,
        event: wl_callback::Event,
        id: &WindowId,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_callback::Event::Done { .. } = event {
            if let Some(window) = state.window_mut(*id) {
                window.frame_pending = false;
            }
        }
    }
}

impl Dispatch<WpFractionalScaleV1, WindowId> for State {
    fn event(
        state: &mut Self,
        _: &WpFractionalScaleV1,
        event: wp_fractional_scale_v1::Event,
        id: &WindowId,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wp_fractional_scale_v1::Event::PreferredScale { scale } = event {
            state.rescale(*id, scale as f32 / 120.0);
        }
    }
}

impl Dispatch<WlSurface, WindowId> for State {
    fn event(
        state: &mut Self,
        _: &WlSurface,
        event: wl_surface::Event,
        id: &WindowId,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // Fractional scale, when bound, is the authority.
        if state.globals.fractional.is_some() {
            return;
        }
        if let wl_surface::Event::PreferredBufferScale { factor } = event {
            state.rescale(*id, factor.max(1) as f32);
        }
    }
}

delegate_noop!(State: ignore WlCompositor);
delegate_noop!(State: ignore ZwlrLayerShellV1);
delegate_noop!(State: ignore WpViewporter);
delegate_noop!(State: ignore WpViewport);
delegate_noop!(State: ignore WpFractionalScaleManagerV1);
delegate_noop!(State: ignore WpCursorShapeManagerV1);
delegate_noop!(State: ignore ZxdgOutputManagerV1);
delegate_noop!(State: ignore XdgPositioner);
