//! The clipboard: UTF-8 text through `wl_data_device`, one device per seat.
//!
//! Pipes are read and written on short-lived threads, so a slow or stuck
//! client never stalls the loop; a finished read comes back through a
//! channel and the loop's wake fd.

use crate::state::State;
use aegle_platform::{Error, Event, SeatId, WindowId};
use std::{
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
const MIME: [&str; 3] = ["text/plain;charset=utf-8", "UTF8_STRING", "text/plain"];
/// The most clipboard text read into memory.
const LIMIT: u64 = 4 << 20;

/// A finished read: the window and seat that asked, and the text.
type Arrived = (WindowId, SeatId, String);

/// The types an offer advertised.
type Types = Mutex<Vec<String>>;

struct Device {
    /// The seat's registry name.
    global: u32,
    device: WlDataDevice,
    /// What another client (or this one) holds as the selection.
    selection: Option<WlDataOffer>,
}

pub(crate) struct Clipboard {
    manager: Option<WlDataDeviceManager>,
    devices: Vec<Device>,
    /// The text this client offers while it holds the selection.
    own: Option<(WlDataSource, Arc<[u8]>)>,
    /// Texts read for a window, waiting for the loop.
    arrived: (Sender<Arrived>, Receiver<Arrived>),
    /// The loop's wake fd, set once the loop has one.
    pub wake: Option<Arc<OwnedFd>>,
}

impl Clipboard {
    pub fn new(manager: Option<WlDataDeviceManager>) -> Self {
        Self {
            manager,
            devices: Vec::new(),
            own: None,
            arrived: channel(),
            wake: None,
        }
    }

    pub fn add_seat(&mut self, seat: &WlSeat, global: u32, qh: &QueueHandle<State>) {
        if let Some(manager) = &self.manager {
            self.devices.push(Device {
                global,
                device: manager.get_data_device(seat, qh, global),
                selection: None,
            });
        }
    }

    pub fn remove_seat(&mut self, global: u32) {
        self.devices.retain(|d| {
            if d.global != global {
                return true;
            }
            if let Some(offer) = &d.selection {
                offer.destroy();
            }
            if d.device.version() >= 2 {
                d.device.release();
            }
            false
        });
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
        let manager = (self.manager.as_ref())
            .ok_or(Error::Unsupported("clipboard: no wl_data_device_manager"))?;
        let device = (self.devices.iter().find(|d| d.global == global))
            .ok_or_else(|| Error::backend("the seat has no data device"))?;
        let source = manager.create_data_source(qh, ());
        for mime in MIME {
            source.offer(mime.into());
        }
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
        events: &mut std::collections::VecDeque<Event>,
    ) -> Result<(), Error> {
        if let Some((_, text)) = &self.own {
            let text = String::from_utf8_lossy(text).into_owned();
            events.push_back(Event::Clipboard { window, seat, text });
            return Ok(());
        }
        let Some(offer) = (self.devices.iter())
            .find(|d| d.global == global)
            .and_then(|d| d.selection.as_ref())
        else {
            return Ok(());
        };
        let types = offer.data::<Types>().map(|t| t.lock().unwrap().clone());
        let Some(mime) = MIME
            .into_iter()
            .find(|m| types.iter().flatten().any(|t| t == m))
        else {
            return Ok(());
        };
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
                if read.is_err() || bytes.len() as u64 > LIMIT {
                    return;
                }
                if let Ok(text) = String::from_utf8(bytes) {
                    let _ = arrived.send((window, seat, text));
                    if let Some(wake) = wake {
                        let _ = rustix::io::write(&*wake, &1u64.to_ne_bytes());
                    }
                }
            })
            .map_err(Error::backend)?;
        Ok(())
    }

    /// Moves read clipboards into `events`, for windows still there.
    pub fn deliver(&self, events: &mut std::collections::VecDeque<Event>, live: impl Fn(WindowId) -> bool) {
        let read = self.arrived.1.try_iter().filter(|(window, ..)| live(*window));
        events.extend(read.map(|(window, seat, text)| Event::Clipboard { window, seat, text }));
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
            // Drops are not taken; let go of what is dragged over.
            wl_data_device::Event::Enter { id: Some(offer), .. } => offer.destroy(),
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
        let own = &mut state.clipboard.own;
        let ours = own.as_ref().is_some_and(|(s, _)| s == source);
        match event {
            wl_data_source::Event::Send { fd, .. } if ours => {
                let text = own.as_ref().map(|(_, t)| Arc::clone(t)).unwrap_or_default();
                let _ = std::thread::Builder::new()
                    .name("palettes-clipboard".into())
                    .spawn(move || drop(std::fs::File::from(fd).write_all(&text)));
            }
            wl_data_source::Event::Cancelled => {
                source.destroy();
                if ours {
                    *own = None;
                }
            }
            _ => {}
        }
    }
}

delegate_noop!(State: WlDataDeviceManager);
