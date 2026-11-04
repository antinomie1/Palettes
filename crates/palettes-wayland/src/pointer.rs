//! Seats: capabilities, pointer (with cursor shapes, or theme images in
//! `cursor.rs`) and touch. The keyboard is in `keyboard.rs`.

use crate::{keyboard::Keyboard, state::State};
use aegle_platform::{
    Cursor, Event, Point, PointerButton, PointerKind, SeatId, TouchPhase, WindowId,
};
use wayland_client::{
    Connection, Dispatch, Proxy, QueueHandle, WEnum,
    protocol::{
        wl_pointer::{self, Axis, AxisSource, ButtonState, WlPointer},
        wl_seat::{self, Capability, WlSeat},
        wl_surface::WlSurface,
        wl_touch::{self, WlTouch},
    },
};
use wayland_protocols::wp::cursor_shape::v1::client::wp_cursor_shape_device_v1::{
    Shape, WpCursorShapeDeviceV1,
};

const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;
const BTN_MIDDLE: u32 = 0x112;
const BTN_SIDE: u32 = 0x113;
const BTN_EXTRA: u32 = 0x114;

pub(crate) struct Seat {
    /// Registry name.
    pub global: u32,
    pub id: SeatId,
    pub seat: WlSeat,
    pub pointer: Option<WlPointer>,
    pub cursor: Option<WpCursorShapeDeviceV1>,
    /// The cursor image's surface, without cursor-shape-v1.
    pub cursor_surface: Option<WlSurface>,
    touch: Option<WlTouch>,
    pub keyboard: Keyboard,
    /// The serial of the latest press, which authorizes popup grabs.
    pub last_serial: u32,
    pub pointer_focus: Option<WindowId>,
    pub enter_serial: u32,
    position: Point,
    /// Scrolling accumulated over one pointer frame.
    scroll: Point,
    finger: bool,
    stopped: bool,
    time: u32,
    /// Touch points: the window each went down in, and where it is.
    fingers: Vec<(i32, WindowId, Point)>,
}

fn window_of(surface: &WlSurface) -> Option<WindowId> {
    surface.data::<WindowId>().copied()
}

pub(crate) fn shape(cursor: Cursor) -> Shape {
    match cursor {
        Cursor::Default => Shape::Default,
        Cursor::Text => Shape::Text,
        Cursor::Pointer => Shape::Pointer,
        Cursor::Crosshair => Shape::Crosshair,
        Cursor::Move => Shape::Move,
        Cursor::Grab => Shape::Grab,
        Cursor::Grabbing => Shape::Grabbing,
        Cursor::NotAllowed => Shape::NotAllowed,
        Cursor::ResizeHorizontal => Shape::EwResize,
        Cursor::ResizeVertical => Shape::NsResize,
    }
}

impl Seat {
    pub fn new(global: u32, id: SeatId, seat: WlSeat) -> Self {
        Self {
            global,
            id,
            seat,
            pointer: None,
            cursor: None,
            cursor_surface: None,
            touch: None,
            keyboard: Keyboard::default(),
            last_serial: 0,
            pointer_focus: None,
            enter_serial: 0,
            position: Point::new(0.0, 0.0),
            scroll: Point::new(0.0, 0.0),
            finger: false,
            stopped: false,
            time: 0,
            fingers: Vec::new(),
        }
    }

    pub fn release(mut self) {
        self.set_pointer(false, None, None);
        self.keyboard.release();
        if let Some(touch) = self.touch.take() {
            touch.release();
        }
        if self.seat.version() >= 5 {
            self.seat.release();
        }
    }

    /// A removed window stops receiving input.
    pub fn forget(&mut self, id: WindowId) {
        if self.pointer_focus == Some(id) {
            self.pointer_focus = None;
        }
        self.fingers.retain(|(_, window, _)| *window != id);
        self.keyboard.forget(id);
    }

