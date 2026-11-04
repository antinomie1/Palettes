//! [`Wayland`]: the connection, the blocking loop and the aegle platform contract.

use crate::{gpu::WindowSurface, state::State};
use aegle_platform::{
    Cursor, DragData, DrawResult, Error, Event, ImeRequest, LayerOptions, OutputInfo, PixelRect,
    PixelSize, Platform, Preferences, Region, Result, SeatId, SurfacePlatform, WakeHandle,
    WindowId, WindowInfo, WindowOptions,
};
use rustix::event::{EventfdFlags, PollFd, PollFlags, Timespec, eventfd, poll};
use std::{
    os::{
        fd::{AsFd, OwnedFd},
        unix::net::UnixStream,
    },
    sync::Arc,
    time::{Duration, Instant},
};
use wayland_client::{Connection, EventQueue, backend::WaylandError, globals::registry_queue_init};

/// A Wayland connection for shell surfaces. Use it from one thread.
pub struct Wayland {
    // Fields drop in order: the state's protocol objects before the queue and
    // connection they belong to.
    state: State,
    queue: EventQueue<State>,
    connection: Connection,
    wake: Arc<OwnedFd>,
    /// Appearance settings and the portal subscription that updates them.
    preferences: Preferences,
    portal: Option<(UnixStream, aegle_portal::Portal)>,
}

fn backend(error: impl std::fmt::Display) -> Error {
    Error::backend(error)
}

impl Wayland {
    /// Every connected output, as [`Platform::outputs`] without the `Result`.
    pub fn output_list(&self) -> Vec<OutputInfo> {
        self.state.outputs.iter().map(|o| o.info()).collect()
    }

    /// Limits pointer and touch input on a window to `rects` less `holes`,
    /// in logical surface coordinates; elsewhere events reach what is below.
    /// `None` takes input everywhere, the default. Applies with the next frame.
    pub fn set_input(
        &mut self,
        window: WindowId,
        rects: Option<&[PixelRect]>,
        holes: &[PixelRect],
    ) -> Result<()> {
        self.state.set_input(window, rects, holes)?;
        self.flush()
    }

    /// Has the compositor blur what lies behind `rects` less `holes` of a
    /// window, in logical surface coordinates (see
    /// [`rounded_rects`](crate::rounded_rects)); empty `rects` remove the
    /// blur. Applies with the window's next frame. A compositor without
    /// ext-background-effect-v1 ignores it.
    pub fn set_blur(
        &mut self,
        window: WindowId,
        rects: &[PixelRect],
        holes: &[PixelRect],
    ) -> Result<()> {
        self.state.set_blur(window, rects, holes)?;
        self.flush()
    }

    /// Resizes a popup to `size` logical pixels, placed again beside its
    /// anchor; a [`Event::Configure`](aegle_platform::Event) follows.
    /// Compositors without xdg-shell 3 cannot resize popups.
    pub fn resize_popup(&mut self, window: WindowId, size: PixelSize) -> Result<()> {
        self.state.resize_popup(window, size)?;
        self.flush()
    }

    /// Asks the compositor to lock the session; then cover every output
    /// with [`Self::create_lock_window`]. A compositor that refuses closes
    /// those windows ([`Event::Close`](aegle_platform::Event)).
    pub fn lock(&mut self) -> Result<()> {
        self.state.lock_session()?;
        self.flush()
    }

    /// Whether the compositor confirmed the lock: every output is hidden.
    pub fn is_locked(&self) -> bool {
        self.state.is_locked()
    }

    /// A lock surface covering `output`, while the session is locking or
    /// locked; the compositor gives its size.
    pub fn create_lock_window(&mut self, output: aegle_platform::OutputId) -> Result<WindowId> {
        let id = self.state.create_lock_window(output)?;
        self.flush()?;
        Ok(id)
    }

    /// Unlocks the session. Remove its lock windows next.
    pub fn unlock(&mut self) -> Result<()> {
        self.state.unlock_session();
        self.flush()
    }

    /// Sends queued requests. A full socket keeps the rest for the next
    /// flush; a closed connection is an error, so the loop ends.
    fn flush(&self) -> Result<()> {
        match self.connection.flush() {
            Ok(()) => Ok(()),
            Err(WaylandError::Io(e)) if e.kind() == std::io::ErrorKind::WouldBlock => Ok(()),
            Err(error) => Err(backend(error)),
        }
    }

