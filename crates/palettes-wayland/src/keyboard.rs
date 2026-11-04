//! The keyboard: xkbcommon keymaps, translation into aegle keys and text,
//! compose sequences (dead keys, the Compose key) from the locale's table,
//! and key repeat, which the loop times through [`State::next_repeat`].
#![allow(unsafe_code)]

use crate::state::State;
use aegle_platform::{Event, Key, Modifiers, WindowId};
use std::time::{Duration, Instant};
use wayland_client::{
    Connection, Dispatch, Proxy, QueueHandle, WEnum,
    protocol::{
        wl_keyboard::{self, KeyState, KeymapFormat, WlKeyboard},
        wl_seat::WlSeat,
    },
};
use xkbcommon::xkb::{self, compose, keysyms};

struct Repeat {
    key: u32,
    next: Instant,
    time: u32,
    /// What the first press typed; a repeat types it again rather than
    /// feeding the compose sequence anew.
    text: Option<String>,
}

pub(crate) struct Keyboard {
    keyboard: Option<WlKeyboard>,
    context: xkb::Context,
    state: Option<xkb::State>,
    /// `None` where the locale has no compose table.
    compose: Option<compose::State>,
    focus: Option<WindowId>,
    modifiers: Modifiers,
    repeat: Option<Repeat>,
    /// Keys per second; zero disables repeat.
    rate: i32,
    delay: Duration,
}

impl Default for Keyboard {
    fn default() -> Self {
        Self {
            keyboard: None,
            context: xkb::Context::new(xkb::CONTEXT_NO_FLAGS),
            state: None,
            compose: None,
            focus: None,
            modifiers: Modifiers::default(),
            repeat: None,
            rate: 25,
            delay: Duration::from_millis(600),
        }
    }
}

/// The logical key of a keysym; printable keys by their unshifted character.
pub fn logical_key(sym: u32) -> Key {
    match sym {
        keysyms::KEY_Return | keysyms::KEY_KP_Enter | keysyms::KEY_ISO_Enter => Key::Enter,
        keysyms::KEY_Tab | keysyms::KEY_ISO_Left_Tab => Key::Tab,
        keysyms::KEY_Escape => Key::Escape,
        keysyms::KEY_BackSpace => Key::Backspace,
        keysyms::KEY_Delete | keysyms::KEY_KP_Delete => Key::Delete,
        keysyms::KEY_Left | keysyms::KEY_KP_Left => Key::Left,
        keysyms::KEY_Right | keysyms::KEY_KP_Right => Key::Right,
        keysyms::KEY_Up | keysyms::KEY_KP_Up => Key::Up,
        keysyms::KEY_Down | keysyms::KEY_KP_Down => Key::Down,
        keysyms::KEY_Home | keysyms::KEY_KP_Home => Key::Home,
        keysyms::KEY_End | keysyms::KEY_KP_End => Key::End,
        keysyms::KEY_Page_Up | keysyms::KEY_KP_Page_Up => Key::PageUp,
        keysyms::KEY_Page_Down | keysyms::KEY_KP_Page_Down => Key::PageDown,
        keysyms::KEY_Menu => Key::ContextMenu,
        keysyms::KEY_F1..=keysyms::KEY_F35 => Key::Function((sym - keysyms::KEY_F1 + 1) as u8),
        _ => match char::from_u32(xkb::keysym_to_utf32(xkb::Keysym::new(sym))) {
            Some(c) if !c.is_control() && c != '\0' => {
                Key::Character(c.to_lowercase().next().unwrap_or(c))
            }
            _ => Key::Unidentified,
        },
    }
}

impl Keyboard {
    pub fn set_enabled(&mut self, on: bool, seat: &WlSeat, global: u32, qh: &QueueHandle<State>) {
        match (on, self.keyboard.is_some()) {
            (true, false) => self.keyboard = Some(seat.get_keyboard(qh, global)),
            (false, true) => self.release(),
            _ => {}
        }
    }