    fn set_pointer(&mut self, on: bool, state: Option<&State>, qh: Option<&QueueHandle<State>>) {
        match (on, self.pointer.is_some()) {
            (true, false) => {
                let (state, qh) = (state.expect("state"), qh.expect("qh"));
                let pointer = self.seat.get_pointer(qh, self.global);
                self.cursor = state
                    .globals
                    .cursor_shape
                    .as_ref()
                    .map(|manager| manager.get_pointer(&pointer, qh, ()));
                self.cursor_surface = state.cursor_surface(qh);
                self.pointer = Some(pointer);
            }
            (false, true) => {
                if let Some(cursor) = self.cursor.take() {
                    cursor.destroy();
                }
                if let Some(surface) = self.cursor_surface.take() {
                    surface.destroy();
                }
                if let Some(pointer) = self.pointer.take() {
                    pointer.release();
                }
                self.pointer_focus = None;
            }
            _ => {}
        }
    }

}

impl State {
    fn seat_mut(&mut self, global: u32) -> Option<&mut Seat> {
        self.seats.iter_mut().find(|s| s.global == global)
    }

    fn pointer_event(
        &mut self,
        global: u32,
        window: WindowId,
        kind: PointerKind,
        time: Option<u32>,
    ) {
        let Some(seat) = self.seat_mut(global) else {
            return;
        };
        let (id, position) = (seat.id, seat.position);
        self.events.push_back(Event::Pointer {
            window,
            seat: id,
            position,
            kind,
            time,
        });
    }
}

impl Dispatch<WlSeat, u32> for State {
    fn event(
        state: &mut Self,
        _: &WlSeat,
        event: wl_seat::Event,
        global: &u32,
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        let wl_seat::Event::Capabilities {
            capabilities: WEnum::Value(caps),
        } = event
        else {
            return;
        };
        let Some(index) = state.seats.iter().position(|s| s.global == *global) else {
            return;
        };
        let mut seat = state.seats.swap_remove(index);
        seat.set_pointer(caps.contains(Capability::Pointer), Some(state), Some(qh));
        seat.keyboard
            .set_enabled(caps.contains(Capability::Keyboard), &seat.seat, *global, qh);
        match (caps.contains(Capability::Touch), seat.touch.is_some()) {
            (true, false) => seat.touch = Some(seat.seat.get_touch(qh, *global)),
            (false, true) => {
                if let Some(touch) = seat.touch.take() {
                    touch.release();
                }
            }
            _ => {}
        }
        state.seats.push(seat);
    }
}

