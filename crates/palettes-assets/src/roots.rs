//! Asset roots and path resolution.

use std::path::{Path, PathBuf};

/// Directories relative asset paths resolve against, most specific first.
#[derive(Clone, Debug, Default)]
pub struct Roots {
    dirs: Vec<PathBuf>,
}

impl Roots {
    /// The standard roots, after `document`'s directory when given.
    pub fn new(document: Option<&Path>) -> Self {
        let own = document.and_then(Path::parent).map(|d| {
            if d.as_os_str().is_empty() {
                PathBuf::from(".")
            } else {
                d.to_owned()
            }
        });
        let config = crate::config_home().map(|c| c.join("palettes"));
        let data = crate::data_dirs().into_iter().map(|d| d.join("palettes"));
        let mut dirs: Vec<PathBuf> = Vec::new();
        for dir in own.into_iter().chain(config).chain(data) {
            if !dirs.contains(&dir) {
                dirs.push(dir);
            }
        }
        Self { dirs }
    }

    /// Exactly these roots.
    pub fn from_dirs(dirs: Vec<PathBuf>) -> Self {
        Self { dirs }
    }

    /// The roots in lookup order.
    pub fn dirs(&self) -> &[PathBuf] {
        &self.dirs
    }

    /// The file `name` names: `~/` expands to the home directory, absolute
    /// paths stay, relative ones are looked up in each root. `None` if no
    /// such file exists.
    pub fn resolve(&self, name: &str) -> Option<PathBuf> {
        if let Some(rest) = name.strip_prefix("~/") {
            let path = PathBuf::from(std::env::var_os("HOME")?).join(rest);
            return path.exists().then_some(path);
        }
        let path = Path::new(name);
        if path.is_absolute() {
            return path.exists().then(|| path.to_owned());
        }
        self.dirs.iter().map(|d| d.join(path)).find(|p| p.exists())
    }

    /// Font files (`.ttf`, `.otf`, `.ttc`, `.otc`) in every root's `fonts/`,
    /// sorted by root then name.
    pub fn fonts(&self) -> Vec<PathBuf> {
        let mut fonts = Vec::new();
        for dir in &self.dirs {
            let Ok(entries) = std::fs::read_dir(dir.join("fonts")) else {
                continue;
            };
            let mut found: Vec<PathBuf> = entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.extension().and_then(|e| e.to_str()).is_some_and(|e| {
                        matches!(
                            e.to_ascii_lowercase().as_str(),
                            "ttf" | "otf" | "ttc" | "otc"
                        )
                    })
                })
                .collect();
            found.sort();
            fonts.extend(found);
        }
        fonts
    }
}
