//! Lock documents: a `Panel { edge: lock }` locks the session and covers
//! every output until `host.unlock(password)` passes PAM; then the document
//! closes and the session unlocks. A failure fills `lock_error`.

use super::{
    Runner,
    actions::action,
    elements::Edge,
    states::{set, string},
    with_runner,
};
use aegle_loader::{Data, markup::Type};
use aegle_ui::Result;
use palettes_shell::{OutputInfo, Shell, Surface};
use std::{cell::Cell, rc::Rc};

/// A lock surface on `output`, locking the session first.
pub(super) fn surface(shell: &Shell, output: &OutputInfo) -> Result<Surface> {
    shell.lock()?;
    shell.lock_surface(output.id)
}

/// Gives the keyboard to the first text field, so typing goes to the
/// password at once.
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

/// Registers `host.unlock`; without a shell it does nothing, for checking.
pub(super) fn register(shell: Option<&Shell>) {
    let Some(shell) = shell else {
        return action("unlock", &[Type::String], |_| Ok(()));
    };
    let busy = Rc::new(Cell::new(false));
    let idle = busy.clone();
    let done = shell.channel(move |_, result: std::result::Result<(), String>| {
        idle.set(false);
        if let Err(e) = with_runner(|runner| finished(runner, result)) {
            eprintln!("palettes: unlocking: {e}");
        }
    });
    action("unlock", &[Type::String], move |a| {
        let password = match a.first() {
            Some(Data::String(text)) => text.to_string(),
            _ => String::new(),
        };
        // One check at a time: PAM delays each failure.
        if busy.replace(true) {
            return Ok(());
        }
        let done = done.clone();
        std::thread::spawn(move || {
            let user = palettes_system::user_name().unwrap_or_default();
            let checked =
                palettes_system::authenticate(palettes_system::pam_service(), &user, &password);
            done.send(checked);
        });
        Ok(())
    });
}

/// Closes the lock documents, unlocking, or tells them why not.
fn finished(runner: &mut Runner, result: std::result::Result<(), String>) -> Result<()> {
    let is_lock = |d: &super::Document| d.placement.edge == Edge::Lock;
    match result {
        Ok(()) => {
            let (locks, kept) = std::mem::take(&mut runner.documents)
                .into_iter()
                .partition::<Vec<_>, _>(is_lock);
            runner.documents = kept;
            for document in locks {
                document.close(&runner.shell);
            }
        }
        Err(e) => {
            for document in runner.documents.iter().filter(|d| is_lock(d)) {
                for open in &document.panels {
                    set(&open.view.borrow(), "lock_error", string(&e));
                }
            }
        }
    }
    Ok(())
}
