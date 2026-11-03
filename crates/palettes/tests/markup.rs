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
fn panels_take_visual_effects() {
    let glass = document(
        "glass.aegle",
        "use \"palettes.aegle\"\nPanel {\n    offset: 8dp\n    radius: 12dp\n    background: #1e1e2ecc\n    opacity: 0.9\n    blur: true\n    Text { text: \"glass\" }\n}\n",
    );
    markup::check(&glass).unwrap();
    let bad = document("bad-blur.aegle", "Panel { blur: 3 }");
    assert!(markup::check(&bad).is_err());
}

#[test]
fn panels_take_shapes_animations_and_image_backgrounds() {
    let fx = document(
        "fx.aegle",
        r#"use "palettes.aegle"
Panel {
    state level: float = 0.5
    radius: 12dp
    Image { source: "wallpaper.png"; fit: cover; inset: 0dp; radius: 12dp }
    Row {
        Stack {
            Arc { width: 24dp; height: 24dp; stroke: #45475a; stroke_width: 3dp }
            Arc { width: 24dp; height: 24dp; sweep: level * 360.0; stroke: #89b4fa
                  stroke_width: 3dp; cap: round }
        }
        Shape { width: 16dp; height: 16dp; path: "M 2 8 L 6 12 L 14 4"; stroke: #ffffff }
        Text { text: "busy"; rotation_animation: [[0, 360], 1000ms, linear, forever]
               scale_transition: spring(300, 10) }
    }
}
"#,
    );
    markup::check(&fx).unwrap();
    let bad = document(
        "bad-fx.aegle",
        "Panel { Image { source: \"a.png\"; fit: tile } }",
    );
    assert!(markup::check(&bad).is_err());
}

#[test]
fn shaders_check_and_the_example_program_compiles() {
    let examples = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    markup::check(&examples.join("aurora.aegle")).unwrap();
    let fonts = std::rc::Rc::new(std::cell::RefCell::new(aegle_ui::TextSystem::new()));
    let ui = aegle_ui::Ui::with_fonts(fonts, aegle_ui::Theme::default()).unwrap();
    let view = aegle_widgets::Widgets::shader_view(&ui.root());
    view.load(examples.join("shaders/aurora.wgsl")).unwrap();
    assert!(view.shader().is_some_and(|s| s.is_animated()));
    let bad = document(
        "bad-shader.aegle",
        "Panel { Shader { source: \"a.wgsl\"; values: [1, 2, 3, 4, 5, 6, 7, 8, 9] } }",
    );
    assert!(markup::check(&bad).is_err());
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
