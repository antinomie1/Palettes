//! Finding things a shell shows, without the user spelling out paths.
//!
//! - [`Roots`]: where a document's relative paths resolve, in order: its own
//!   directory, `$XDG_CONFIG_HOME/palettes`, `$XDG_DATA_HOME/palettes`, then
//!   each `$XDG_DATA_DIRS/palettes`. Fonts in any root's `fonts/` are found
//!   by [`Roots::fonts`].
//! - [`Icons`]: freedesktop icon theme lookup by name, following the user's
//!   theme and its parents down to `hicolor`, and by application id through
//!   [`Applications`], the installed desktop entries.
//! - [`Watcher`]: an inotify thread reporting changed files, for hot reload.
//!
//! Lookups are cached; nothing here polls.

mod apps;
mod icons;
mod roots;
mod watch;

pub use apps::{Applications, DesktopEntry};
pub use icons::Icons;
pub use roots::Roots;
pub use watch::Watcher;

use std::path::PathBuf;

/// `$XDG_DATA_HOME` then `$XDG_DATA_DIRS`, the bases of icons and applications.
pub(crate) fn data_dirs() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| home.as_ref().map(|h| h.join(".local/share")));
    let dirs = std::env::var("XDG_DATA_DIRS")
        .ok()
        .filter(|d| !d.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    data_home
        .into_iter()
        .chain(dirs.split(':').filter(|d| !d.is_empty()).map(PathBuf::from))
        .collect()
}

/// `$XDG_CONFIG_HOME`, or `~/.config`.
pub(crate) fn config_home() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
}

/// Reads `key=value` lines of one `[group]` of an ini-style file.
pub(crate) fn ini_group<'a>(
    text: &'a str,
    group: &str,
) -> impl Iterator<Item = (&'a str, &'a str)> {
    let header = format!("[{group}]");
    let mut inside = false;
    text.lines().filter_map(move |line| {
        let line = line.trim();
        if line.starts_with('[') {
            inside = line == header;
            return None;
        }
        if !inside || line.starts_with('#') {
            return None;
        }
        let (key, value) = line.split_once('=')?;
        Some((key.trim(), value.trim()))
    })
}
