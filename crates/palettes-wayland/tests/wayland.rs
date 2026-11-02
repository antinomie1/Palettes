//! Key translation everywhere; surfaces only inside a Wayland session with
//! wlr-layer-shell (KDE, Pastel, sway, niri...), skipped elsewhere.

use aegle_platform::{
    Anchor, Event, Key, LayerOptions, PixelSize, Platform, PopupOptions, Rect, Side, WindowOptions,
};
use palettes_wayland::{Wayland, logical_key};
use std::time::{Duration, Instant};

#[test]
fn keysyms_become_logical_keys() {
    assert_eq!(logical_key(0xff0d), Key::Enter);
    assert_eq!(logical_key(0xff1b), Key::Escape);
    assert_eq!(logical_key(0xffbe), Key::Function(1));
    assert_eq!(logical_key(0x0041), Key::Character('a'));
    assert_eq!(logical_key(0x0061), Key::Character('a'));
    assert_eq!(logical_key(0xffe1), Key::Unidentified); // Shift_L
}

fn wait(wayland: &mut Wayland, mut want: impl FnMut(&Event) -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        wayland.dispatch(Some(Duration::from_millis(50))).unwrap();
        while let Some(event) = wayland.next_event() {
            if want(&event) {
                return true;
            }
        }
    }
    false
}

#[test]
fn a_panel_and_its_popup_are_configured() {
    if std::env::var_os("WAYLAND_DISPLAY").is_none() {
        eprintln!("skipped: no Wayland session");
        return;
    }
    let mut wayland = Wayland::connect().unwrap();
    let outputs = wayland.outputs().unwrap();
    assert!(!outputs.is_empty());
    assert!(outputs.iter().all(|o| o.size.width > 0 && o.scale > 0.0));

    let layer = LayerOptions {
        anchor: Anchor::TOP | Anchor::LEFT | Anchor::RIGHT,
        output: Some(outputs[0].id),
        ..Default::default()
    };
    let panel = wayland
        .create_window(WindowOptions {
            app_id: "palettes-test",
            size: PixelSize {
                width: 100,
                height: 24,
            },
            layer: Some(layer),
            ..Default::default()
        })
        .unwrap();
    let mut info = None;
    assert!(wait(&mut wayland, |event| match event {
        Event::Configure { window, info: i } if *window == panel => {
            info = Some(*i);
            true
        }
        _ => false,
    }));
    let info = info.unwrap();
    assert_eq!(info.size.height, 24);
    assert_eq!(
        info.size.width, outputs[0].size.width,
        "stretched across the output"
    );
    assert!(wait(
        &mut wayland,
        |e| matches!(e, Event::Redraw { window } if *window == panel)
    ));

    let popup = wayland
        .create_window(WindowOptions {
            size: PixelSize {
                width: 80,
                height: 40,
            },
            popup: Some(PopupOptions {
                parent: panel,
                anchor: Rect::new(0.0, 0.0, 20.0, 24.0),
                side: Side::Below,
                grab: None,
            }),
            ..Default::default()
        })
        .unwrap();
    assert!(wait(
        &mut wayland,
        |e| matches!(e, Event::Configure { window, .. } if *window == popup)
    ));
    wayland.remove_window(popup).unwrap();
    wayland.remove_window(panel).unwrap();
    assert!(wayland.window_info(panel).is_err());
}
