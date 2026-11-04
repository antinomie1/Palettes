//! wlr-foreign-toplevel-management: windows, their titles and state, and
//! requests to focus or close them.

use crate::{
    Window,
    wayland::{Model, key},
};
use wayland_client::{
    Connection, Dispatch, QueueHandle, event_created_child, protocol::wl_output::WlOutput,
};
use wayland_protocols_wlr::foreign_toplevel::v1::client::{
    zwlr_foreign_toplevel_handle_v1::{self as handle, State, ZwlrForeignToplevelHandleV1},
    zwlr_foreign_toplevel_manager_v1::{self as manager, ZwlrForeignToplevelManagerV1},
};

pub(crate) struct Toplevel {
    proxy: ZwlrForeignToplevelHandleV1,
    title: String,
    app_id: String,
    pub outputs: Vec<WlOutput>,
    pub activated: bool,
    fullscreen: bool,
}

impl Toplevel {
    pub fn window(&self) -> Window {
        Window {
            id: key(&self.proxy),
            title: self.title.clone(),
            app_id: self.app_id.clone(),
            icon: String::new(),
            // The protocol has no workspaces.
            workspace: 0,
            focused: self.activated,
            floating: false,
            fullscreen: self.fullscreen,
        }
    }
}

/// The states in a `state` event: an array of native-endian u32.
fn states(bytes: &[u8]) -> impl Iterator<Item = u32> + '_ {
    bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|c| u32::from_ne_bytes(*c))
}

impl Dispatch<ZwlrForeignToplevelManagerV1, ()> for Model {
    fn event(
        model: &mut Self,
        _: &ZwlrForeignToplevelManagerV1,
        event: manager::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let manager::Event::Toplevel { toplevel } = event {
            model
                .handles()
                .windows
                .push((key(&toplevel), toplevel.clone()));
            model.toplevels.push(Toplevel {
                proxy: toplevel,
                title: String::new(),
                app_id: String::new(),
                outputs: Vec::new(),
                activated: false,
                fullscreen: false,
            });
        }
    }

    event_created_child!(Model, ZwlrForeignToplevelManagerV1, [
        manager::EVT_TOPLEVEL_OPCODE => (ZwlrForeignToplevelHandleV1, ()),
    ]);
}

impl Dispatch<ZwlrForeignToplevelHandleV1, ()> for Model {
    fn event(
        model: &mut Self,
        proxy: &ZwlrForeignToplevelHandleV1,
        event: handle::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(at) = model.toplevels.iter().position(|t| &t.proxy == proxy) else {
            return;
        };
        let toplevel = &mut model.toplevels[at];
        match event {
            handle::Event::Title { title } => toplevel.title = title,
            handle::Event::AppId { app_id } => toplevel.app_id = app_id,
            handle::Event::OutputEnter { output } => toplevel.outputs.push(output),
            handle::Event::OutputLeave { output } => toplevel.outputs.retain(|o| o != &output),
            handle::Event::State { state } => {
                let has = |s: State| states(&state).any(|x| x == s as u32);
                toplevel.activated = has(State::Activated);
                toplevel.fullscreen = has(State::Fullscreen);
            }
            handle::Event::Done => model.dirty = true,
            handle::Event::Closed => {
                model.toplevels.remove(at);
                let id = key(proxy);
                model.handles().windows.retain(|(k, _)| *k != id);
                proxy.destroy();
                model.dirty = true;
            }
            _ => {}
        }
    }
}
