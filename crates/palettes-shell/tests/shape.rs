//! Input follows a surface's rounded shape: a click in a corner reaches the
//! surface below. Needs a compositor with wlr-virtual-pointer (Pastel, sway),
//! skipped elsewhere.

use aegle_widgets::Widgets;
use palettes_shell::{Anchor, Layer, PanelOptions, Shell, Surface};
use std::{
    cell::Cell,
    rc::Rc,
    time::{Duration, Instant},
};
use wayland_client::{
    Connection, Dispatch, QueueHandle, delegate_noop, globals::GlobalListContents,
    globals::registry_queue_init, protocol::wl_registry::WlRegistry,
};
use wayland_protocols_wlr::virtual_pointer::v1::client::{
    zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1,
    zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1,
};

struct Client;
impl Dispatch<WlRegistry, GlobalListContents> for Client {
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
delegate_noop!(Client: ignore ZwlrVirtualPointerManagerV1);
delegate_noop!(Client: ignore ZwlrVirtualPointerV1);

/// A virtual pointer on its own connection, clicking in output coordinates.
struct Pointer {
    connection: Connection,
    pointer: ZwlrVirtualPointerV1,
    extent: (u32, u32),
}

impl Pointer {
    fn new(extent: (u32, u32)) -> Option<Self> {
        let connection = Connection::connect_to_env().ok()?;
        let (globals, queue) = registry_queue_init::<Client>(&connection).ok()?;
        let manager: ZwlrVirtualPointerManagerV1 = globals.bind(&queue.handle(), 1..=2, ()).ok()?;
        let pointer = manager.create_virtual_pointer(None, &queue.handle(), ());
        Some(Self {
            connection,
            pointer,
            extent,
        })
    }

    fn click(&self, x: u32, y: u32) {
        use wayland_client::protocol::wl_pointer::ButtonState;
        const LEFT: u32 = 0x110;
        let p = &self.pointer;
        p.motion_absolute(0, x, y, self.extent.0, self.extent.1);
        p.frame();
        p.button(1, LEFT, ButtonState::Pressed);
        p.frame();
        p.button(2, LEFT, ButtonState::Released);
        p.frame();
        self.connection.flush().unwrap();
    }
}

/// A 200 × 100 panel in the top-left corner, all of it one counting button.
fn counter(shell: &Shell, layer: Layer) -> (Surface, Rc<Cell<u32>>) {
    let options = PanelOptions::floating(Anchor::TOP | Anchor::LEFT, 200, 100).layer(layer);
    let panel = shell.panel(options).unwrap();
    panel.set_padding(0.0);
    let button = panel.button("");
    button.set_grow(1.0);
    let clicks = Rc::new(Cell::new(0));
    let counted = clicks.clone();
    button.on_click(move |_| counted.set(counted.get() + 1));
    (panel, clicks)
}

fn run(shell: &Shell, time: Duration) {
    let deadline = Instant::now() + time;
    while Instant::now() < deadline {
        shell.dispatch(Some(Duration::from_millis(20))).unwrap();
    }
}

#[test]
fn clicks_outside_rounded_corners_go_through() {
    if std::env::var_os("WAYLAND_DISPLAY").is_none() {
        eprintln!("skipped: no Wayland session");
        return;
    }
    let shell = Shell::new().unwrap();
    let output = shell.outputs()[0].clone();
    let Some(pointer) = Pointer::new((output.size.width, output.size.height)) else {
        eprintln!("skipped: no wlr-virtual-pointer");
        return;
    };
    let (below, under) = counter(&shell, Layer::Bottom);
    let (above, over) = counter(&shell, Layer::Top);
    above.set_radius(40.0);
    run(&shell, Duration::from_millis(500));

    pointer.click(100, 50);
    run(&shell, Duration::from_millis(300));
    assert_eq!(
        (over.get(), under.get()),
        (1, 0),
        "the middle is the top panel's"
    );

    pointer.click(3, 3);
    run(&shell, Duration::from_millis(300));
    assert_eq!(
        (over.get(), under.get()),
        (1, 1),
        "the corner is the one below's"
    );

    // Shape nodes replace the root's: an empty one takes no input at all,
    // so even the middle goes through.
    let nothing = above.column();
    nothing.set_height(0.0);
    above.set_input(&nothing, true).unwrap();
    run(&shell, Duration::from_millis(300));
    pointer.click(100, 50);
    run(&shell, Duration::from_millis(300));
    assert_eq!(
        (over.get(), under.get()),
        (1, 2),
        "nothing takes the middle"
    );

    // Removing it brings back the root's rounded shape.
    above.set_input(&nothing, false).unwrap();
    run(&shell, Duration::from_millis(300));
    pointer.click(100, 50);
    run(&shell, Duration::from_millis(300));
    assert_eq!(
        (over.get(), under.get()),
        (2, 2),
        "the middle is the top panel's again"
    );
    below.close().unwrap();
    above.close().unwrap();
}
