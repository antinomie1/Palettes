//! Typing, compose, input methods, the clipboard and drag and drop, driven
//! from a second client through virtual-keyboard, input-method-v2,
//! wlr-data-control and wlr-virtual-pointer.
//!
//! They type into the session and replace its clipboard, so they are
//! ignored by default. Run them under a headless Pastel:
//! `WAYLAND_DISPLAY=… cargo test -p palettes-shell --test input -- --ignored`.

use aegle_types::DragData;
use aegle_ui::DropEvent;
use aegle_widgets::Widgets;
use palettes_shell::{Anchor, KeyboardInteractivity, PanelOptions, Shell};
use std::{
    cell::RefCell,
    io::{Read, Write},
    os::fd::AsFd,
    rc::Rc,
    sync::mpsc,
    time::{Duration, Instant},
};
use wayland_client::{
    Connection, Dispatch, EventQueue, QueueHandle, delegate_noop, event_created_child,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{wl_pointer::ButtonState, wl_registry::WlRegistry, wl_seat::WlSeat},
};
use wayland_protocols_misc::{
    zwp_input_method_v2::client::{
        zwp_input_method_manager_v2::ZwpInputMethodManagerV2,
        zwp_input_method_v2::{self as im, ZwpInputMethodV2},
    },
    zwp_virtual_keyboard_v1::client::{
        zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1,
        zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1,
    },
};
use wayland_protocols_wlr::{
    data_control::v1::client::{
        zwlr_data_control_device_v1::{self as device, ZwlrDataControlDeviceV1},
        zwlr_data_control_manager_v1::ZwlrDataControlManagerV1,
        zwlr_data_control_offer_v1::ZwlrDataControlOfferV1,
        zwlr_data_control_source_v1::{self as source, ZwlrDataControlSourceV1},
    },
    virtual_pointer::v1::client::{
        zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1,
        zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1,
    },
};
use xkbcommon::xkb;

const TAB: u32 = 15;
const A: u32 = 30;
const E: u32 = 18;
const C: u32 = 46;
const V: u32 = 47;
/// `dead_acute` in the US international layout.
const APOSTROPHE: u32 = 40;
const CTRL: u32 = 29;

/// What the driving client has heard.
#[derive(Default)]
struct Heard {
    active: bool,
    dones: u32,
    /// Text for the input method to commit once active.
    commit: Option<String>,
    selection: Option<ZwlrDataControlOfferV1>,
    /// What the driver's clipboard source serves.
    serves: String,
}

/// A second client acting as keyboard, pointer, input method and clipboard
/// manager.
struct Driver {
    connection: Connection,
    queue: EventQueue<Heard>,
    heard: Heard,
    keyboard: ZwpVirtualKeyboardV1,
    pointer: ZwlrVirtualPointerV1,
    data: (ZwlrDataControlManagerV1, ZwlrDataControlDeviceV1),
    ctrl: u32,
    time: u32,
}

impl Driver {
    fn new() -> Option<Self> {
        let connection = Connection::connect_to_env().ok()?;
        let (globals, queue) = registry_queue_init::<Heard>(&connection).ok()?;
        let qh = queue.handle();
        let seat: WlSeat = globals.bind(&qh, 1..=7, ()).ok()?;
        let keyboards: ZwpVirtualKeyboardManagerV1 = globals.bind(&qh, 1..=1, ()).ok()?;
        let methods: ZwpInputMethodManagerV2 = globals.bind(&qh, 1..=1, ()).ok()?;
        let control: ZwlrDataControlManagerV1 = globals.bind(&qh, 1..=2, ()).ok()?;
        let pointers: ZwlrVirtualPointerManagerV1 = globals.bind(&qh, 1..=2, ()).ok()?;
        let keyboard = keyboards.create_virtual_keyboard(&seat, &qh, ());
        methods.get_input_method(&seat, &qh, ());
        let device = control.get_data_device(&seat, &qh, ());
        let pointer = pointers.create_virtual_pointer(Some(&seat), &qh, ());

        let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        let keymap =
            xkb::Keymap::new_from_names(&context, "evdev", "pc105", "us", "intl", None, 0)?;
        let ctrl = 1 << keymap.mod_get_index(xkb::MOD_NAME_CTRL);
        let text = keymap.get_as_string(xkb::KEYMAP_FORMAT_TEXT_V1);
        let fd = rustix::fs::memfd_create("keymap", rustix::fs::MemfdFlags::CLOEXEC).ok()?;
        let mut file = std::fs::File::from(fd);
        file.write_all(text.as_bytes()).ok()?;
        file.write_all(&[0]).ok()?;
        keyboard.keymap(1, file.as_fd(), text.len() as u32 + 1);
        Some(Self {
            connection,
            queue,
            heard: Heard::default(),
            keyboard,
            pointer,
            data: (control, device),
            ctrl,
            time: 0,
        })
    }

    fn pump(&mut self) {
        self.queue.roundtrip(&mut self.heard).unwrap();
    }

