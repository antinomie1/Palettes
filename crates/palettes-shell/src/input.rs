//! Platform input into a surface's tree, and the tree's clipboard, drag and
//! input method requests back to the platform. Each surface has one focus domain,
//! owned by the seat that last focused it.

use crate::runtime::Entry;
use aegle_platform::{Event, ImeEvent, Platform, PointerKind as Pointer, SeatId};
use aegle_ui::{
    ClipboardRequest, ImeEdit, Key, KeyInput, Modifiers, Point, PointerId, PointerKind, Result,
    Selection, Size,
};
use palettes_wayland::Wayland;
use std::time::{Duration, Instant};

/// Maps wrapping platform milliseconds onto [`Instant`], never ahead of now.
#[derive(Default)]
pub(crate) struct EventClock {
    anchor: Option<(Instant, u32)>,
}

impl EventClock {
    pub fn at(&mut self, time: u32) -> Instant {
        let now = Instant::now();
        let at = match self.anchor {
            Some((anchor, base)) => {
                let delta = time.wrapping_sub(base) as i32;
                let offset = Duration::from_millis(u64::from(delta.unsigned_abs()));
                let at = if delta >= 0 {
                    anchor.checked_add(offset)
                } else {
                    anchor.checked_sub(offset)
                };
                at.map_or(now, |at| at.min(now))
            }
            None => now,
        };
        self.anchor = Some((at, time));
        at
    }
}

impl Entry {
    fn adopt(&mut self, seat: aegle_platform::SeatId) -> bool {
        *self.seat.get_or_insert(seat) == seat
    }

    /// Momentum from the last 100 ms of finger scrolling.
    fn release_flick(&mut self, time: u32, position: Point) {
        let samples = std::mem::take(&mut self.flick);
        let recent: Vec<_> = samples
            .iter()
            .filter(|(at, _)| time.wrapping_sub(*at) <= 100)
            .collect();
        let (Some(first), Some(last)) = (recent.first(), recent.last()) else {
            return;
        };
        let span = last.0.wrapping_sub(first.0) as f32 / 1000.0;
        if recent.len() < 2 || span <= 0.0 {
            return;
        }
        let (x, y) = recent[1..]
            .iter()
            .fold((0.0, 0.0), |(x, y), (_, d)| (x + d.x, y + d.y));
        self.ui.fling(position, Point::new(x / span, y / span));
    }

