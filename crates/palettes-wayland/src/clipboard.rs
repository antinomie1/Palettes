//! The clipboard: UTF-8 text through `wl_data_device`, one device per seat.
//! Drag and drop shares the devices and sources; see `drag.rs`.
//!
//! Pipes are read and written on short-lived threads, so a slow or stuck
//! client never stalls the loop; a finished read comes back through a
//! channel and the loop's wake fd.

use crate::{drag::Over, state::State};
use aegle_platform::{Error, Event, Point, SeatId, WindowId};
use std::{
    collections::VecDeque,
    io::{Read, Write},
    os::fd::{AsFd, OwnedFd},
    sync::{
        Arc, Mutex,
        mpsc::{Receiver, Sender, channel},
    },
};
use wayland_client::{
    Connection, Dispatch, Proxy, QueueHandle, delegate_noop, event_created_child,
    protocol::{
        wl_data_device::{self, WlDataDevice},
        wl_data_device_manager::WlDataDeviceManager,
        wl_data_offer::{self, WlDataOffer},
        wl_data_source::{self, WlDataSource},
        wl_seat::WlSeat,
    },
};

/// Text types, in order of preference.
pub(crate) const MIME: [&str; 3] = ["text/plain;charset=utf-8", "UTF8_STRING", "text/plain"];
/// The most clipboard text read into memory.
const LIMIT: u64 = 4 << 20;

/// The types an offer advertised.
pub(crate) type Types = Mutex<Vec<String>>;

/// Data this client serves, and the source serving it.
type Source = (WlDataSource, Arc<[u8]>);

/// A finished pipe read, for the loop.
pub(crate) enum Arrived {
    Paste {
        window: WindowId,
        seat: SeatId,
        text: String,
    },
    Drop {
        window: WindowId,
        seat: SeatId,
        position: Point,
        mime: &'static str,
        offer: WlDataOffer,
        bytes: Option<Vec<u8>>,
    },
}

pub(crate) struct Device {
    /// The seat's registry name.
    pub global: u32,
    pub seat: SeatId,
    pub device: WlDataDevice,
    /// What another client (or this one) holds as the selection.
    selection: Option<WlDataOffer>,
    /// A drag over one of this client's windows.
    pub over: Option<Over>,
}

pub(crate) struct Clipboard {
    pub manager: Option<WlDataDeviceManager>,
    pub devices: Vec<Device>,
    /// The text this client offers while it holds the selection.
    own: Option<Source>,
    /// The drag this client started.
    pub dragging: Option<Source>,
    arrived: (Sender<Arrived>, Receiver<Arrived>),
    /// The loop's wake fd, set once the loop has one.
    pub wake: Option<Arc<OwnedFd>>,
}

/// The first of `wanted` that `offer` advertised.
pub(crate) fn offered(
    offer: &WlDataOffer,
    wanted: impl IntoIterator<Item = &'static str>,
) -> Option<&'static str> {
    let types = offer.data::<Types>()?.lock().unwrap();
    wanted.into_iter().find(|m| types.iter().any(|t| t == m))
}

/// Serves `data` into a pipe a client asked for.
fn serve(fd: OwnedFd, data: Arc<[u8]>) {
    let _ = std::thread::Builder::new()
        .name("palettes-clipboard".into())
        .spawn(move || drop(std::fs::File::from(fd).write_all(&data)));
}

impl Clipboard {
    pub fn new(manager: Option<WlDataDeviceManager>) -> Self {
        Self {
            manager,
            devices: Vec::new(),
            own: None,
            dragging: None,
            arrived: channel(),
            wake: None,
        }
    }

    pub fn add_seat(&mut self, seat: &WlSeat, id: SeatId, global: u32, qh: &QueueHandle<State>) {
        if let Some(manager) = &self.manager {
            self.devices.push(Device {
                global,
                seat: id,
                device: manager.get_data_device(seat, qh, global),
                selection: None,
                over: None,
            });
        }
    }

    pub fn remove_seat(&mut self, global: u32) {
        self.devices.retain(|d| {
            if d.global != global {
                return true;
            }
            for offer in d.selection.iter().chain(d.over.as_ref().map(|o| &o.offer)) {
                offer.destroy();
            }
            if d.device.version() >= 2 {
                d.device.release();
            }
            false
        });
    }

    pub fn device(&self, global: u32) -> Result<&Device, Error> {
        if self.manager.is_none() {
            return Err(Error::Unsupported("clipboard: no wl_data_device_manager"));
        }
        (self.devices.iter().find(|d| d.global == global))
            .ok_or_else(|| Error::backend("the seat has no data device"))
    }

    /// A source offering `mimes`, for a selection or a drag.
    pub fn source(&self, mimes: &[&str], qh: &QueueHandle<State>) -> Result<WlDataSource, Error> {
        let manager = (self.manager.as_ref())
            .ok_or(Error::Unsupported("clipboard: no wl_data_device_manager"))?;
        let source = manager.create_data_source(qh, ());
        for mime in mimes {
            source.offer((*mime).into());
        }
        Ok(source)
    }

    /// Offers `text` as the seat's selection, authorized by `serial`, the
    /// seat's latest input.
    pub fn set(
        &mut self,
        global: u32,
        serial: u32,
        text: &str,
        qh: &QueueHandle<State>,
    ) -> Result<(), Error> {
        let device = self.device(global)?;
        let source = self.source(&MIME, qh)?;
        device.device.set_selection(Some(&source), serial);
        if let Some((old, _)) = self.own.replace((source, text.as_bytes().into())) {
            old.destroy();
        }
        Ok(())
    }

