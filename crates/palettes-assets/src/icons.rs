//! Freedesktop icon themes: name to file, for one size.

use crate::Applications;
use std::{
    cell::RefCell,
    collections::HashMap,
    path::{Path, PathBuf},
};

#[derive(Debug)]
struct Dir {
    path: String,
    size: u32,
    min: u32,
    max: u32,
    scalable: bool,
}

#[derive(Debug)]
struct Theme {
    /// Every base directory holding this theme.
    bases: Vec<PathBuf>,
    dirs: Vec<Dir>,
    parents: Vec<String>,
}

/// Icon lookup by name or application id, cached per name and size.
///
/// The theme is `PALETTES_ICON_THEME`, else the desktop's (KDE's
/// `kdeglobals`, then GTK's `settings.ini`), else `hicolor`; parents named by
/// `Inherits` follow, and `hicolor` and `/usr/share/pixmaps` come last.
pub struct Icons {
    theme: String,
    bases: Vec<PathBuf>,
    svg: bool,
    themes: RefCell<HashMap<String, Option<Theme>>>,
    found: RefCell<HashMap<(String, u32), Option<PathBuf>>>,
    apps: RefCell<Option<Applications>>,
}

fn desktop_theme() -> Option<String> {
    if let Ok(theme) = std::env::var("PALETTES_ICON_THEME") {
        return Some(theme);
    }
    let config = crate::config_home()?;
    let read = |path: PathBuf, group: &str, key: &str| {
        let text = std::fs::read_to_string(path).ok()?;
        crate::ini_group(&text, group)
            .find(|(k, _)| *k == key)
            .map(|(_, v)| v.to_owned())
    };
    read(config.join("kdeglobals"), "Icons", "Theme")
        .or_else(|| {
            read(
                config.join("gtk-4.0/settings.ini"),
                "Settings",
                "gtk-icon-theme-name",
            )
        })
        .or_else(|| {
            read(
                config.join("gtk-3.0/settings.ini"),
                "Settings",
                "gtk-icon-theme-name",
            )
        })
}

impl Default for Icons {
    fn default() -> Self {
        Self::new()
    }
}

