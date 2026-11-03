//! Panel documents: checking, the prelude, placement, and a live run where a
//! Wayland session exists.

use palettes::markup::{
    self, Documents,
    elements::{Edge, Placement},
};
use palettes::{KeyboardInteractivity, Layer, Shell};
use std::time::Duration;

fn document(name: &str, text: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("palettes-markup-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, text).unwrap();
    path
}

#[test]
fn the_example_bar_checks() {
    let bar = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/bar.aegle");
    markup::check(&bar).unwrap();
}

#[test]
fn checking_reports_mistakes_with_their_place() {
    let bad = document(
        "bad.aegle",
        "use \"palettes.aegle\"\nPanel {\n    edge: sideways\n}\n",
    );
    let message = markup::check(&bad).unwrap_err().to_string();
    assert!(message.contains("bad.aegle:3"), "{message}");
    let unknown = document(
        "unknown.aegle",
        "Panel { Button { on clicked { host.reboot() } } }",
    );
    assert!(
        markup::check(&unknown)
            .unwrap_err()
            .to_string()
            .contains("reboot")
    );
}

#[test]
fn placements_become_surface_options() {
    let bar = Placement::default().options();
    assert_eq!(
        (bar.height, bar.layer.exclusive_zone, bar.layer.layer),
        (32, 32, Layer::Top)
    );
    let dock = Placement {
        edge: Edge::Bottom,
        size: 64,
        span: 600,
        exclusive: Some(false),
        ..Placement::default()
    }
    .options();
    assert_eq!(
        (dock.width, dock.height, dock.layer.exclusive_zone),
        (600, 64, 0)
    );
    let launcher = Placement {
        edge: Edge::Center,
        size: 400,
        keyboard: KeyboardInteractivity::Exclusive,
        ..Placement::default()
    }
    .options();
    assert_eq!(
        (launcher.width, launcher.layer.layer),
        (480, Layer::Overlay)
    );
    assert_eq!(launcher.layer.keyboard, KeyboardInteractivity::Exclusive);
}

#[test]
fn a_document_opens_on_every_output_with_its_states() {
    if std::env::var_os("WAYLAND_DISPLAY").is_none() {
        eprintln!("skipped: no Wayland session");
        return;
    }
    let path = document(
        "live.aegle",
        "use \"palettes.aegle\"\nPanel {\n    size: 28dp\n    state output: string = \"\"\n    state time: string = \"\"\n    Text { text: output + \" \" + time }\n}\n",
    );
    let shell = Shell::new().unwrap();
    let outputs = shell.outputs().len();
    let documents = Documents::with(shell.clone(), markup::elements::elements())
        .unwrap()
        .open(&path)
        .unwrap();
    for _ in 0..10 {
        shell.dispatch(Some(Duration::from_millis(20))).unwrap();
    }
    assert!(outputs > 0);
    drop(documents);
}
