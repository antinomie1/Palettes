//! A popup document opens beside the button that asked for it, runs its
//! script while open, and closes with a click outside. It moves the pointer,
//! so it is ignored by default; run it against a headless Pastel:
//! `cargo test -p palettes --test popup -- --ignored`.

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
fn a_popup_opens_by_its_button_and_closes_with_a_click_outside() {
    let dir = std::env::temp_dir().join(format!("palettes-popup-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // SAFETY: the only test in this binary.
    unsafe { std::env::set_var("PALETTES_SOCKET", dir.join("ipc.sock")) };
    let bar = dir.join("bar.aegle");
    std::fs::write(
        &bar,
        r#"Panel { size: 40dp; Row { Button { text: "menu"; on clicked { host.toggle("menu.aegle") } } } }"#,
    )
    .unwrap();
    std::fs::write(
        dir.join("menu.aegle"),
        r#"Panel { edge: popup; size: 100dp; span: 200dp; Text { text: "menu" } }"#,
    )
    .unwrap();
    std::fs::write(
        dir.join("menu.luau"),
        "function menu() return \"open\" end\n",
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
        .open(&bar)
        .unwrap();
    let settle = |time: u64| {
        let end = Instant::now() + Duration::from_millis(time);
        while Instant::now() < end {
            shell.dispatch(Some(Duration::from_millis(10))).unwrap();
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
    let mut click = |(x, y): (u32, u32)| {
        for state in [
            None,
            Some(ButtonState::Pressed),
            Some(ButtonState::Released),
        ] {
            time += 20;
            match state {
                None => pointer.motion_absolute(time, x, y, extent.0, extent.1),
                Some(state) => pointer.button(time, 0x110, state),
            }
            pointer.frame();
            connection.flush().unwrap();
            settle(100);
        }
    };
    settle(300);
    click((20, 20));
    settle(200);
    assert_eq!(ask("menu"), Ok("open".into()), "the popup's script runs");
    click((extent.0 / 2, extent.1 / 2));
    settle(200);
    let gone = ask("menu").unwrap_err();
    assert!(
        gone.contains("no action"),
        "dismissing stops the script: {gone}"
    );
    click((20, 20));
    settle(200);
    assert_eq!(ask("menu"), Ok("open".into()), "it opens again");
    drop(documents);
}