impl Icons {
    /// The user's theme, PNG icons only.
    pub fn new() -> Self {
        let home = std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".icons"));
        let bases = home
            .into_iter()
            .chain(crate::data_dirs().into_iter().map(|d| d.join("icons")))
            .collect();
        Self::with(desktop_theme().unwrap_or_else(|| "hicolor".into()), bases)
    }

    /// Theme `theme`, looked up under these icon base directories.
    pub fn with(theme: String, bases: Vec<PathBuf>) -> Self {
        Self {
            theme,
            bases,
            svg: false,
            themes: RefCell::default(),
            found: RefCell::default(),
            apps: RefCell::default(),
        }
    }

    /// Also accept SVG icons; many themes have nothing else. Use it when
    /// the renderer can decode SVG.
    pub fn with_svg(mut self, svg: bool) -> Self {
        self.svg = svg;
        self.found.get_mut().clear();
        self
    }

    /// The theme looked up first.
    pub fn theme(&self) -> &str {
        &self.theme
    }

    fn load(&self, name: &str) -> Option<Theme> {
        let bases: Vec<PathBuf> = self
            .bases
            .iter()
            .map(|b| b.join(name))
            .filter(|d| d.is_dir())
            .collect();
        let index = bases
            .iter()
            .find_map(|b| std::fs::read_to_string(b.join("index.theme")).ok())?;
        let mut dirs = Vec::new();
        let mut parents = Vec::new();
        let mut listed = Vec::new();
        for (key, value) in crate::ini_group(&index, "Icon Theme") {
            match key {
                "Directories" | "ScaledDirectories" => {
                    listed.extend(value.split(',').map(|d| d.trim().to_owned()))
                }
                "Inherits" => parents.extend(value.split(',').map(|p| p.trim().to_owned())),
                _ => {}
            }
        }
        for path in listed.into_iter().filter(|d| !d.is_empty()) {
            let fields: HashMap<&str, &str> = crate::ini_group(&index, &path).collect();
            if fields.get("Scale").is_some_and(|s| *s != "1") {
                continue;
            }
            let number = |key: &str| fields.get(key).and_then(|v| v.parse::<u32>().ok());
            let Some(size) = number("Size") else { continue };
            let kind = fields.get("Type").copied().unwrap_or("Threshold");
            let threshold = number("Threshold").unwrap_or(2);
            let (min, max) = match kind {
                "Fixed" => (size, size),
                "Scalable" => (
                    number("MinSize").unwrap_or(size),
                    number("MaxSize").unwrap_or(size),
                ),
                _ => (size.saturating_sub(threshold), size + threshold),
            };
            dirs.push(Dir {
                scalable: kind == "Scalable",
                path,
                size,
                min,
                max,
            });
        }
        Some(Theme {
            bases,
            dirs,
            parents,
        })
    }

    /// The file for `name` in theme `theme`: an exact size first, then the
    /// nearest size.
    fn in_theme(&self, theme: &Theme, name: &str, size: u32) -> Option<PathBuf> {
        let extensions: &[&str] = if self.svg { &["png", "svg"] } else { &["png"] };
        let file = |dir: &Dir| {
            theme.bases.iter().find_map(|base| {
                extensions
                    .iter()
                    .filter(|e| dir.scalable || **e == "png")
                    .map(|e| base.join(&dir.path).join(format!("{name}.{e}")))
                    .find(|p| p.is_file())
            })
        };
        let distance = |dir: &Dir| {
            if size < dir.min {
                dir.min - size
            } else {
                size.saturating_sub(dir.max)
            }
        };
        let mut order: Vec<&Dir> = theme.dirs.iter().collect();
        // Exact matches, then the closest, preferring bigger (downscaling looks better).
        order.sort_by_key(|d| (distance(d), d.size < size, d.size.abs_diff(size)));
        order.into_iter().find_map(file)
    }

    fn search(&self, name: &str, size: u32) -> Option<PathBuf> {
        let mut queue = vec![self.theme.clone()];
        let mut visited = Vec::new();
        while let Some(theme) = queue.first().cloned() {
            queue.remove(0);
            if visited.contains(&theme) {
                continue;
            }
            visited.push(theme.clone());
            let mut themes = self.themes.borrow_mut();
            let loaded = themes
                .entry(theme.clone())
                .or_insert_with(|| self.load(&theme));
            if let Some(loaded) = loaded {
                if let Some(path) = self.in_theme(loaded, name, size) {
                    return Some(path);
                }
                queue.extend(loaded.parents.iter().cloned());
            }
            if queue.is_empty() && !visited.iter().any(|t| t == "hicolor") {
                queue.push("hicolor".into());
            }
        }
        let pixmaps = crate::data_dirs().into_iter().map(|d| d.join("pixmaps"));
        pixmaps
            .map(|d| d.join(format!("{name}.png")))
            .find(|p| p.is_file())
    }

    /// The icon file for a name such as `firefox` or `audio-volume-high` at
    /// `size` logical pixels. An absolute path names itself.
    pub fn lookup(&self, name: &str, size: u32) -> Option<PathBuf> {
        if name.is_empty() {
            return None;
        }
        if Path::new(name).is_absolute() {
            return Path::new(name).is_file().then(|| name.into());
        }
        let key = (name.to_owned(), size);
        if let Some(found) = self.found.borrow().get(&key) {
            return found.clone();
        }
        let found = self.search(name, size);
        self.found.borrow_mut().insert(key, found.clone());
        found
    }

    /// The icon of the application whose windows have `app_id`, through its
    /// desktop entry, else an icon named like the id.
    pub fn for_app(&self, app_id: &str, size: u32) -> Option<PathBuf> {
        let icon = {
            let mut apps = self.apps.borrow_mut();
            let apps = apps.get_or_insert_with(Applications::load);
            apps.find(app_id).map(|e| e.icon.clone())
        };
        icon.and_then(|icon| self.lookup(&icon, size))
            .or_else(|| self.lookup(app_id, size))
            .or_else(|| self.lookup(&app_id.to_lowercase(), size))
    }

    /// Forgets cached lookups, after themes or applications were installed.
    pub fn clear(&self) {
        self.themes.borrow_mut().clear();
        self.found.borrow_mut().clear();
        self.apps.borrow_mut().take();
    }
}
