//! Popup documents: a `Panel { edge: popup }` opens beside the control under
//! the pointer, on the surface it is over, and closes with a click elsewhere.

use super::{Document, Open, Runner, attach, elements::Placement};
use aegle_ui::{Color, Rect, Result};
use std::{cell::RefCell, path::PathBuf, rc::Rc};

/// The surface under the pointer and where on it the popup attaches: the
/// hovered control, or the pointer itself over the panel's background.
fn anchor(runner: &Runner) -> Option<(&Open, &Placement, Rect)> {
    runner.documents.iter().find_map(|document| {
        document.panels.iter().find_map(|open| {
            let ui = open.surface.ui().ok()?;
            let (point, hovered) = ui
                .with_state(|state| Ok((state.pointer_position(), state.hover_target())))
                .ok()?;
            let point = point?;
            let root = open.view.borrow().root().node().id();
            let rect = match hovered.filter(|id| *id != root) {
                Some(id) => open.surface.handle(id).bounds(),
                None => Rect::new(point.x, point.y, 1.0, 1.0),
            };
            Some((open, &document.placement, rect))
        })
    })
}

/// Opens `document`, a popup, beside the control under the pointer.
pub(super) fn open(runner: &Runner, document: &mut Document) -> Result<()> {
    let (parent, placement, rect) =
        anchor(runner).ok_or("a popup opens under the pointer, which is on no panel")?;
    let surface = parent
        .surface
        .popup(document.placement.popup_options(rect, placement))?;
    surface.set_background(Color::TRANSPARENT);
    surface.set_padding(0.0);
    let view = match document.program.build(&surface) {
        Ok(view) => Rc::new(RefCell::new(view)),
        Err(e) => {
            surface.close()?;
            return Err(e);
        }
    };
    let open = Open {
        output: parent.output.clone(),
        surface,
        view,
    };
    attach(&runner.shell, &open, &document.states)?;
    // Dismissed by a click elsewhere, or closed with its parent: the
    // document closes, and its script stops.
    let path = document.path.clone();
    open.surface.on_close(move || dismissed(path))?;
    document.panels.push(open);
    Ok(())
}

fn dismissed(path: PathBuf) {
    let closed = super::with_runner(|runner| {
        let index = runner.documents.iter().position(|d| {
            d.path == path && d.is_popup() && !d.panels.iter().any(|p| p.surface.is_open())
        });
        Ok(index.map(|index| runner.documents.remove(index)))
    });
    if let Ok(Some(document)) = closed {
        document.close();
    }
}
