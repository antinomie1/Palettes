//! Native handles for GPU presenters.
#![allow(unsafe_code)]

use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawDisplayHandle,
    RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle, WindowHandle,
};
use std::{marker::PhantomData, ptr::NonNull, rc::Rc};
use wayland_client::{Connection, Proxy, protocol::wl_surface::WlSurface};

/// A lease on a window's `wl_surface` and the connection, for a GPU presenter
/// such as aegle's Vulkan `WindowRenderer`. Drop the presenter before
/// removing the window: removal destroys the surface.
#[derive(Clone, Debug)]
pub struct WindowSurface {
    surface: WlSurface,
    connection: Connection,
    _thread: PhantomData<Rc<()>>,
}

impl WindowSurface {
    pub(crate) fn new(surface: WlSurface, connection: Connection) -> Self {
        Self {
            surface,
            connection,
            _thread: PhantomData,
        }
    }
}

impl HasWindowHandle for WindowSurface {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let ptr = NonNull::new(self.surface.id().as_ptr()).ok_or(HandleError::Unavailable)?;
        let raw = RawWindowHandle::Wayland(WaylandWindowHandle::new(ptr.cast()));
        // SAFETY: the surface lives until its window is removed, which the
        // host does only after dropping the presenter that borrows this.
        Ok(unsafe { WindowHandle::borrow_raw(raw) })
    }
}

impl HasDisplayHandle for WindowSurface {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        let ptr = NonNull::new(self.connection.backend().display_ptr())
            .ok_or(HandleError::Unavailable)?;
        let raw = RawDisplayHandle::Wayland(WaylandDisplayHandle::new(ptr.cast()));
        // SAFETY: the connection held here keeps the display alive.
        Ok(unsafe { DisplayHandle::borrow_raw(raw) })
    }
}
