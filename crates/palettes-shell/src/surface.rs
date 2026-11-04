//! [`Surface`]: a panel or popup and its root container.

use crate::{PanelOptions, PopupOptions, Shell, shell::Inner};
use aegle_platform::{PixelSize, Platform, WindowId, WindowOptions};
use aegle_ui::{Container, Node, Result, Ui, UiError};
use std::{
    ops::Deref,
    rc::{Rc, Weak},
};

/// A weak handle to a panel or popup; dereferences to its root [`Container`].
/// Dropping it keeps the surface open; [`Self::close`] closes it and its popups.
#[derive(Clone)]
pub struct Surface {
    shell: Weak<Inner>,
    id: WindowId,
    root: Container,
}

impl Surface {
    pub(crate) fn new(shell: &Shell, id: WindowId, ui: &Ui) -> Self {
        Self {
            shell: Rc::downgrade(&shell.0),
            id,
            root: ui.root(),
        }
    }

    fn inner(&self) -> Result<Rc<Inner>> {
        Ok(self.shell.upgrade().ok_or(UiError::DeadHandle)?)
    }

    /// Whether the surface is still open.
    pub fn is_open(&self) -> bool {
        self.shell
            .upgrade()
            .is_some_and(|inner| inner.runtime.borrow().entry(self.id).is_some())
    }

    /// Closes the surface and its popups; its controls' handles die with it.
    pub fn close(&self) -> Result<()> {
        self.inner()?.runtime.borrow_mut().close(self.id)
    }

    /// Runs `closer` once the surface closes, by [`Self::close`] or because
    /// the compositor dismissed it, as it does popups clicked away from.
    pub fn on_close(&self, closer: impl FnOnce() + 'static) -> Result<()> {
        let inner = self.inner()?;
        let mut runtime = inner.runtime.borrow_mut();
        runtime.entry(self.id).ok_or(UiError::DeadHandle)?;
        runtime.closers.push((self.id, Box::new(closer)));
        Ok(())
    }

    /// The surface's engine, for key handlers and whole-surface settings.
    pub fn ui(&self) -> Result<Rc<Ui>> {
        let inner = self.inner()?;
        let runtime = inner.runtime.borrow();
        Ok(runtime
            .entry(self.id)
            .ok_or(UiError::DeadHandle)?
            .ui
            .clone())
    }

    /// Physical pixels per logical pixel, possibly fractional.
    pub fn scale(&self) -> Result<f32> {
        Ok(self
            .inner()?
            .runtime
            .borrow()
            .platform
            .window_info(self.id)?
            .scale)
    }

    /// Moves a panel in place: new layer, edges, zone, keyboard policy, margin
    /// or size. The output cannot change.
    pub fn set_placement(&self, options: PanelOptions) -> Result<()> {
        let size = PixelSize {
            width: options.width,
            height: options.height,
        };
        Ok(self
            .inner()?
            .runtime
            .borrow_mut()
            .platform
            .set_layer(self.id, options.layer, size)?)
    }

    /// Takes pointer and touch input only on `node`, a control of this
    /// surface, within its rounded bounds; clicks elsewhere reach what is
    /// below. Several nodes, like separate pills, can each take input.
    /// With none, the surface takes input on its root's rounded bounds:
    /// everywhere but outside the corners of `bar.set_radius(…)`.
    pub fn set_input(&self, node: &Node, input: bool) -> Result<()> {
        self.with_shape(|shape| crate::shape::toggle(&mut shape.input, node, input))
    }

    /// Has the compositor blur the desktop behind `node`, a control of this
    /// surface, within its rounded bounds: frosted glass under a translucent
    /// background. `bar.set_blur(&bar, true)` blurs behind the whole surface;
    /// several nodes, like separate pills, can each be blurred. The blur
    /// follows the node's bounds, corner radius and visibility. Compositors
    /// without ext-background-effect-v1 show no blur.
    pub fn set_blur(&self, node: &Node, blur: bool) -> Result<()> {
        self.with_shape(|shape| crate::shape::toggle(&mut shape.blur, node, blur))
    }

    fn with_shape(&self, change: impl FnOnce(&mut crate::shape::Shape)) -> Result<()> {
        let inner = self.inner()?;
        let mut runtime = inner.runtime.borrow_mut();
        let entry = runtime
            .entries
            .iter_mut()
            .find(|e| e.id == self.id)
            .ok_or(UiError::DeadHandle)?;
        change(&mut entry.shape);
        Ok(())
    }

    /// Opens a popup attached to this surface.
    pub fn popup(&self, options: PopupOptions) -> Result<Surface> {
        let inner = self.inner()?;
        let mut runtime = inner.runtime.borrow_mut();
        // Grabs belong to the seat that last used this surface.
        let seat = runtime.entry(self.id).ok_or(UiError::DeadHandle)?.seat;
        let grab = options
            .grab
            .then(|| seat.unwrap_or(aegle_platform::SeatId(0)));
        let size = PixelSize {
            width: options.width,
            height: options.height,
        };
        let (id, ui) = runtime.open(
            WindowOptions {
                title: "",
                size,
                popup: Some(aegle_platform::PopupOptions {
                    parent: self.id,
                    anchor: options.anchor,
                    side: options.side,
                    grab,
                }),
                transparent: true,
                ..Default::default()
            },
            size,
        )?;
        drop(runtime);
        Ok(Surface {
            shell: Rc::downgrade(&inner),
            id,
            root: ui.root(),
        })
    }

    /// Opens a grabbing popup of `width` × `height` below `node`, a control
    /// of this surface, such as the button that opens a menu.
    pub fn popup_below(&self, node: &Node, width: u32, height: u32) -> Result<Surface> {
        self.popup(PopupOptions::below(node.bounds(), width, height))
    }
}

impl Deref for Surface {
    type Target = Container;
    fn deref(&self) -> &Container {
        &self.root
    }
}
