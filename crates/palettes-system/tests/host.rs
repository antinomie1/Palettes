//! The backlight against a fake sysfs and logind, and `/proc` parsing.

mod common;

use common::*;
use palettes_system::{Brightness, Value, cpu_times, memory};

#[test]
fn the_firmware_backlight_is_read_and_set_through_logind() {
    let Some(daemon) = daemon("brightness") else {
        return;
    };
    let root = std::env::temp_dir().join(format!("palettes-backlight-{}", std::process::id()));
    for (name, kind, max, level) in [
        ("intel_backlight", "raw", "96000", "1000"),
        ("acpi_video0", "firmware", "100", "40"),
    ] {
        let dir = root.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("type"), format!("{kind}\n")).unwrap();
        std::fs::write(dir.join("max_brightness"), format!("{max}\n")).unwrap();
        std::fs::write(dir.join("actual_brightness"), format!("{level}\n")).unwrap();
    }
    let fake = Fake::serve(&daemon, &["org.freedesktop.login1"], |call| {
        (call.member == "SetBrightness").then(Vec::new)
    });
    let (changed, states) = reports();
    let brightness = Brightness::at(&root, &daemon.bus, changed).unwrap();
    let state = until(&states, |s| s.present);
    assert_eq!(state.level, 0.4, "firmware before raw");
    brightness.set(0.55);
    let call = fake.called("SetBrightness");
    assert_eq!(call.path, "/org/freedesktop/login1/session/auto");
    assert_eq!(
        call.body,
        [
            Value::str("backlight"),
            Value::str("acpi_video0"),
            Value::U32(55)
        ]
    );
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn processor_and_memory_use_parse_from_proc() {
    let stat = "cpu  100 5 50 800 20 3 2 0 0 0\ncpu0 50 2 25 400 10 1 1 0 0 0\n";
    assert_eq!(cpu_times(stat), Some((160, 980)));
    let meminfo = "MemTotal:       16000000 kB\nMemFree:  1000 kB\nMemAvailable:   12000000 kB\n";
    assert_eq!(memory(meminfo), Some((4_000_000 * 1024, 16_000_000 * 1024)));
    assert_eq!(memory("MemTotal: 1 kB\n"), None);
}
