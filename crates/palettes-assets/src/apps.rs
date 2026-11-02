//! Installed applications: freedesktop desktop entries.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

/// One `.desktop` file's `[Desktop Entry]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DesktopEntry {
    /// The desktop file id: its path under `applications/`, `/` replaced by
    /// `-`, without `.desktop`, such as `org.kde.dolphin`.
    pub id: String,
    /// Display name, localized for `LANG` when the entry has a translation.
    pub name: String,
    /// Icon name or path.
    pub icon: String,
    /// Command line, with field codes such as `%U` removed.
    pub exec: String,
    /// The X11/Wayland class some applications use instead of their id.
    pub wm_class: String,
    /// Hidden from launchers.
    pub no_display: bool,
}

/// Every desktop entry, earlier data directories winning, read once.
#[derive(Clone, Debug, Default)]
pub struct Applications {
    entries: Vec<DesktopEntry>,
    by_id: HashMap<String, usize>,
}

fn locale_names() -> Vec<String> {
    let lang = std::env::var("LC_ALL")
        .or_else(|_| std::env::var("LC_MESSAGES"))
        .or_else(|_| std::env::var("LANG"))
        .unwrap_or_default();
    let lang = lang.split(['.', '@']).next().unwrap_or("").to_owned();
    let short = lang.split('_').next().unwrap_or("").to_owned();
    [lang, short]
        .into_iter()
        .filter(|l| !l.is_empty() && l != "C")
        .collect()
}

fn parse(id: String, text: &str, locales: &[String]) -> Option<DesktopEntry> {
    let mut entry = DesktopEntry {
        id,
        name: String::new(),
        icon: String::new(),
        exec: String::new(),
        wm_class: String::new(),
        no_display: false,
    };
    let mut localized: Option<(usize, String)> = None;
    let mut application = false;
    for (key, value) in crate::ini_group(text, "Desktop Entry") {
        match key {
            "Name" => entry.name = value.to_owned(),
            "Icon" => entry.icon = value.to_owned(),
            "Exec" => {
                let words = value
                    .split_whitespace()
                    .filter(|w| !(w.len() == 2 && w.starts_with('%')));
                entry.exec = words.collect::<Vec<_>>().join(" ");
            }
            "StartupWMClass" => entry.wm_class = value.to_owned(),
            "NoDisplay" | "Hidden" => entry.no_display |= value == "true",
            "Type" => application = value == "Application",
            _ => {
                let Some(locale) = key.strip_prefix("Name[").and_then(|k| k.strip_suffix(']'))
                else {
                    continue;
                };
                if let Some(rank) = locales.iter().position(|l| l == locale)
                    && localized.as_ref().is_none_or(|(best, _)| rank < *best)
                {
                    localized = Some((rank, value.to_owned()));
                }
            }
        }
    }
    if let Some((_, name)) = localized {
        entry.name = name;
    }
    (application && !entry.name.is_empty()).then_some(entry)
}

fn scan(
    root: &Path,
    dir: &Path,
    locales: &[String],
    out: &mut Vec<DesktopEntry>,
    seen: &mut HashMap<String, usize>,
) {
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = read.flatten().map(|e| e.path()).collect();
    paths.sort();
    for path in paths {
        if path.is_dir() {
            scan(root, &path, locales, out, seen);
            continue;
        }
        if path.extension().is_none_or(|e| e != "desktop") {
            continue;
        }
        let Ok(relative) = path
            .with_extension("")
            .strip_prefix(root)
            .map(Path::to_owned)
        else {
            continue;
        };
        let id = relative.to_string_lossy().replace('/', "-");
        if seen.contains_key(&id) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        if let Some(entry) = parse(id.clone(), &text, locales) {
            seen.insert(id, out.len());
            out.push(entry);
        } else {
            // A hidden override still shadows entries further down the path.
            seen.insert(id, usize::MAX);
        }
    }
}

impl Applications {
    /// Reads the entries under every data directory's `applications/`.
    pub fn load() -> Self {
        Self::load_from(&crate::data_dirs())
    }

    /// Reads the entries under these data directories, earlier ones winning.
    pub fn load_from(data_dirs: &[PathBuf]) -> Self {
        let locales = locale_names();
        let (mut entries, mut seen) = (Vec::new(), HashMap::new());
        for dir in data_dirs {
            let root = dir.join("applications");
            scan(&root, &root, &locales, &mut entries, &mut seen);
        }
        let by_id = entries
            .iter()
            .enumerate()
            .map(|(i, e)| (e.id.clone(), i))
            .collect();
        Self { entries, by_id }
    }

    /// Every application, in no particular order.
    pub fn all(&self) -> &[DesktopEntry] {
        &self.entries
    }

    /// Applications a launcher lists, sorted by name.
    pub fn launchable(&self) -> Vec<&DesktopEntry> {
        let mut shown: Vec<_> = self.entries.iter().filter(|e| !e.no_display).collect();
        shown.sort_by_cached_key(|e| e.name.to_lowercase());
        shown
    }

    /// The entry of a window's app id. Tries the exact id, then ignoring
    /// case, then `StartupWMClass`, then an id ending in `.<app_id>` (so
    /// `dolphin` finds `org.kde.dolphin`).
    pub fn find(&self, app_id: &str) -> Option<&DesktopEntry> {
        if app_id.is_empty() {
            return None;
        }
        if let Some(&i) = self.by_id.get(app_id) {
            return Some(&self.entries[i]);
        }
        let lower = app_id.to_lowercase();
        let suffix = format!(".{lower}");
        let pick = |test: &dyn Fn(&DesktopEntry) -> bool| self.entries.iter().find(|e| test(e));
        pick(&|e| e.id.to_lowercase() == lower)
            .or_else(|| pick(&|e| e.wm_class.to_lowercase() == lower))
            .or_else(|| pick(&|e| e.id.to_lowercase().ends_with(&suffix)))
    }
}