    fn read_portal(&mut self) {
        let Some((_, portal)) = self.portal.as_mut() else {
            return;
        };
        let before = self.preferences;
        if !portal.read(&mut self.preferences) {
            self.portal = None;
        }
        if self.preferences != before {
            self.state
                .events
                .push_back(Event::Preferences(self.preferences));
        }
    }

    fn drain_wake(&self) {
        let mut buffer = [0u8; 8];
        let _ = rustix::io::read(&*self.wake, &mut buffer);
    }
}

impl Platform for Wayland {
    fn connect() -> Result<Self> {
        let connection = Connection::connect_to_env().map_err(backend)?;
        let (globals, mut queue) = registry_queue_init::<State>(&connection).map_err(backend)?;
        let mut state = State::new(&connection, &globals, queue.handle())?;
        // Two roundtrips: one for output and seat descriptions, one for what
        // binding them produced (xdg-output, capabilities, keymaps).
        for _ in 0..2 {
            queue.roundtrip(&mut state).map_err(backend)?;
        }
        state.events.retain(|e| !matches!(e, Event::Outputs));
        let wake =
            Arc::new(eventfd(0, EventfdFlags::CLOEXEC | EventfdFlags::NONBLOCK).map_err(backend)?);
        state.clipboard.wake = Some(Arc::clone(&wake));
        let mut preferences = Preferences::default();
        let portal = aegle_portal::connect(&mut preferences);
        Ok(Self {
            state,
            queue,
            connection,
            wake,
            preferences,
            portal,
        })
    }

    fn preferences(&self) -> Preferences {
        self.preferences
    }

    fn wake_handle(&mut self) -> Result<WakeHandle> {
        let fd = self.wake.clone();
        Ok(WakeHandle::new(move || {
            let _ = rustix::io::write(&*fd, &1u64.to_ne_bytes());
        }))
    }

    fn ime_available(&self) -> bool {
        self.state.ime.available()
    }

    fn create_window(&mut self, options: WindowOptions<'_>) -> Result<WindowId> {
        let id = self.state.create_window(options)?;
        self.flush()?;
        Ok(id)
    }

    fn set_layer(&mut self, window: WindowId, layer: LayerOptions, size: PixelSize) -> Result<()> {
        self.state.set_layer(window, layer, size)?;
        self.flush()
    }

    fn outputs(&self) -> Result<Vec<OutputInfo>> {
        Ok(self.output_list())
    }

    fn remove_window(&mut self, window: WindowId) -> Result<()> {
        self.state.remove_window(window)?;
        self.flush()
    }

    fn window_info(&self, window: WindowId) -> Result<WindowInfo> {
        Ok(self.state.window(window).ok_or(Error::InvalidWindow)?.info)
    }

    fn dispatch(&mut self, timeout: Option<Duration>) -> Result<()> {
        self.queue
            .dispatch_pending(&mut self.state)
            .map_err(backend)?;
        self.state.fire_repeats(Instant::now());
        self.state.deliver_clipboard();
        self.state.queue_redraws();
        if !self.state.events.is_empty() {
            return self.flush();
        }
        self.flush()?;
        let repeat = self
            .state
            .next_repeat()
            .map(|at| at.saturating_duration_since(Instant::now()));
        let timeout = match (timeout, repeat) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        if let Some(guard) = self.queue.prepare_read() {
            let (readable, woken, portal) = {
                let (connection, wake) = (guard.connection_fd(), self.wake.as_fd());
                // Without a portal the wake fd stands in, so the array keeps its shape.
                let portal = self
                    .portal
                    .as_ref()
                    .map_or(wake, |(stream, _)| stream.as_fd());
                let mut fds = [
                    PollFd::new(&connection, PollFlags::IN),
                    PollFd::new(&wake, PollFlags::IN),
                    PollFd::new(&portal, PollFlags::IN),
                ];
                let timeout = timeout.map(|t| {
                    Timespec::try_from(t).unwrap_or(Timespec {
                        tv_sec: i64::MAX,
                        tv_nsec: 0,
                    })
                });
                match poll(&mut fds, timeout.as_ref()) {
                    Ok(_) | Err(rustix::io::Errno::INTR) => {}
                    Err(error) => return Err(backend(error)),
                }
                let ready = |i: usize| !fds[i].revents().is_empty();
                (ready(0), ready(1), self.portal.is_some() && ready(2))
            };
            if readable {
                // Another reader may have taken the events; a closed
                // connection is an error, or the loop would spin on it.
                match guard.read() {
                    Ok(_) => {}
                    Err(WaylandError::Io(e)) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                    Err(error) => return Err(backend(error)),
                }
            } else {
                drop(guard);
            }
            if woken {
                self.drain_wake();
                self.state.events.push_back(Event::Wake);
            }
            if portal {
                self.read_portal();
            }
        }
        self.queue
            .dispatch_pending(&mut self.state)
            .map_err(backend)?;
        self.state.fire_repeats(Instant::now());
        self.state.deliver_clipboard();
        self.state.queue_redraws();
        self.flush()
    }

