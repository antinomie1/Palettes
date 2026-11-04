//! Input methods through text-input-v3: one text input per seat, enabled
//! while the window with its keyboard has an editor that asked for one.
//!
//! The protocol is a transaction log: requests go out in batches closed by
//! `commit`, and the input method answers with batches closed by `done`
//! carrying the number of commits it has seen. Answers to older commits are
//! still applied, but hold back new state until the input method catches up,
//! so the two sides never chase each other.

use crate::state::State;
use aegle_platform::{
    Error, Event, ImeEvent, ImeRequest, ImeUpdate, Preedit, Rect, SeatId, WindowId,
};
use wayland_client::{
    Connection, Dispatch, Proxy, QueueHandle, delegate_noop, protocol::wl_seat::WlSeat,
};
use wayland_protocols::wp::text_input::zv3::client::{
    zwp_text_input_manager_v3::ZwpTextInputManagerV3,
    zwp_text_input_v3::{self, ChangeCause, ContentHint, ContentPurpose, ZwpTextInputV3},
};

/// What a window's focused editor wants from the input method.
struct Desired {
    window: WindowId,
    request: ImeRequest,
    /// The caret in surface coordinates, rounded outward.
    rect: [i32; 4],
}

/// An answer collected until its `done`.
#[derive(Default)]
struct Pending {
    commit: Option<String>,
    preedit: Preedit,
    before: u32,
    after: u32,
    error: Option<&'static str>,
}

struct Input {
    seat: SeatId,
    /// The seat's registry name.
    global: u32,
    object: ZwpTextInputV3,
    /// The window text input entered.
    focus: Option<WindowId>,
    /// Enabled on the compositor's side.
    enabled: bool,
    /// The content type sent with the enable; `None` needs a new enable.
    content: Option<(ContentHint, bool)>,
    /// Commits sent, and the count at the latest enable.
    serial: u32,
    enable_serial: u32,
    /// The input method answered an older commit; wait for the current one.
    blocked: bool,
    /// The desired state changed while blocked.
    dirty: bool,
    /// A preedit is showing, so an empty one ends it.
    composing: bool,
    pending: Pending,
}

impl Input {
    fn commit(&mut self) {
        self.object.commit();
        self.serial = self.serial.wrapping_add(1);
    }

    fn disable(&mut self) {
        if std::mem::take(&mut self.enabled) {
            self.object.disable();
            self.commit();
        }
        self.content = None;
        self.blocked = false;
        self.composing = false;
        self.pending = Pending::default();
    }

    fn send(&mut self, desired: Option<&Desired>) {
        self.dirty = false;
        let Some(Desired { request, rect, .. }) = desired else {
            self.disable();
            return;
        };
        let hint = if request.multiline {
            ContentHint::Multiline
        } else {
            ContentHint::empty()
        };
        let content = (hint, request.surrounding.is_some());
        // A new content type only applies with an enable.
        if self.enabled && self.content != Some(content) {
            self.disable();
        }
        if self.content.is_none() {
            self.object.enable();
            self.enable_serial = self.serial.wrapping_add(1);
            self.enabled = true;
            self.content = Some(content);
            self.pending = Pending::default();
        }
        self.object.set_content_type(hint, ContentPurpose::Normal);
        self.object.set_text_change_cause(if request.input_method {
            ChangeCause::InputMethod
        } else {
            ChangeCause::Other
        });
        if let Some(text) = &request.surrounding {
            self.object
                .set_surrounding_text(text.clone(), request.cursor as i32, request.anchor as i32);
        }
        self.object
            .set_cursor_rectangle(rect[0], rect[1], rect[2], rect[3]);
        self.commit();
    }

    /// Focus left: the compositor ignores requests until the next enter,
    /// which must enable and send everything again.
    fn leave(&mut self) -> Option<WindowId> {
        self.content = None;
        self.pending = Pending::default();
        self.blocked = false;
        self.composing = false;
        self.dirty = false;
        self.focus.take()
    }
}

pub(crate) struct Ime {
    manager: Option<ZwpTextInputManagerV3>,
    inputs: Vec<Input>,
    desired: Vec<Desired>,
}

/// The caret rectangle in protocol coordinates, rounded outward.
fn caret(request: &ImeRequest) -> Result<[i32; 4], Error> {
    request.validate()?;
    let Rect { origin, size } = request.cursor_rect;
    // Edges in f64: f32 cannot tell i32::MAX from 2^31.
    let x = f64::from(origin.x).floor();
    let y = f64::from(origin.y).floor();
    let right = (f64::from(origin.x) + f64::from(size.width)).ceil();
    let bottom = (f64::from(origin.y) + f64::from(size.height)).ceil();
    let values = [x, y, right - x, bottom - y];
    if values
        .iter()
        .any(|v| *v < f64::from(i32::MIN) || *v > f64::from(i32::MAX))
    {
        return Err(Error::InvalidIme(
            "cursor rectangle exceeds protocol coordinates",
        ));
    }
    Ok(values.map(|v| v as i32))
}

/// A preedit from its protocol text and byte cursor; `-1, -1` hides it.
fn preedit(text: Option<String>, begin: i32, end: i32) -> Result<Preedit, &'static str> {
    let text = text.unwrap_or_default();
    let cursor = if begin == -1 && end == -1 {
        None
    } else if begin >= 0
        && end >= 0
        && text.is_char_boundary(begin as usize)
        && text.is_char_boundary(end as usize)
    {
        Some((begin as usize, end as usize))
    } else {
        return Err("preedit cursor must be hidden or on UTF-8 boundaries");
    };
    Ok(Preedit { text, cursor })
}