    /// Reads the seat's selection for `window`; it arrives as an
    /// [`Event::Clipboard`], at once when this client holds it.
    pub fn request(
        &mut self,
        window: WindowId,
        seat: SeatId,
        global: u32,
        events: &mut VecDeque<Event>,
    ) -> Result<(), Error> {
        if let Some((_, text)) = &self.own {
            let text = String::from_utf8_lossy(text).into_owned();
            events.push_back(Event::Clipboard { window, seat, text });
            return Ok(());
        }
        let Some(offer) = self.device(global)?.selection.as_ref() else {
            return Ok(());
        };
        let Some(mime) = offered(offer, MIME) else {
            return Ok(());
        };
        self.read(offer, mime, move |bytes| {
            let text = bytes.and_then(|b| String::from_utf8(b).ok())?;
            Some(Arrived::Paste { window, seat, text })
        })
    }

    /// Receives `offer` as `mime` on a thread, then hands what `done` makes
    /// of the bytes (`None` after an error or past the limit) to the loop.
    pub fn read(
        &self,
        offer: &WlDataOffer,
        mime: &str,
        done: impl FnOnce(Option<Vec<u8>>) -> Option<Arrived> + Send + 'static,
    ) -> Result<(), Error> {
        let (read, write) =
            rustix::pipe::pipe_with(rustix::pipe::PipeFlags::CLOEXEC).map_err(Error::backend)?;
        // The request carries a duplicate of `write`; ours closes now, so
        // the read ends when the source closes its copy.
        offer.receive(mime.into(), write.as_fd());
        drop(write);
        let (arrived, wake) = (self.arrived.0.clone(), self.wake.clone());
        std::thread::Builder::new()
            .name("palettes-clipboard".into())
            .spawn(move || {
                let mut bytes = Vec::new();
                let read = std::fs::File::from(read)
                    .take(LIMIT + 1)
                    .read_to_end(&mut bytes);
                let bytes = (read.is_ok() && bytes.len() as u64 <= LIMIT).then_some(bytes);
                if let Some(value) = done(bytes) {
                    let _ = arrived.send(value);
                    if let Some(wake) = wake {
                        let _ = rustix::io::write(&*wake, &1u64.to_ne_bytes());
                    }
                }
            })
            .map_err(Error::backend)?;
        Ok(())
    }

    /// Moves finished reads into `events`, for windows still there.
    pub fn deliver(&self, events: &mut VecDeque<Event>, live: impl Fn(WindowId) -> bool) {
        for arrived in self.arrived.1.try_iter() {
            match arrived {
                Arrived::Paste { window, seat, text } if live(window) => {
                    events.push_back(Event::Clipboard { window, seat, text });
                }
                Arrived::Paste { .. } => {}
                Arrived::Drop {
                    window,
                    seat,
                    position,
                    mime,
                    offer,
                    bytes,
                } => {
                    let data = bytes.and_then(|b| crate::drag::decode(mime, b));
                    if offer.version() >= 3 && data.is_some() {
                        offer.finish();
                    }
                    offer.destroy();
                    if !live(window) {
                        continue;
                    }
                    events.push_back(match data {
                        Some(data) => Event::Drop {
                            window,
                            seat,
                            position,
                            data,
                        },
                        None => Event::DragLeave { window, seat },
                    });
                }
            }
        }
    }
}

impl Dispatch<WlDataDevice, u32> for State {
    fn event(
        state: &mut Self,
        _: &WlDataDevice,
        event: wl_data_device::Event,
        global: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(device) = (state.clipboard.devices.iter_mut()).find(|d| d.global == *global)
        else {
            return;
        };
        match event {
            wl_data_device::Event::Selection { id } => {
                if let Some(old) = std::mem::replace(&mut device.selection, id) {
                    old.destroy();
                }
            }
            wl_data_device::Event::Enter {
                serial,
                surface,
                x,
                y,
                id,
            } => state.drag_enter(
                *global,
                serial,
                &surface,
                Point::new(x as f32, y as f32),
                id,
            ),
            wl_data_device::Event::Motion { x, y, .. } => {
                state.drag_motion(*global, Point::new(x as f32, y as f32));
            }
            wl_data_device::Event::Leave => state.drag_leave(*global),
            wl_data_device::Event::Drop => state.drag_drop(*global),
            _ => {}
        }
    }

    event_created_child!(State, WlDataDevice, [
        wl_data_device::EVT_DATA_OFFER_OPCODE => (WlDataOffer, Types::default()),
    ]);
}

impl Dispatch<WlDataOffer, Types> for State {
    fn event(
        _: &mut Self,
        _: &WlDataOffer,
        event: wl_data_offer::Event,
        types: &Types,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_data_offer::Event::Offer { mime_type } = event {
            types.lock().unwrap().push(mime_type);
        }
    }
}

impl Dispatch<WlDataSource, ()> for State {
    fn event(
        state: &mut Self,
        source: &WlDataSource,
        event: wl_data_source::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let clipboard = &mut state.clipboard;
        let slot = [&mut clipboard.own, &mut clipboard.dragging]
            .into_iter()
            .find(|s| s.as_ref().is_some_and(|(s, _)| s == source));
        match event {
            wl_data_source::Event::Send { fd, .. } => {
                if let Some(Some((_, data))) = slot {
                    serve(fd, Arc::clone(data));
                }
            }
            // A replaced selection, or a drag that ended either way.
            wl_data_source::Event::Cancelled | wl_data_source::Event::DndFinished => {
                source.destroy();
                if let Some(slot) = slot {
                    *slot = None;
                }
            }
            _ => {}
        }
    }
}

delegate_noop!(State: WlDataDeviceManager);