    pub fn event(&mut self, event: Event) -> Result<()> {
        match event {
            Event::Configure { info, .. } => {
                self.fitting = None;
                self.ui
                    .resize(Size::new(info.size.width as f32, info.size.height as f32));
            }
            Event::Redraw { .. } => self.ready = true,
            Event::KeyboardFocus { seat, focused, .. } => {
                if focused {
                    if self.seat != Some(seat) {
                        self.ui.window_focus(false)?;
                        self.seat = Some(seat);
                        self.modifiers = Modifiers::default();
                    }
                    self.ui.window_focus(true)?;
                } else if self.seat == Some(seat) {
                    self.ui.window_focus(false)?;
                    self.seat = None;
                }
            }
            Event::Key {
                seat,
                key,
                text,
                pressed,
                repeat,
                modifiers,
                time,
                ..
            } if self.seat == Some(seat) => {
                self.modifiers = modifiers;
                let at = self.clock.at(time);
                let input = KeyInput {
                    key,
                    text: text.as_deref().unwrap_or(""),
                    modifiers,
                    pressed,
                    repeat,
                };
                self.ui.key_at(input, at)?;
            }
            Event::Text { seat, text, .. } if self.seat == Some(seat) => {
                self.ui.key(KeyInput {
                    key: Key::Unidentified,
                    text: &text,
                    modifiers: Modifiers::default(),
                    pressed: true,
                    repeat: false,
                })?;
            }
            Event::Modifiers {
                seat, modifiers, ..
            } if self.adopt(seat) => self.modifiers = modifiers,
            Event::Pointer {
                seat,
                position,
                kind,
                time,
                ..
            } if self.adopt(seat) => {
                let at = time.map_or_else(Instant::now, |time| self.clock.at(time));
                let kind = match kind {
                    Pointer::Move => PointerKind::Move,
                    Pointer::Down { clicks } => PointerKind::Down { clicks },
                    Pointer::Up => PointerKind::Up,
                    Pointer::ButtonDown(button) => PointerKind::ButtonDown(button),
                    Pointer::ButtonUp(button) => PointerKind::ButtonUp(button),
                    Pointer::Leave => return self.ui.pointer_leave(),
                    Pointer::Scroll { delta } => {
                        if let Some(time) = time {
                            if self.flick.len() == 8 {
                                self.flick.remove(0);
                            }
                            self.flick.push((time, delta));
                        }
                        return self.ui.wheel(position, delta, self.modifiers, at);
                    }
                    Pointer::ScrollEnd => {
                        if let Some(time) = time {
                            self.release_flick(time, position);
                        }
                        return Ok(());
                    }
                };
                self.ui.pointer_at(
                    PointerId(u64::from(seat.0)),
                    kind,
                    position,
                    self.modifiers,
                    at,
                )?;
            }
            Event::Ime { seat, event, .. } => {
                // Text-input focus counts even before a key from the seat,
                // as with an on-screen keyboard.
                if self.seat.is_none() && event == ImeEvent::Entered {
                    self.seat = Some(seat);
                    self.ui.window_focus(true)?;
                }
                if self.seat != Some(seat) {
                    return Ok(());
                }
                match event {
                    ImeEvent::Update(update) => self.ui.ime(ImeEdit {
                        delete_before: update.delete_before,
                        delete_after: update.delete_after,
                        commit: update.commit.as_deref(),
                        preedit: &update.preedit.text,
                        cursor: (update.preedit.cursor)
                            .map(|(anchor, focus)| Selection { anchor, focus }),
                    })?,
                    ImeEvent::Left => self.ui.ime_left()?,
                    ImeEvent::Entered => self.ui.request_ime_sync(),
                }
            }
            Event::Clipboard { seat, text, .. } if self.seat == Some(seat) => {
                self.ui.paste(&text)?;
            }
            Event::DragLeave { .. } => self.ui.drag_leave(),
            Event::Drop { position, data, .. } => {
                self.ui.drop_data(position, data);
            }
            Event::Touch {
                seat,
                finger,
                position,
                time,
                phase,
                ..
            } => {
                // Fingers are pointers of their own.
                let pointer =
                    PointerId((1 << 40) | (u64::from(seat.0) << 16) | u64::from(finger as u16));
                self.ui.touch(pointer, phase, position, time)?;
            }
            _ => {}
        }
        Ok(())
    }
}

/// Hands the tree's clipboard, drag and input method requests to the
/// platform. A compositor without them only loses the feature, so failures
/// are reported, not fatal.
pub(crate) fn sync_requests(platform: &mut Wayland, entry: &Entry) {
    let seat = entry.seat.unwrap_or(SeatId(0));
    let report = |what: &str, result: aegle_platform::Result<()>| {
        if let Err(error) = result {
            eprintln!("palettes: {what}: {error}");
        }
    };
    match entry.ui.take_clipboard() {
        Some(ClipboardRequest::Write(text)) => {
            report("clipboard", platform.set_clipboard(entry.id, seat, &text));
        }
        Some(ClipboardRequest::Read) => {
            report("clipboard", platform.request_clipboard(entry.id, seat));
        }
        None => {}
    }
    if let Some(data) = entry.ui.take_drag() {
        report("drag", platform.start_drag(entry.id, seat, &data));
    }
    let Some(ime) = entry.ui.take_ime_state(aegle_platform::MAX_SURROUNDING) else {
        return;
    };
    if !platform.ime_available() {
        return;
    }
    if ime.reset {
        report("input method", platform.configure_ime(entry.id, None));
    }
    let request = ime.request.map(|r| aegle_platform::ImeRequest {
        surrounding: r.surrounding,
        cursor: r.selection.focus,
        anchor: r.selection.anchor,
        cursor_rect: r.cursor_rect,
        multiline: r.multiline,
        input_method: r.input_method,
    });
    report("input method", platform.configure_ime(entry.id, request));
}
