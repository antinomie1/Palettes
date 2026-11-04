//! The cursor theme follows the environment, then KDE's, then GTK's
//! settings. One test in its own binary: it changes the environment.

use palettes_assets::cursor_theme;

#[test]
fn cursor_theme_follows_the_desktop_settings() {
    let config = std::env::temp_dir().join(format!("palettes-cursor-{}", std::process::id()));
    std::fs::create_dir_all(config.join("gtk-3.0")).unwrap();
    // SAFETY: the only test in this binary, before any thread reads them.
    unsafe {
        std::env::remove_var("XCURSOR_THEME");
        std::env::remove_var("XCURSOR_SIZE");
        std::env::set_var("XDG_CURRENT_DESKTOP", "none");
        std::env::set_var("XDG_CONFIG_HOME", &config);
    }
    assert_eq!(cursor_theme(), ("default".into(), 24));

    let gtk = "[Settings]\ngtk-cursor-theme-name=Adwaita\ngtk-cursor-theme-size=32\n";
    std::fs::write(config.join("gtk-3.0/settings.ini"), gtk).unwrap();
    assert_eq!(cursor_theme(), ("Adwaita".into(), 32));

    let kde = "[Mouse]\ncursorSize=48\ncursorTheme=breeze_cursors\n";
    std::fs::write(config.join("kcminputrc"), kde).unwrap();
    assert_eq!(cursor_theme(), ("breeze_cursors".into(), 48), "KDE's first");

    unsafe { std::env::set_var("XCURSOR_THEME", "Bibata") };
    assert_eq!(cursor_theme(), ("Bibata".into(), 48), "the environment first");
    let _ = std::fs::remove_dir_all(config);
}
