//! The Wayland backend against the session's compositor, read only. Skips
//! without a display or without ext-workspace-v1 and
//! wlr-foreign-toplevel-management.

use palettes_workspace::{Compositor, Source};
use std::{sync::mpsc, time::Duration};

#[test]
fn reports_the_desktop() {
    if std::env::var_os("WAYLAND_DISPLAY").is_none() {
        return;
    }
    let Ok(compositor) = Compositor::connect_wayland() else {
        return;
    };
    assert_eq!(compositor.source(), Source::Wayland);
    let (send, desktops) = mpsc::channel();
    compositor
        .subscribe(move |desktop| drop(send.send(desktop)))
        .unwrap();
    let desktop = desktops.recv_timeout(Duration::from_secs(2)).unwrap();
    // The protocols say nothing about which workspace a window is on.
    assert!(desktop.windows.iter().all(|w| w.workspace == 0));
    assert!(desktop.windows.iter().filter(|w| w.focused).count() <= 1);
    assert!(compositor.subscribe(|_| {}).is_err(), "one subscription");
}
