//! A live shell inside the current Wayland session; skipped without one.

use aegle_widgets::Widgets;
use palettes_shell::{PanelOptions, Shell};
use std::{
    cell::Cell,
    rc::Rc,
    time::{Duration, Instant},
};

fn until(shell: &Shell, mut done: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        shell.dispatch(Some(Duration::from_millis(20))).unwrap();
        if done() {
            return true;
        }
    }
    false
}

#[test]
fn panels_lay_out_and_channels_deliver() {
    if std::env::var_os("WAYLAND_DISPLAY").is_none() {
        eprintln!("skipped: no Wayland session");
        return;
    }
    let shell = Shell::new().unwrap();
    let output = shell.outputs()[0].clone();
    let bar = shell.panel(PanelOptions::top(30).on(output.id)).unwrap();
    let label = bar.text("waiting");
    assert!(until(&shell, || bar.bounds().size.width as u32
        == output.size.width));
    assert_eq!(bar.bounds().size.height, 30.0);

    let got = Rc::new(Cell::new(0));
    let seen = got.clone();
    let sender = shell.channel(move |_, n: u32| {
        seen.set(n);
        label.set_text(&format!("got {n}"));
    });
    std::thread::spawn(move || sender.send(7));
    assert!(until(&shell, || got.get() == 7));

    let popup = bar.popup_below(&bar, 100, 50).unwrap();
    assert!(until(&shell, || popup.bounds().size.width == 100.0));
    // A fitted popup takes its content's size, and follows it.
    let content = popup.column();
    content.set_width(123.0);
    content.set_height(45.0);
    popup.set_padding(0.0);
    popup.fit(&content).unwrap();
    let size = |w: f32, h: f32| popup.bounds().size == aegle_ui::Size::new(w, h);
    assert!(until(&shell, || size(123.0, 45.0)), "fitted");
    content.set_height(60.0);
    assert!(until(&shell, || size(123.0, 60.0)), "grown");
    bar.close().unwrap();
    assert!(!popup.is_open(), "popups close with their parent");
    assert!(
        !shell.dispatch(Some(Duration::ZERO)).unwrap(),
        "no surfaces left"
    );
}