    pub fn release(&mut self) {
        if let Some(keyboard) = self.keyboard.take()
            && keyboard.version() >= 3
        {
            keyboard.release();
        }
        self.focus = None;
        self.repeat = None;
    }

    /// The compose table of the user's locale, as `setlocale` would pick it.
    fn load_compose(&mut self) {
        let locale = ["LC_ALL", "LC_CTYPE", "LANG"]
            .into_iter()
            .filter_map(std::env::var_os)
            .find(|v| !v.is_empty())
            .unwrap_or_else(|| "C".into());
        self.compose = compose::Table::new_from_locale(
            &self.context,
            &locale,
            compose::COMPILE_NO_FLAGS,
        )
        .ok()
        .map(|table| compose::State::new(&table, compose::STATE_NO_FLAGS));
    }

    /// The text a press types: what a compose sequence finishes, nothing
    /// while one is under way or after it is cancelled, else the key's own.
    fn press_text(&mut self, key: u32) -> Option<String> {
        let state = self.state.as_ref()?;
        let code = xkb::Keycode::new(key + 8);
        let text = if let Some(compose) = self.compose.as_mut()
            && compose.feed(state.key_get_one_sym(code)) == compose::FeedResult::Accepted
        {
            match compose.status() {
                compose::Status::Composing => None,
                compose::Status::Composed => {
                    let text = compose.utf8();
                    compose.reset();
                    text
                }
                compose::Status::Cancelled => {
                    compose.reset();
                    None
                }
                compose::Status::Nothing => Some(state.key_get_utf8(code)),
            }
        } else {
            Some(state.key_get_utf8(code))
        };
        text.filter(|t| !t.is_empty() && !t.chars().any(char::is_control))
    }

    pub fn forget(&mut self, id: WindowId) {
        if self.focus == Some(id) {
            self.focus = None;
            self.repeat = None;
        }
    }

    fn read_modifiers(&self) -> Modifiers {
        let Some(state) = &self.state else {
            return Modifiers::default();
        };
        let active = |name| state.mod_name_is_active(name, xkb::STATE_MODS_EFFECTIVE);
        Modifiers {
            shift: active(xkb::MOD_NAME_SHIFT),
            control: active(xkb::MOD_NAME_CTRL),
            alt: active(xkb::MOD_NAME_ALT),
            meta: active(xkb::MOD_NAME_LOGO),
        }
    }

    /// The key event for evdev `key` typing `text`, or `None` before a
    /// keymap or focus.
    fn event(
        &self,
        seat: aegle_platform::SeatId,
        key: u32,
        pressed: bool,
        text: Option<String>,
        repeat: bool,
        time: u32,
    ) -> Option<Event> {
        let (state, window) = (self.state.as_ref()?, self.focus?);
        let sym = state.key_get_one_sym(xkb::Keycode::new(key + 8)).raw();
        Some(Event::Key {
            window,
            seat,
            key: logical_key(sym),
            text,
            pressed,
            repeat,
            modifiers: self.modifiers,
            time,
        })
    }
}

impl State {
    /// When the next held key repeats, if one does.
    pub fn next_repeat(&self) -> Option<Instant> {
        self.seats
            .iter()
            .filter_map(|s| s.keyboard.repeat.as_ref().map(|r| r.next))
            .min()
    }

    /// Queues the repeats that are due.
    pub fn fire_repeats(&mut self, now: Instant) {
        for seat in &mut self.seats {
            let keyboard = &mut seat.keyboard;
            let Some(repeat) = keyboard.repeat.as_mut() else {
                continue;
            };
            if repeat.next > now || keyboard.rate <= 0 {
                continue;
            }
            let interval = Duration::from_secs(1) / keyboard.rate as u32;
            // Late wakes (a busy frame) repeat once, not in a burst.
            repeat.next = (repeat.next + interval).max(now);
            repeat.time = repeat.time.wrapping_add(interval.as_millis() as u32);
            let (key, time, text) = (repeat.key, repeat.time, repeat.text.clone());
            if let Some(event) = keyboard.event(seat.id, key, true, text, true, time) {
                self.events.push_back(event);
            }
        }
    }
}