impl Ime {
    pub fn new(manager: Option<ZwpTextInputManagerV3>) -> Self {
        Self {
            manager,
            inputs: Vec::new(),
            desired: Vec::new(),
        }
    }

    pub fn available(&self) -> bool {
        self.manager.is_some()
    }

    pub fn add_seat(&mut self, seat: &WlSeat, id: SeatId, global: u32, qh: &QueueHandle<State>) {
        if let Some(manager) = &self.manager {
            self.inputs.push(Input {
                seat: id,
                global,
                object: manager.get_text_input(seat, qh, ()),
                focus: None,
                enabled: false,
                content: None,
                serial: 0,
                enable_serial: 0,
                blocked: false,
                dirty: false,
                composing: false,
                pending: Pending::default(),
            });
        }
    }

    pub fn remove_seat(&mut self, global: u32) {
        self.inputs.retain(|input| {
            let keep = input.global != global;
            if !keep {
                input.object.destroy();
            }
            keep
        });
    }

    pub fn remove_window(&mut self, window: WindowId) {
        self.desired.retain(|d| d.window != window);
        for input in &mut self.inputs {
            if input.focus == Some(window) {
                input.disable();
                input.leave();
            }
        }
    }

    pub fn configure(&mut self, window: WindowId, request: Option<ImeRequest>) -> Result<(), Error> {
        let next = request
            .map(|request| {
                let rect = caret(&request)?;
                if !self.available() {
                    return Err(Error::ImeUnavailable);
                }
                Ok(Desired {
                    window,
                    request,
                    rect,
                })
            })
            .transpose()?;
        let index = self.desired.iter().position(|d| d.window == window);
        let changed = match (index, next) {
            (Some(index), Some(next)) => {
                let changed = self.desired[index].request != next.request;
                self.desired[index] = next;
                changed
            }
            (Some(index), None) => {
                self.desired.swap_remove(index);
                true
            }
            (None, Some(next)) => {
                self.desired.push(next);
                true
            }
            (None, None) => false,
        };
        let desired = self.desired.iter().find(|d| d.window == window);
        for input in &mut self.inputs {
            if input.focus == Some(window) {
                input.dirty |= changed;
                // Disabling is immediate; updates wait for the input method.
                if input.dirty && (!input.blocked || desired.is_none()) {
                    input.send(desired);
                }
            }
        }
        Ok(())
    }
}

impl Drop for Ime {
    fn drop(&mut self) {
        for input in &self.inputs {
            input.object.destroy();
        }
        if let Some(manager) = &self.manager {
            manager.destroy();
        }
    }
}

impl Dispatch<ZwpTextInputV3, ()> for State {
    fn event(
        state: &mut Self,
        object: &ZwpTextInputV3,
        event: zwp_text_input_v3::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let ime = &mut state.ime;
        let Some(input) = ime.inputs.iter_mut().find(|i| &i.object == object) else {
            return;
        };
        let seat = input.seat;
        let ime_event = |window, event| Event::Ime {
            window,
            seat,
            event,
        };
        match event {
            zwp_text_input_v3::Event::Enter { surface } => {
                input.leave();
                input.focus = surface.data::<WindowId>().copied();
                if let Some(window) = input.focus {
                    input.send(ime.desired.iter().find(|d| d.window == window));
                    state.events.push_back(ime_event(window, ImeEvent::Entered));
                }
            }
            zwp_text_input_v3::Event::Leave { .. } => {
                if let Some(window) = input.leave() {
                    state.events.push_back(ime_event(window, ImeEvent::Left));
                }
            }
            zwp_text_input_v3::Event::PreeditString {
                text,
                cursor_begin,
                cursor_end,
            } => match preedit(text, cursor_begin, cursor_end) {
                Ok(preedit) => input.pending.preedit = preedit,
                Err(error) => input.pending.error = Some(error),
            },
            zwp_text_input_v3::Event::CommitString { text } => input.pending.commit = text,
            zwp_text_input_v3::Event::DeleteSurroundingText {
                before_length,
                after_length,
            } => {
                input.pending.before = before_length;
                input.pending.after = after_length;
            }
            zwp_text_input_v3::Event::Done { serial } => {
                let pending = std::mem::take(&mut input.pending);
                // Answers queued before a disable belong to the old editor.
                // Unsigned distances keep [enable, latest commit] valid when
                // the counter wraps.
                if input.content.is_none()
                    || input.focus.is_none()
                    || serial.wrapping_sub(input.enable_serial)
                        > input.serial.wrapping_sub(input.enable_serial)
                {
                    return;
                }
                input.blocked = serial != input.serial;
                // An empty answer only acknowledges what the input method was
                // told; reporting it would have the host tell it again, back
                // and forth forever. An empty preedit that ends a composition
                // is still news.
                let composing = !pending.preedit.text.is_empty();
                let ends = input.composing && !composing;
                input.composing = composing;
                let empty = pending.commit.is_none()
                    && pending.before == 0
                    && pending.after == 0
                    && pending.preedit.text.is_empty()
                    && pending.preedit.cursor.is_none();
                if let Some(error) = pending.error {
                    state
                        .events
                        .push_back(Event::Error(Error::InvalidIme(error)));
                } else if !empty || ends {
                    let window = input.focus.expect("checked above");
                    state.events.push_back(ime_event(
                        window,
                        ImeEvent::Update(ImeUpdate {
                            delete_before: pending.before as usize,
                            delete_after: pending.after as usize,
                            commit: pending.commit,
                            preedit: pending.preedit,
                        }),
                    ));
                }
                // The host answers with the editor's new state through
                // `configure`; resending the old one here would be stale.
            }
            _ => {}
        }
    }
}

delegate_noop!(State: ignore ZwpTextInputManagerV3);
