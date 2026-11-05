//! A `movable` panel follows a drag that starts on no control and stays
//! where it is dropped, while its controls still take clicks. It moves the
//! pointer, so it is ignored by default; run it against a headless Pastel:
//! `cargo test -p palettes --test moving -- --ignored`.

use palettes::{Shell, ipc, markup};
use std::time::{Duration, Instant};
use wayland_client::{
    Connection, Dispatch, QueueHandle, delegate_noop,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{wl_pointer::ButtonState, wl_registry::WlRegistry, wl_seat::WlSeat},
};
use wayland_protocols_wlr::virtual_pointer::v1::client::{
    zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1,
    zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1,
};

struct Pointer;

impl Dispatch<WlRegistry, GlobalListContents> for Pointer {
    fn event(
        _: &mut Self,
        _: &WlRegistry,
        _: <WlRegistry as wayland_client::Proxy>::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
delegate_noop!(Pointer: ignore WlSeat);
delegate_noop!(Pointer: ZwlrVirtualPointerManagerV1);
delegate_noop!(Pointer: ZwlrVirtualPointerV1);

#[test]
#[ignore = "moves the pointer; run against a headless Pastel"]
fn a_movable_panel_follows_a_drag_from_no_control() {
    let dir = std::env::temp_dir().join(format!("palettes-moving-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // SAFETY: the only test in this binary.
    unsafe { std::env::set_var("PALETTES_SOCKET", dir.join("ipc.sock")) };
    let panel = dir.join("panel.aegle");
    std::fs::write(
        &panel,
        r#"Panel { edge: center; span: 200dp; size: 100dp; padding: 0dp; movable: true
            Row { Button { text: "hit"; min_width: 60dp; min_height: 30dp; on clicked { host.hit() } } } }"#,
    )
    .unwrap();
    std::fs::write(
        dir.join("panel.luau"),
        "local n = 0\nfunction hit() n += 1 end\nfunction hits() return tostring(n) end\n",
    )
    .unwrap();

    let connection = Connection::connect_to_env().unwrap();
    let (globals, mut queue) = registry_queue_init::<Pointer>(&connection).unwrap();
    let qh = queue.handle();
    let seat: WlSeat = globals.bind(&qh, 1..=7, ()).unwrap();
    let manager: ZwlrVirtualPointerManagerV1 = globals.bind(&qh, 1..=2, ()).unwrap();
    let pointer = manager.create_virtual_pointer(Some(&seat), &qh, ());
    queue.roundtrip(&mut Pointer).unwrap();

    let shell = Shell::new().unwrap();
    let output = shell.outputs()[0].clone();
    let extent = (
        (output.size.width as f32 / output.scale) as u32,
        (output.size.height as f32 / output.scale) as u32,
    );
    let documents = markup::Documents::with(shell.clone(), markup::elements::elements())
        .unwrap()
        .open(&panel)
        .unwrap();
    let settle = |time: u64| {
        let end = Instant::now() + Duration::from_millis(time);
        while Instant::now() < end {
            shell.dispatch(Some(Duration::from_millis(5))).unwrap();
        }
    };
    let ask = |name: &str| {
        let request = vec![name.to_owned()];
        let asking = std::thread::spawn(move || ipc::send(&request).map_err(|e| e.to_string()));
        while !asking.is_finished() {
            shell.dispatch(Some(Duration::from_millis(10))).unwrap();
        }
        asking.join().unwrap()
    };
    let mut time = 0;
    let mut act = |x: u32, y: u32, state: Option<ButtonState>, wait: u64| {
        time += 10;
        pointer.motion_absolute(time, x, y, extent.0, extent.1);
        if let Some(state) = state {
            pointer.button(time, 0x110, state);
        }
        pointer.frame();
        connection.flush().unwrap();
        settle(wait);
    };
    settle(300);
    // Centered at first; the button fills its top-left corner.
    let (x, y) = ((extent.0 - 200) / 2, (extent.1 - 100) / 2);
    act(x + 100, y + 70, None, 50);
    act(x + 100, y + 70, Some(ButtonState::Pressed), 50);
    for step in 1..=10 {
        act(x + 100 + step * 15, y + 70 + step * 8, None, 40);
    }
    act(x + 250, y + 150, Some(ButtonState::Released), 200);
    let (x, y) = (x + 150, y + 80);

    let click = |act: &mut dyn FnMut(u32, u32, Option<ButtonState>, u64), at: (u32, u32)| {
        act(at.0, at.1, None, 50);
        act(at.0, at.1, Some(ButtonState::Pressed), 50);
        act(at.0, at.1, Some(ButtonState::Released), 150);
    };
    click(&mut act, (x + 15, y + 12));
    assert_eq!(
        ask("hits"),
        Ok("1".into()),
        "the button moved with the panel"
    );
    click(&mut act, (x - 150 + 15, y - 80 + 12));
    assert_eq!(ask("hits"), Ok("1".into()), "nothing is left where it was");
    // A press on a control is a click, not a move.
    act(x + 15, y + 12, Some(ButtonState::Pressed), 50);
    act(x + 45, y + 22, None, 50);
    act(x + 45, y + 22, Some(ButtonState::Released), 150);
    click(&mut act, (x + 15, y + 12));
    assert_eq!(ask("hits"), Ok("3".into()), "the panel stayed put");
    drop(documents);
}