impl Dispatch<WlPointer, u32> for State {
    fn event(
        state: &mut Self,
        _: &WlPointer,
        event: wl_pointer::Event,
        global: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let global = *global;
        let cursors: Vec<(WindowId, Cursor)> =
            state.windows.iter().map(|w| (w.id, w.cursor)).collect();
        let Some(seat) = state.seat_mut(global) else {
            return;
        };
        match event {
            wl_pointer::Event::Enter {
                serial,
                surface,
                surface_x,
                surface_y,
            } => {
                let Some(window) = window_of(&surface) else {
                    return;
                };
                seat.pointer_focus = Some(window);
                seat.enter_serial = serial;
                seat.position = Point::new(surface_x as f32, surface_y as f32);
                state.pointer_event(global, window, PointerKind::Move, None);
                if let Some((_, cursor)) = cursors.iter().find(|(id, _)| *id == window) {
                    state.show_cursor(window, *cursor);
                }
            }
            wl_pointer::Event::Leave { .. } => {
                if let Some(window) = seat.pointer_focus.take() {
                    state.pointer_event(global, window, PointerKind::Leave, None);
                }
            }
            wl_pointer::Event::Motion {
                time,
                surface_x,
                surface_y,
            } => {
                seat.position = Point::new(surface_x as f32, surface_y as f32);
                if let Some(window) = seat.pointer_focus {
                    state.pointer_event(global, window, PointerKind::Move, Some(time));
                }
            }
            wl_pointer::Event::Button {
                serial,
                time,
                button,
                state: WEnum::Value(pressed),
            } => {
                let pressed = pressed == ButtonState::Pressed;
                if pressed {
                    seat.last_serial = serial;
                }
                let Some(window) = seat.pointer_focus else {
                    return;
                };
                let other = |b| {
                    if pressed {
                        PointerKind::ButtonDown(b)
                    } else {
                        PointerKind::ButtonUp(b)
                    }
                };
                let kind = match button {
                    BTN_LEFT if pressed => PointerKind::Down { clicks: 1 },
                    BTN_LEFT => PointerKind::Up,
                    BTN_RIGHT => other(PointerButton::Secondary),
                    BTN_MIDDLE => other(PointerButton::Middle),
                    BTN_SIDE => other(PointerButton::Back),
                    BTN_EXTRA => other(PointerButton::Forward),
                    _ => return,
                };
                state.pointer_event(global, window, kind, Some(time));
            }
            wl_pointer::Event::Axis { time, axis, value } => {
                seat.time = time;
                match axis {
                    WEnum::Value(Axis::VerticalScroll) => seat.scroll.y += value as f32,
                    WEnum::Value(Axis::HorizontalScroll) => seat.scroll.x += value as f32,
                    _ => {}
                }
            }
            wl_pointer::Event::AxisSource { axis_source } => {
                seat.finger = axis_source == WEnum::Value(AxisSource::Finger);
            }
            wl_pointer::Event::AxisStop { time, .. } => {
                seat.time = time;
                seat.stopped = true;
            }
            wl_pointer::Event::Frame => {
                let Some(window) = seat.pointer_focus else {
                    return;
                };
                let delta = std::mem::replace(&mut seat.scroll, Point::new(0.0, 0.0));
                let (stopped, time) = (std::mem::take(&mut seat.stopped), seat.time);
                if delta.x != 0.0 || delta.y != 0.0 {
                    state.pointer_event(global, window, PointerKind::Scroll { delta }, Some(time));
                }
                if stopped {
                    state.pointer_event(global, window, PointerKind::ScrollEnd, Some(time));
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<WlTouch, u32> for State {
    fn event(
        state: &mut Self,
        _: &WlTouch,
        event: wl_touch::Event,
        global: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(seat) = state.seat_mut(*global) else {
            return;
        };
        let seat_id = seat.id;
        let (finger, position, time, phase) = match event {
            wl_touch::Event::Down {
                serial,
                time,
                surface,
                id,
                x,
                y,
            } => {
                let Some(window) = window_of(&surface) else {
                    return;
                };
                seat.last_serial = serial;
                let position = Point::new(x as f32, y as f32);
                seat.fingers.push((id, window, position));
                (id, position, time, TouchPhase::Down)
            }
            wl_touch::Event::Motion { time, id, x, y } => {
                (id, Point::new(x as f32, y as f32), time, TouchPhase::Move)
            }
            wl_touch::Event::Up { time, id, .. } => {
                let last = seat.fingers.iter().find(|f| f.0 == id).map(|f| f.2);
                (
                    id,
                    last.unwrap_or(Point::new(0.0, 0.0)),
                    time,
                    TouchPhase::Up,
                )
            }
            wl_touch::Event::Cancel => {
                for (id, window, position) in std::mem::take(&mut seat.fingers) {
                    state.events.push_back(Event::Touch {
                        window,
                        seat: seat_id,
                        finger: id as u32,
                        position,
                        time: 0,
                        phase: TouchPhase::Cancel,
                    });
                }
                return;
            }
            _ => return,
        };
        let Some(index) = seat.fingers.iter().position(|(id, _, _)| *id == finger) else {
            return;
        };
        seat.fingers[index].2 = position;
        let window = seat.fingers[index].1;
        if phase == TouchPhase::Up {
            seat.fingers.remove(index);
        }
        state.events.push_back(Event::Touch {
            window,
            seat: seat_id,
            finger: finger as u32,
            position,
            time,
            phase,
        });
    }
}

wayland_client::delegate_noop!(State: ignore WpCursorShapeDeviceV1);
