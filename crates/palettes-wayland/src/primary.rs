//! The primary selection (zwp-primary-selection-v1): the text last selected,
//! pasted with a middle click. One device per seat, read and served like the
//! clipboard.

use crate::{
    clipboard::{Arrived, MIME, Types, serve},
    state::State,
};
use aegle_platform::{Error, Event, SeatId, WindowId};
use std::sync::Arc;
use wayland_client::{
    Connection, Dispatch, Proxy, QueueHandle, delegate_noop, event_created_child,
    protocol::wl_seat::WlSeat,
};
use wayland_protocols::wp::primary_selection::zv1::client::{
    zwp_primary_selection_device_manager_v1::ZwpPrimarySelectionDeviceManagerV1,
    zwp_primary_selection_device_v1::{self, ZwpPrimarySelectionDeviceV1},
    zwp_primary_selection_offer_v1::{self, ZwpPrimarySelectionOfferV1},
    zwp_primary_selection_source_v1::{self, ZwpPrimarySelectionSourceV1},
};

struct Device {
    global: u32,
    device: ZwpPrimarySelectionDeviceV1,
    offer: Option<ZwpPrimarySelectionOfferV1>,
}

pub(crate) struct Primary {
    manager: Option<ZwpPrimarySelectionDeviceManagerV1>,
    devices: Vec<Device>,
    /// What this client offers while it holds the selection.
    own: Option<(ZwpPrimarySelectionSourceV1, Arc<[u8]>)>,
}

impl Primary {
    pub fn new(manager: Option<ZwpPrimarySelectionDeviceManagerV1>) -> Self {
        Self {
            manager,
            devices: Vec::new(),
            own: None,
        }
    }

    pub fn add_seat(&mut self, seat: &WlSeat, global: u32, qh: &QueueHandle<State>) {
        if let Some(manager) = &self.manager {
            let device = manager.get_device(seat, qh, global);
            self.devices.push(Device {
                global,
                device,
                offer: None,
            });
        }
    }

    pub fn remove_seat(&mut self, global: u32) {
        self.devices.retain(|d| {
            if d.global != global {
                return true;
            }
            if let Some(offer) = &d.offer {
                offer.destroy();
            }
            d.device.destroy();
            false
        });
    }

    fn device(&self, global: u32) -> Option<&Device> {
        self.devices.iter().find(|d| d.global == global)
    }
}

impl State {
    /// Offers `text` as the primary selection of the seat `global`,
    /// authorized by `serial`; without the protocol, nothing.
    pub fn set_primary(&mut self, global: u32, serial: u32, text: &str) {
        let (Some(manager), Some(device)) = (&self.primary.manager, self.primary.device(global))
        else {
            return;
        };
        let source = manager.create_source(&self.qh, ());
        for mime in MIME {
            source.offer(mime.into());
        }
        device.device.set_selection(Some(&source), serial);
        if let Some((old, _)) = self.primary.own.replace((source, text.as_bytes().into())) {
            old.destroy();
        }
    }

    /// Reads the seat's primary selection for `window`; it arrives as
    /// [`Event::Primary`], at once when this client holds it.
    pub fn request_primary(
        &mut self,
        window: WindowId,
        seat: SeatId,
        global: u32,
    ) -> Result<(), Error> {
        if let Some((_, text)) = &self.primary.own {
            let text = String::from_utf8_lossy(text).into_owned();
            self.events.push_back(Event::Primary { window, seat, text });
            return Ok(());
        }
        let Some(offer) = self.primary.device(global).and_then(|d| d.offer.clone()) else {
            return Ok(());
        };
        let mime = {
            let types = offer.data::<Types>().map(|t| t.lock().unwrap().clone());
            let types = types.unwrap_or_default();
            MIME.into_iter().find(|m| types.iter().any(|t| t == m))
        };
        let Some(mime) = mime else {
            return Ok(());
        };
        self.clipboard.read_pipe(
            |write| offer.receive(mime.into(), write),
            move |bytes| {
                let text = bytes.and_then(|b| String::from_utf8(b).ok())?;
                let primary = true;
                Some(Arrived::Paste {
                    window,
                    seat,
                    text,
                    primary,
                })
            },
        )
    }
}

delegate_noop!(State: ZwpPrimarySelectionDeviceManagerV1);

impl Dispatch<ZwpPrimarySelectionDeviceV1, u32> for State {
    fn event(
        state: &mut Self,
        _: &ZwpPrimarySelectionDeviceV1,
        event: zwp_primary_selection_device_v1::Event,
        global: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwp_primary_selection_device_v1::Event::Selection { id } = event
            && let Some(device) = (state.primary.devices.iter_mut()).find(|d| d.global == *global)
            && let Some(old) = std::mem::replace(&mut device.offer, id)
        {
            old.destroy();
        }
    }

    event_created_child!(State, ZwpPrimarySelectionDeviceV1, [
        zwp_primary_selection_device_v1::EVT_DATA_OFFER_OPCODE => (ZwpPrimarySelectionOfferV1, Types::default()),
    ]);
}

impl Dispatch<ZwpPrimarySelectionOfferV1, Types> for State {
    fn event(
        _: &mut Self,
        _: &ZwpPrimarySelectionOfferV1,
        event: zwp_primary_selection_offer_v1::Event,
        types: &Types,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwp_primary_selection_offer_v1::Event::Offer { mime_type } = event {
            types.lock().unwrap().push(mime_type);
        }
    }
}

impl Dispatch<ZwpPrimarySelectionSourceV1, ()> for State {
    fn event(
        state: &mut Self,
        source: &ZwpPrimarySelectionSourceV1,
        event: zwp_primary_selection_source_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let own = &mut state.primary.own;
        let ours = own.as_ref().filter(|(s, _)| s == source);
        match event {
            zwp_primary_selection_source_v1::Event::Send { fd, .. } => {
                if let Some((_, data)) = ours {
                    serve(fd, Arc::clone(data));
                }
            }
            zwp_primary_selection_source_v1::Event::Cancelled => {
                source.destroy();
                if ours.is_some() {
                    *own = None;
                }
            }
            _ => {}
        }
    }
}