impl Dispatch<WlKeyboard, u32> for State {
    fn event(
        state: &mut Self,
        _: &WlKeyboard,
        event: wl_keyboard::Event,
        global: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(seat) = state.seats.iter_mut().find(|s| s.global == *global) else {
            return;
        };
        let (id, keyboard) = (seat.id, &mut seat.keyboard);
        match event {
            wl_keyboard::Event::Keymap { format, fd, size } => {
                if format != WEnum::Value(KeymapFormat::XkbV1) {
                    return;
                }
                // SAFETY: the compositor hands over a sealed, read-only keymap
                // file of `size` bytes; xkbcommon maps it privately.
                let keymap = unsafe {
                    xkb::Keymap::new_from_fd(
                        &keyboard.context,
                        fd,
                        size as usize,
                        xkb::KEYMAP_FORMAT_TEXT_V1,
                        xkb::KEYMAP_COMPILE_NO_FLAGS,
                    )
                };
                keyboard.state = keymap.ok().flatten().map(|keymap| xkb::State::new(&keymap));
                if keyboard.compose.is_none() {
                    keyboard.load_compose();
                }
            }
            wl_keyboard::Event::Enter {
                serial, surface, ..
            } => {
                let Some(window) = surface.data::<WindowId>().copied() else {
                    return;
                };
                seat.last_serial = serial;
                keyboard.focus = Some(window);
                state.events.push_back(Event::KeyboardFocus {
                    window,
                    seat: id,
                    focused: true,
                });
            }
            wl_keyboard::Event::Leave { .. } => {
                keyboard.repeat = None;
                // A sequence begun in one window does not finish in another.
                if let Some(compose) = keyboard.compose.as_mut() {
                    compose.reset();
                }
                if let Some(window) = keyboard.focus.take() {
                    state.events.push_back(Event::KeyboardFocus {
                        window,
                        seat: id,
                        focused: false,
                    });
                }
            }
            wl_keyboard::Event::Key {
                serial,
                time,
                key,
                state: WEnum::Value(key_state),
            } => {
                let pressed = key_state == KeyState::Pressed;
                if pressed {
                    seat.last_serial = serial;
                }
                let repeats = keyboard
                    .state
                    .as_ref()
                    .is_some_and(|s| s.get_keymap().key_repeats(xkb::Keycode::new(key + 8)));
                let text = pressed.then(|| keyboard.press_text(key)).flatten();
                if pressed && repeats && keyboard.rate > 0 {
                    keyboard.repeat = Some(Repeat {
                        key,
                        next: Instant::now() + keyboard.delay,
                        time,
                        text: text.clone(),
                    });
                } else if keyboard.repeat.as_ref().is_some_and(|r| r.key == key) {
                    keyboard.repeat = None;
                }
                if let Some(event) = keyboard.event(id, key, pressed, text, false, time) {
                    state.events.push_back(event);
                }
            }
            wl_keyboard::Event::Modifiers {
                mods_depressed,
                mods_latched,
                mods_locked,
                group,
                ..
            } => {
                if let Some(xkb_state) = keyboard.state.as_mut() {
                    xkb_state.update_mask(mods_depressed, mods_latched, mods_locked, 0, 0, group);
                }
                keyboard.modifiers = keyboard.read_modifiers();
                if let Some(window) = keyboard.focus {
                    state.events.push_back(Event::Modifiers {
                        window,
                        seat: id,
                        modifiers: keyboard.modifiers,
                    });
                }
            }
            wl_keyboard::Event::RepeatInfo { rate, delay } => {
                keyboard.rate = rate;
                keyboard.delay = Duration::from_millis(delay.max(0) as u64);
            }
            _ => {}
        }
    }
}