    fn next_event(&mut self) -> Option<Event> {
        self.state.events.pop_front()
    }

    fn request_redraw(&mut self, window: WindowId) -> Result<()> {
        self.state
            .window_mut(window)
            .ok_or(Error::InvalidWindow)?
            .redraw_requested = true;
        self.state.queue_redraws();
        Ok(())
    }

    fn set_title(&mut self, _: WindowId, _: &str) -> Result<()> {
        Err(Error::Unsupported("titles: shell surfaces have none"))
    }

    fn set_cursor(&mut self, window: WindowId, cursor: Cursor) -> Result<()> {
        self.state
            .window_mut(window)
            .ok_or(Error::InvalidWindow)?
            .cursor = cursor;
        self.state.show_cursor(window, cursor);
        self.flush()
    }

    fn configure_ime(&mut self, window: WindowId, request: Option<ImeRequest>) -> Result<()> {
        self.state.window(window).ok_or(Error::InvalidWindow)?;
        self.state.ime.configure(window, request)?;
        self.flush()
    }

    fn set_clipboard(&mut self, window: WindowId, seat: SeatId, text: &str) -> Result<()> {
        self.state.window(window).ok_or(Error::InvalidWindow)?;
        let seat = self.state.seat(seat)?;
        let (global, serial) = (seat.global, seat.last_serial);
        let state = &mut self.state;
        state.clipboard.set(global, serial, text, &state.qh)?;
        self.flush()
    }

    fn request_clipboard(&mut self, window: WindowId, seat: SeatId) -> Result<()> {
        self.state.window(window).ok_or(Error::InvalidWindow)?;
        let global = self.state.seat(seat)?.global;
        let state = &mut self.state;
        state
            .clipboard
            .request(window, seat, global, &mut state.events)?;
        self.flush()
    }

    fn accept_drag(&mut self, window: WindowId, seat: SeatId, accept: bool) -> Result<()> {
        let global = self.state.seat(seat)?.global;
        self.state.accept_drag(window, global, accept);
        self.flush()
    }

    fn start_drag(&mut self, window: WindowId, seat: SeatId, data: &DragData) -> Result<()> {
        let seat = self.state.seat(seat)?;
        let (global, serial) = (seat.global, seat.last_serial);
        self.state.start_drag(window, global, serial, data)?;
        self.flush()
    }

    fn present(
        &mut self,
        _: WindowId,
        _: Option<Region<PixelRect>>,
        _: impl FnOnce(&mut [u8], PixelSize, &Region<PixelRect>) -> DrawResult,
    ) -> Result<bool> {
        Err(Error::Unsupported(
            "software presentation; present through a GPU renderer",
        ))
    }
}

impl SurfacePlatform for Wayland {
    type Surface = WindowSurface;

    fn window_surface(&self, window: WindowId) -> Result<WindowSurface> {
        let window = self.state.window(window).ok_or(Error::InvalidWindow)?;
        Ok(WindowSurface::new(
            window.surface.clone(),
            self.connection.clone(),
        ))
    }

    fn present_external(
        &mut self,
        id: WindowId,
        present: impl FnOnce(PixelSize) -> DrawResult<bool>,
    ) -> Result<bool> {
        let qh = self.state.qh.clone();
        let window = self.state.window_mut(id).ok_or(Error::InvalidWindow)?;
        window.redraw_queued = false;
        if !window.info.configured || window.frame_pending {
            return Ok(false);
        }
        let size = window.info.buffer;
        if size.width == 0 || size.height == 0 {
            return Err(Error::InvalidSize);
        }
        window.apply_scale();
        // The presenter's commit carries this request; a failed one leaves it
        // pending for the next.
        if !window.frame_requested {
            window.surface.frame(&qh, id);
            window.frame_requested = true;
        }
        let result = present(size);
        let window = self.state.window_mut(id).ok_or(Error::InvalidWindow)?;
        match result {
            Ok(true) => {
                window.frame_requested = false;
                window.frame_pending = true;
                window.redraw_requested = false;
                self.flush()?;
                Ok(true)
            }
            Ok(false) => Ok(false),
            Err(error) => Err(Error::Draw(error)),
        }
    }
}