    fn tap(&mut self, key: u32, ctrl: bool) {
        self.time += 10;
        let (k, t) = (&self.keyboard, self.time);
        if ctrl {
            k.key(t, CTRL, 1);
            k.modifiers(self.ctrl, 0, 0, 0);
        }
        k.key(t, key, 1);
        k.key(t + 1, key, 0);
        if ctrl {
            k.key(t + 2, CTRL, 0);
            k.modifiers(0, 0, 0, 0);
        }
        self.connection.flush().unwrap();
    }

    fn point(&mut self, (x, y): (u32, u32), extent: (u32, u32)) {
        self.time += 10;
        self.pointer
            .motion_absolute(self.time, x, y, extent.0, extent.1);
        self.pointer.frame();
        self.connection.flush().unwrap();
    }

    fn button(&mut self, pressed: bool) {
        self.time += 10;
        let state = if pressed {
            ButtonState::Pressed
        } else {
            ButtonState::Released
        };
        self.pointer.button(self.time, 0x110, state);
        self.pointer.frame();
        self.connection.flush().unwrap();
    }

    /// Takes the clipboard, serving `text`.
    fn own_clipboard(&mut self, text: &str) {
        let source = self.data.0.create_data_source(&self.queue.handle(), ());
        source.offer("text/plain;charset=utf-8".into());
        self.data.1.set_selection(Some(&source));
        self.heard.serves = text.into();
        self.pump();
    }

    fn middle_click(&mut self) {
        for state in [ButtonState::Pressed, ButtonState::Released] {
            self.time += 10;
            self.pointer.button(self.time, 0x112, state);
            self.pointer.frame();
        }
        self.connection.flush().unwrap();
    }

    /// Reads the clipboard on a thread, as the other client serves it from
    /// the shell's loop.
    fn read_clipboard(&mut self) -> Option<mpsc::Receiver<String>> {
        let offer = self.heard.selection.take()?;
        let (read, write) = rustix::pipe::pipe().unwrap();
        offer.receive("text/plain;charset=utf-8".into(), write.as_fd());
        drop(write);
        self.connection.flush().unwrap();
        let (send, text) = mpsc::channel();
        std::thread::spawn(move || {
            let mut out = String::new();
            let _ = std::fs::File::from(read).read_to_string(&mut out);
            let _ = send.send(out);
        });
        Some(text)
    }
}

/// Runs the shell and the driver for `time`, or until `done`.
fn run(shell: &Shell, driver: &mut Driver, time: Duration, mut done: impl FnMut(&Driver) -> bool) {
    let deadline = Instant::now() + time;
    while Instant::now() < deadline && !done(driver) {
        shell.dispatch(Some(Duration::from_millis(10))).unwrap();
        driver.pump();
    }
}

fn session() -> Option<(Shell, Driver)> {
    if std::env::var_os("WAYLAND_DISPLAY").is_none() {
        eprintln!("skipped: no Wayland session");
        return None;
    }
    let Some(driver) = Driver::new() else {
        eprintln!(
            "skipped: needs virtual-keyboard, input-method-v2, data-control, virtual-pointer"
        );
        return None;
    };
    Some((Shell::new().unwrap(), driver))
}

#[test]
#[ignore = "types into the session and replaces its clipboard; run under a headless Pastel"]
fn typing_composing_input_methods_and_the_clipboard() {
    let Some((shell, mut driver)) = session() else {
        return;
    };
    let options = PanelOptions::top(48).keyboard(KeyboardInteractivity::Exclusive);
    let panel = shell.panel(options).unwrap();
    let field = panel.text_field("");
    field.set_grow(1.0);
    run(&shell, &mut driver, Duration::from_millis(800), |_| false);
    driver.tap(TAB, false);
    run(&shell, &mut driver, Duration::from_secs(2), |d| {
        d.heard.active
    });
    assert!(
        driver.heard.active,
        "text input enabled for the focused field"
    );

    driver.tap(A, false);
    // A dead key types nothing until the next key finishes the sequence.
    driver.tap(APOSTROPHE, false);
    run(&shell, &mut driver, Duration::from_millis(200), |_| false);
    assert_eq!(field.text(), "a");
    driver.tap(E, false);
    run(&shell, &mut driver, Duration::from_secs(1), |_| {
        field.text() == "aé"
    });
    assert_eq!(field.text(), "aé", "compose");

    driver.heard.commit = Some("中".into());
    driver.tap(A, false);
    run(&shell, &mut driver, Duration::from_secs(1), |_| {
        field.text().contains('中')
    });
    assert_eq!(field.text(), "aéa中", "the input method's commit");

    driver.own_clipboard("XY");
    driver.tap(V, true);
    run(&shell, &mut driver, Duration::from_secs(1), |_| {
        field.text().ends_with("XY")
    });
    assert_eq!(field.text(), "aéa中XY", "pasted from another client");

    driver.tap(A, true);
    driver.tap(C, true);
    // The shell's selection replaces the driver's.
    run(&shell, &mut driver, Duration::from_secs(1), |d| {
        d.heard.selection.is_some() && d.heard.serves.is_empty()
    });
    run(&shell, &mut driver, Duration::from_millis(300), |_| false);
    let read = driver
        .read_clipboard()
        .expect("the shell took the selection");
    let mut copied = None;
    run(&shell, &mut driver, Duration::from_secs(2), |_| {
        copied = read.try_recv().ok();
        copied.is_some()
    });
    assert_eq!(
        copied.as_deref(),
        Some("aéa中XY"),
        "copied for another client"
    );

    // Selecting made it the primary selection: a middle click in another
    // field pastes it there.
    let below = shell.panel(PanelOptions::bottom(48)).unwrap();
    let other = below.text_field("");
    other.set_grow(1.0);
    let output = shell.outputs()[0].clone();
    let extent = (
        (output.size.width as f32 / output.scale) as u32,
        (output.size.height as f32 / output.scale) as u32,
    );
    run(&shell, &mut driver, Duration::from_millis(300), |_| false);
    driver.point((extent.0 / 2, extent.1 - 24), extent);
    run(&shell, &mut driver, Duration::from_millis(200), |_| false);
    driver.middle_click();
    run(&shell, &mut driver, Duration::from_secs(2), |_| {
        !other.text().is_empty()
    });
    assert_eq!(
        other.text(),
        "aéa中XY",
        "a middle click pastes the primary selection"
    );
}

