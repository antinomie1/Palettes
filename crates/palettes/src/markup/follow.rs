//! Views showing a file, so a change to the file shows the new one.

use aegle_ui::Node;
use std::{
    cell::RefCell,
    ops::Deref,
    path::{Path, PathBuf},
};

/// Views of one kind and the canonical paths of the files they show.
pub(crate) struct Followers<V>(RefCell<Vec<(V, PathBuf)>>);

impl<V: Clone + Deref<Target = Node>> Followers<V> {
    pub const fn new() -> Self {
        Self(RefCell::new(Vec::new()))
    }

    /// Follows `path` for `view` instead of what it showed, watching the
    /// file; returns the path as followed.
    pub fn follow(&self, view: &V, path: &Path) -> PathBuf {
        let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_owned());
        super::watch(&path);
        let mut shown = self.0.borrow_mut();
        shown.retain(|(v, _)| v.is_alive() && v.id() != view.id());
        shown.push((view.clone(), path.clone()));
        path
    }

    /// Stops following a file for `view`.
    pub fn forget(&self, view: &V) {
        (self.0.borrow_mut()).retain(|(v, _)| v.is_alive() && v.id() != view.id());
    }

    /// The file `view` shows.
    pub fn path_of(&self, view: &V) -> Option<PathBuf> {
        let shown = self.0.borrow();
        (shown.iter().find(|(v, _)| v.id() == view.id())).map(|(_, p)| p.clone())
    }

    /// The live views showing `path`.
    pub fn of(&self, path: &Path) -> Vec<V> {
        let mut shown = self.0.borrow_mut();
        shown.retain(|(v, _)| v.is_alive());
        (shown.iter().filter(|(_, p)| p == path))
            .map(|(v, _)| v.clone())
            .collect()
    }
}
