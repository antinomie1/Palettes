//! Roots, icon themes, desktop entries and the watcher, on fabricated directories.

use palettes_assets::{Applications, Icons, Roots, Watcher};
use std::{fs, path::PathBuf, sync::mpsc, time::Duration};

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("palettes-assets-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(path: PathBuf, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

#[test]
fn roots_resolve_in_order_and_find_fonts() {
    let dir = temp("roots");
    let (doc, config) = (dir.join("doc"), dir.join("config"));
    write(doc.join("bar.aegle"), "");
    write(config.join("img/logo.png"), "x");
    write(doc.join("img/logo.png"), "x");
    write(config.join("fonts/Inter.ttf"), "");
    write(config.join("fonts/readme.txt"), "");
    let roots = Roots::from_dirs(vec![doc.clone(), config.clone()]);
    assert_eq!(
        roots.resolve("img/logo.png"),
        Some(doc.join("img/logo.png"))
    );
    assert_eq!(roots.resolve("missing.png"), None);
    assert_eq!(roots.fonts(), [config.join("fonts/Inter.ttf")]);
    let standard = Roots::new(Some(&doc.join("bar.aegle")));
    assert_eq!(standard.dirs()[0], doc);
}

#[test]
fn icons_follow_theme_parents_sizes_and_desktop_entries() {
    let dir = temp("icons");
    let (icons, apps) = (dir.join("icons"), dir.join("share"));
    write(
        icons.join("Mine/index.theme"),
        "[Icon Theme]\nName=Mine\nInherits=hicolor\nDirectories=16x16/apps,48x48/apps\n\n\
         [16x16/apps]\nSize=16\nType=Fixed\n\n[48x48/apps]\nSize=48\nType=Fixed\n",
    );
    write(icons.join("Mine/16x16/apps/term.png"), "");
    write(icons.join("Mine/48x48/apps/term.png"), "");
    write(
        icons.join("hicolor/index.theme"),
        "[Icon Theme]\nName=Hicolor\nDirectories=scalable/apps\n\n[scalable/apps]\nSize=64\nType=Scalable\nMinSize=8\nMaxSize=512\n",
    );
    write(icons.join("hicolor/scalable/apps/editor.svg"), "");
    // Beside the icons base, as /usr/share/pixmaps is beside /usr/share/icons.
    write(dir.join("pixmaps/Shell.svg"), "");
    write(
        apps.join("applications/org.example.Editor.desktop"),
        "[Desktop Entry]\nType=Application\nName=Editor\nName[zh_CN]=编辑器\nIcon=editor\nExec=editor %F\nStartupWMClass=edit\n",
    );
    write(
        apps.join("applications/hidden.desktop"),
        "[Desktop Entry]\nType=Application\nName=Hidden\nNoDisplay=true\n",
    );

    let icons = Icons::with("Mine".into(), vec![icons]);
    assert!(
        icons
            .lookup("term", 48)
            .unwrap()
            .ends_with("48x48/apps/term.png")
    );
    assert!(
        icons
            .lookup("term", 20)
            .unwrap()
            .ends_with("16x16/apps/term.png")
    );
    assert_eq!(icons.lookup("editor", 32), None, "SVG only when asked");
    assert_eq!(icons.lookup("Shell", 32), None);
    let icons = icons.with_svg(true);
    assert_eq!(icons.lookup("Shell", 32), Some(dir.join("pixmaps/Shell.svg")));
    assert!(
        icons
            .lookup("editor", 32)
            .unwrap()
            .ends_with("hicolor/scalable/apps/editor.svg")
    );

    // SAFETY: no other test in this binary reads LANG.
    unsafe { std::env::set_var("LANG", "zh_CN.UTF-8") };
    let applications = Applications::load_from(&[apps]);
    let editor = applications.find("editor").unwrap();
    assert_eq!(
        (editor.name.as_str(), editor.exec.as_str()),
        ("编辑器", "editor")
    );
    assert_eq!(applications.find("edit").unwrap().id, "org.example.Editor");
    assert_eq!(
        applications.find("org.example.editor").unwrap().icon,
        "editor"
    );
    assert_eq!(applications.launchable().len(), 1);
}

#[test]
fn watcher_sees_saves_and_replacements() {
    let dir = temp("watch");
    let file = dir.join("bar.aegle");
    write(file.clone(), "a");
    let (send, changes) = mpsc::channel();
    let watcher = Watcher::spawn(move |path| drop(send.send(path))).unwrap();
    watcher.watch(&file).unwrap();
    fs::write(&file, "b").unwrap();
    let seen = changes.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(seen.file_name(), file.file_name());
    // An editor writing a temporary file and renaming it over the original.
    write(dir.join("other.txt"), "ignored");
    write(dir.join(".bar.aegle.swp"), "c");
    fs::rename(dir.join(".bar.aegle.swp"), &file).unwrap();
    let seen = changes.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(seen.file_name(), file.file_name());
}

#[test]
fn the_desktop_has_icons() {
    let icons = Icons::new().with_svg(true);
    // Informational on a real system: which theme, and whether a common icon resolves.
    eprintln!("theme {}: {:?}", icons.theme(), icons.lookup("folder", 32));
}