#[test]
#[ignore = "drags with the session's pointer; run under a headless Pastel"]
fn drags_text_between_panels() {
    let Some((shell, mut driver)) = session() else {
        return;
    };
    let output = shell.outputs()[0].clone();
    let extent = (output.size.width, output.size.height);
    let place = |anchor| PanelOptions::floating(anchor, 200, 100);
    let from = shell.panel(place(Anchor::TOP | Anchor::LEFT)).unwrap();
    let to = shell.panel(place(Anchor::TOP | Anchor::RIGHT)).unwrap();
    let dropped = Rc::new(RefCell::new(Vec::new()));
    let seen = dropped.clone();
    let _target = to.on_drop(move |_, event| seen.borrow_mut().push(event));
    run(&shell, &mut driver, Duration::from_millis(800), |_| false);

    driver.point((100, 50), extent);
    driver.button(true);
    run(&shell, &mut driver, Duration::from_millis(200), |_| false);
    from.start_drag(DragData::Text("dragged".into()));
    run(&shell, &mut driver, Duration::from_millis(200), |_| false);
    for step in 1..=10 {
        let x = 100 + (extent.0 - 200) * step / 10;
        driver.point((x, 50), extent);
        run(&shell, &mut driver, Duration::from_millis(40), |_| false);
    }
    driver.button(false);
    run(&shell, &mut driver, Duration::from_secs(2), |_| {
        dropped
            .borrow()
            .iter()
            .any(|e| matches!(e, DropEvent::Drop { .. }))
    });
    let events = dropped.borrow();
    assert_eq!(events.first(), Some(&DropEvent::Enter));
    assert!(
        events.iter().any(|e| matches!(
            e,
            DropEvent::Drop { data: DragData::Text(text), .. } if text == "dragged"
        )),
        "{events:?}"
    );
}

impl Dispatch<ZwpInputMethodV2, ()> for Heard {
    fn event(
        heard: &mut Self,
        method: &ZwpInputMethodV2,
        event: im::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            im::Event::Activate => heard.active = true,
            im::Event::Deactivate => heard.active = false,
            im::Event::Done => {
                heard.dones += 1;
                if heard.active
                    && let Some(text) = heard.commit.take()
                {
                    method.commit_string(text);
                    method.commit(heard.dones);
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<ZwlrDataControlDeviceV1, ()> for Heard {
    fn event(
        heard: &mut Self,
        _: &ZwlrDataControlDeviceV1,
        event: device::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let device::Event::Selection { id } = event {
            heard.selection = id;
        }
    }

    event_created_child!(Heard, ZwlrDataControlDeviceV1, [
        device::EVT_DATA_OFFER_OPCODE => (ZwlrDataControlOfferV1, ()),
    ]);
}

impl Dispatch<ZwlrDataControlSourceV1, ()> for Heard {
    fn event(
        heard: &mut Self,
        _: &ZwlrDataControlSourceV1,
        event: source::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            source::Event::Send { fd, .. } => {
                let _ = std::fs::File::from(fd).write_all(heard.serves.as_bytes());
            }
            // The shell took the selection.
            source::Event::Cancelled => heard.serves.clear(),
            _ => {}
        }
    }
}

impl Dispatch<WlRegistry, GlobalListContents> for Heard {
    fn event(
        _: &mut Self,
        _: &WlRegistry,
        _: wayland_client::protocol::wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

delegate_noop!(Heard: ignore WlSeat);
delegate_noop!(Heard: ZwpVirtualKeyboardManagerV1);
delegate_noop!(Heard: ZwpVirtualKeyboardV1);
delegate_noop!(Heard: ZwpInputMethodManagerV2);
delegate_noop!(Heard: ZwlrDataControlManagerV1);
delegate_noop!(Heard: ignore ZwlrDataControlOfferV1);
delegate_noop!(Heard: ZwlrVirtualPointerManagerV1);
delegate_noop!(Heard: ZwlrVirtualPointerV1);
