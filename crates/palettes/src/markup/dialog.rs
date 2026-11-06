//! Dialogs: a `center` panel with `keyboard: exclusive`, such as a launcher,
//! gives the keyboard to its first text field when it opens, and Escape
//! closes its document. Lock documents focus their field the same way.

use aegle_ui::{Key, Result};
use palettes_shell::{Sender, Shell, Surface};
use std::{cell::RefCell, path::PathBuf};

thread_local! {
    /// Closes documents once the key that asked has been handled: a surface
    /// can't close while it delivers a key.
    static CLOSER: RefCell<Option<Sender<PathBuf>>> = const { RefCell::new(None) };
}

fn closer(shell: &Shell) -> Sender<PathBuf> {
    CLOSER.with_borrow_mut(|closer| {
        closer
            .get_or_insert_with(|| {
                shell.channel(|_, path: PathBuf| {
                    if let Err(e) = super::close_path(&path) {
                        eprintln!("palettes: closing {}: {e}", path.display());
                    }
                })
            })
            .clone()
    })
}

/// Gives the keyboard to the first text field, so typing goes there at once.
pub(super) fn focus_field(surface: &Surface) -> Result<()> {
    surface.ui()?.with_state(|state| {
        let mut pending = vec![state.root()];
        while let Some(id) = pending.pop() {
            if state.element(id).control().editor().is_some() {
                return state.set_focus(Some(id));
            }
            let children: Vec<_> = state.children(id).collect();
            pending.extend(children.into_iter().rev());
        }
        Ok(())
    })
}

/// Focuses the dialog's field and closes the document at `path` on Escape.
pub(super) fn attach(shell: &Shell, surface: &Surface, path: PathBuf) -> Result<()> {
    focus_field(surface)?;
    let closer = closer(shell);
    surface.ui()?.on_key(move |key| {
        let escape = key.key == Key::Escape && key.pressed;
        if escape {
            closer.send(path.clone());
        }
        escape
    });
    Ok(())
}
