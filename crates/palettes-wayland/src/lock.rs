//! The session lock (ext-session-lock-v1): one lock surface per output,
//! shown by the compositor instead of everything else until unlocked. If the
//! shell dies while locked, the session stays locked.

use crate::{state::State, window::Role};
use aegle_platform::{Error, Event, OutputId, PixelSize, Result, WindowId};
use wayland_client::{Connection, Dispatch, QueueHandle, delegate_noop};
use wayland_protocols::ext::session_lock::v1::client::{
    ext_session_lock_manager_v1::ExtSessionLockManagerV1,
    ext_session_lock_surface_v1::{self, ExtSessionLockSurfaceV1},
    ext_session_lock_v1::{self, ExtSessionLockV1},
};

/// The lock, while one is asked for or held.
#[derive(Default)]
pub(crate) struct Lock {
    pub manager: Option<ExtSessionLockManagerV1>,
    lock: Option<ExtSessionLockV1>,
    /// The compositor confirmed every output is hidden.
    locked: bool,
}

impl Lock {
    pub fn new(manager: Option<ExtSessionLockManagerV1>) -> Self {
        Self {
            manager,
            ..Self::default()
        }
    }
}

impl State {
    /// Asks the compositor to lock the session.
    pub fn lock_session(&mut self) -> Result<()> {
        let manager = (self.lock.manager.as_ref()).ok_or(Error::Unsupported(
            "locking: the compositor has no ext-session-lock-v1",
        ))?;
        if self.lock.lock.is_some() {
            return Ok(());
        }
        self.lock.lock = Some(manager.lock(&self.qh, ()));
        self.lock.locked = false;
        Ok(())
    }

    /// Whether the compositor confirmed the lock.
    pub fn is_locked(&self) -> bool {
        self.lock.locked
    }

    /// Covers `output` while locked; the compositor sizes the surface.
    pub fn create_lock_window(&mut self, output: OutputId) -> Result<WindowId> {
        let lock =
            (self.lock.lock.clone()).ok_or(Error::InvalidRole("the session is not locking"))?;
        let output = (self.output(output))
            .ok_or(Error::InvalidRole("the output is not connected"))?
            .output
            .clone();
        let id = WindowId(self.next_window);
        let surface = self.globals.compositor.create_surface(&self.qh, id);
        let lock_surface = lock.get_lock_surface(&surface, &output, &self.qh, id);
        self.add_window(id, surface, Role::Lock(lock_surface), PixelSize::default());
        Ok(id)
    }

    /// Unlocks the session; its lock surfaces are to be removed next.
    pub fn unlock_session(&mut self) {
        if let Some(lock) = self.lock.lock.take() {
            match self.lock.locked {
                true => lock.unlock_and_destroy(),
                // Not locked yet: this cancels the request.
                false => lock.destroy(),
            }
        }
        self.lock.locked = false;
    }
}

delegate_noop!(State: ExtSessionLockManagerV1);

impl Dispatch<ExtSessionLockV1, ()> for State {
    fn event(
        state: &mut Self,
        lock: &ExtSessionLockV1,
        event: ext_session_lock_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ext_session_lock_v1::Event::Locked => state.lock.locked = true,
            // Refused (another locker) or ended by the compositor: the lock
            // surfaces are gone.
            ext_session_lock_v1::Event::Finished => {
                lock.destroy();
                state.lock.lock = None;
                state.lock.locked = false;
                let ids: Vec<WindowId> = (state.windows.iter())
                    .filter(|w| matches!(w.role, Role::Lock(_)))
                    .map(|w| w.id)
                    .collect();
                for window in ids {
                    state.events.push_back(Event::Close { window });
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<ExtSessionLockSurfaceV1, WindowId> for State {
    fn event(
        state: &mut Self,
        surface: &ExtSessionLockSurfaceV1,
        event: ext_session_lock_surface_v1::Event,
        id: &WindowId,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let ext_session_lock_surface_v1::Event::Configure {
            serial,
            width,
            height,
        } = event
        {
            surface.ack_configure(serial);
            state.configured(*id, width, height);
        }
    }
}
