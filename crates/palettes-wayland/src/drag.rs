//! Drag and drop through `wl_data_device`: windows as drop targets for text
//! and files, and copy drags this client starts.
//!
//! The host answers each [`Event::Drag`] with `accept_drag`; the compositor
//! drops only onto a window that accepted. Dropped data is read like a
//! paste, then the offer is finished.

use crate::{
    clipboard::{Arrived, MIME, offered},
    state::State,
};
use aegle_platform::{DragData, Error, Event, Point, SeatId, WindowId};
use wayland_client::{
    Proxy,
    protocol::{
        wl_data_device_manager::DndAction, wl_data_offer::WlDataOffer, wl_surface::WlSurface,
    },
};

/// The file list type, preferred over text when both are offered.
const URI_LIST: &str = "text/uri-list";

/// A drag over one of this client's windows.
pub(crate) struct Over {
    pub offer: WlDataOffer,
    serial: u32,
    window: WindowId,
    position: Point,
    /// The type accepted, if the host took the drag.
    accepted: Option<&'static str>,
}

/// What a drop of `offer` would be read as: files first, then text.
fn choose(offer: &WlDataOffer) -> Option<&'static str> {
    offered(offer, [URI_LIST].into_iter().chain(MIME))
}

/// Decodes dropped bytes of type `mime`; a file list without local files is
/// nothing.
pub(crate) fn decode(mime: &str, bytes: Vec<u8>) -> Option<DragData> {
    let text = String::from_utf8(bytes).ok()?;
    if mime != URI_LIST {
        return Some(DragData::Text(text));
    }
    let files: Vec<_> = (text.lines())
        .filter(|line| !line.starts_with('#'))
        .filter_map(|line| aegle_types::path_from_file_uri(line.trim()))
        .collect();
    (!files.is_empty()).then_some(DragData::Files(files))
}

/// The types and bytes a drag of `data` offers.
fn encode(data: &DragData) -> (&'static [&'static str], Vec<u8>) {
    match data {
        DragData::Text(text) => (&MIME, text.as_bytes().to_vec()),
        DragData::Files(files) => {
            let list: String = (files.iter())
                .map(|file| aegle_types::file_uri(file) + "\r\n")
                .collect();
            (&[URI_LIST], list.into_bytes())
        }
    }
}

impl State {
    fn over_mut(&mut self, global: u32) -> Option<(SeatId, &mut Over)> {
        let device = self
            .clipboard
            .devices
            .iter_mut()
            .find(|d| d.global == global)?;
        Some((device.seat, device.over.as_mut()?))
    }

    fn push_drag(&mut self, global: u32) {
        if let Some((seat, over)) = self.over_mut(global) {
            let (window, position) = (over.window, over.position);
            self.events.push_back(Event::Drag {
                window,
                seat,
                position,
            });
        }
    }

    pub(crate) fn drag_enter(
        &mut self,
        global: u32,
        serial: u32,
        surface: &WlSurface,
        position: Point,
        offer: Option<WlDataOffer>,
    ) {
        let Some(device) = self
            .clipboard
            .devices
            .iter_mut()
            .find(|d| d.global == global)
        else {
            return;
        };
        if let Some(old) = device.over.take() {
            old.offer.destroy();
        }
        let window = surface.data::<WindowId>().copied();
        match (window, offer) {
            (Some(window), Some(offer)) if self.windows.iter().any(|w| w.id == window) => {
                device.over = Some(Over {
                    offer,
                    serial,
                    window,
                    position,
                    accepted: None,
                });
                self.push_drag(global);
            }
            (_, offer) => offer.iter().for_each(WlDataOffer::destroy),
        }
    }

    pub(crate) fn drag_motion(&mut self, global: u32, position: Point) {
        if let Some((_, over)) = self.over_mut(global) {
            over.position = position;
            self.push_drag(global);
        }
    }

    pub(crate) fn drag_leave(&mut self, global: u32) {
        let Some(device) = self
            .clipboard
            .devices
            .iter_mut()
            .find(|d| d.global == global)
        else {
            return;
        };
        if let Some(over) = device.over.take() {
            over.offer.destroy();
            self.events.push_back(Event::DragLeave {
                window: over.window,
                seat: device.seat,
            });
        }
    }

    pub(crate) fn drag_drop(&mut self, global: u32) {
        let Some(device) = self
            .clipboard
            .devices
            .iter_mut()
            .find(|d| d.global == global)
        else {
            return;
        };
        let Some(over) = device.over.take() else {
            return;
        };
        let (window, seat, position) = (over.window, device.seat, over.position);
        let Some(mime) = over.accepted else {
            over.offer.destroy();
            self.events.push_back(Event::DragLeave { window, seat });
            return;
        };
        let offer = over.offer.clone();
        let read = self.clipboard.read(&over.offer, mime, move |bytes| {
            Some(Arrived::Drop {
                window,
                seat,
                position,
                mime,
                offer,
                bytes,
            })
        });
        if read.is_err() {
            over.offer.destroy();
            self.events.push_back(Event::DragLeave { window, seat });
        }
    }

    /// Takes or refuses the drag of `seat` over `window`, as a copy of text
    /// or files.
    pub(crate) fn accept_drag(&mut self, window: WindowId, global: u32, accept: bool) {
        let Some((_, over)) = self.over_mut(global).filter(|(_, o)| o.window == window) else {
            return;
        };
        let mime = choose(&over.offer).filter(|_| accept);
        if over.accepted == mime {
            return;
        }
        over.accepted = mime;
        over.offer.accept(over.serial, mime.map(String::from));
        if over.offer.version() >= 3 {
            let action = if mime.is_some() {
                DndAction::Copy
            } else {
                DndAction::empty()
            };
            over.offer.set_actions(action, action);
        }
    }

    /// Starts a copy drag of `data` from `window`, authorized by `serial`,
    /// the seat's press that is still held.
    pub(crate) fn start_drag(
        &mut self,
        window: WindowId,
        global: u32,
        serial: u32,
        data: &DragData,
    ) -> Result<(), Error> {
        let surface = self
            .window(window)
            .ok_or(Error::InvalidWindow)?
            .surface
            .clone();
        let (mimes, bytes) = encode(data);
        let source = self.clipboard.source(mimes, &self.qh)?;
        if source.version() >= 3 {
            source.set_actions(DndAction::Copy);
        }
        let device = self.clipboard.device(global)?;
        device
            .device
            .start_drag(Some(&source), &surface, None, serial);
        if let Some((old, _)) = self.clipboard.dragging.replace((source, bytes.into())) {
            old.destroy();
        }
        Ok(())
    }
}
