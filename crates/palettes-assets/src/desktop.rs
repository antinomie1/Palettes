//! The desktop's own settings: KDE's and GTK's config files, read once
//! when asked, for themes the user picked there.

use std::path::PathBuf;

/// The first value found for `(file, group, key)` under
/// `$XDG_CONFIG_HOME`, in order.
pub(crate) fn setting(sources: &[(&str, &str, &str)]) -> Option<String> {
    let config = crate::config_home()?;
    sources.iter().find_map(|(file, group, key)| {
        let text = std::fs::read_to_string(config.join(PathBuf::from(file))).ok()?;
        crate::ini_group(&text, group)
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.to_owned())
            .filter(|v| !v.is_empty())
    })
}

/// The default theme of desktops whose default is implicit: KDE's, then
/// GNOME's, by `XDG_CURRENT_DESKTOP`.
pub(crate) fn default_for_desktop(kde: &str, gnome: &str) -> Option<String> {
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    match () {
        _ if desktop.contains("KDE") => Some(kde.into()),
        _ if desktop.contains("GNOME") => Some(gnome.into()),
        _ => None,
    }
}

/// The cursor theme and its size in logical pixels: `XCURSOR_THEME` and
/// `XCURSOR_SIZE`, else KDE's `kcminputrc`, else GTK's `settings.ini`,
/// else the desktop's default; `default` at 24 px without any.
pub fn cursor_theme() -> (String, u32) {
    let env = |name| std::env::var(name).ok().filter(|v: &String| !v.is_empty());
    let name = env("XCURSOR_THEME")
        .or_else(|| {
            setting(&[
                ("kcminputrc", "Mouse", "cursorTheme"),
                ("gtk-4.0/settings.ini", "Settings", "gtk-cursor-theme-name"),
                ("gtk-3.0/settings.ini", "Settings", "gtk-cursor-theme-name"),
            ])
        })
        .or_else(|| default_for_desktop("breeze_cursors", "Adwaita"))
        .unwrap_or_else(|| "default".into());
    let size = env("XCURSOR_SIZE")
        .or_else(|| {
            setting(&[
                ("kcminputrc", "Mouse", "cursorSize"),
                ("gtk-4.0/settings.ini", "Settings", "gtk-cursor-theme-size"),
                ("gtk-3.0/settings.ini", "Settings", "gtk-cursor-theme-size"),
            ])
        })
        .and_then(|s| s.parse().ok())
        .filter(|s| *s > 0)
        .unwrap_or(24);
    (name, size)
}
