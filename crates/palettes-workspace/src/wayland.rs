//! Any compositor's own protocols: workspaces through ext-workspace-v1 and
//! windows through wlr-foreign-toplevel-management, each where it is offered.
//! Neither protocol says which workspace a window is on.

use crate::{
    Desktop, Output,
    toplevels::Toplevel,
    workspaces::{Group, Space},
};
use std::{
    io,
    sync::{Arc, Mutex},
};
use wayland_client::{
    Connection, Dispatch, EventQueue, Proxy, QueueHandle, delegate_noop,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{
        wl_output::{self, WlOutput},
        wl_registry::{self, WlRegistry},
        wl_seat::WlSeat,
    },
};
use wayland_protocols::ext::workspace::v1::client::{
    ext_workspace_handle_v1::ExtWorkspaceHandleV1, ext_workspace_manager_v1::ExtWorkspaceManagerV1,
};
use wayland_protocols_wlr::foreign_toplevel::v1::client::{
    zwlr_foreign_toplevel_handle_v1::ZwlrForeignToplevelHandleV1,
    zwlr_foreign_toplevel_manager_v1::ZwlrForeignToplevelManagerV1,
};

type Changed = Box<dyn FnMut(Desktop) + Send>;

/// The objects requests go through, shared with the caller's thread.
#[derive(Default)]
pub(crate) struct Handles {
    pub workspaces: Vec<(u64, ExtWorkspaceHandleV1)>,
    pub windows: Vec<(u64, ZwlrForeignToplevelHandleV1)>,
}

/// What the protocols have said so far.
pub(crate) struct Model {
    pub outputs: Vec<(WlOutput, String)>,
    pub groups: Vec<Group>,
    pub spaces: Vec<Space>,
    pub toplevels: Vec<Toplevel>,
    pub handles: Arc<Mutex<Handles>>,
    /// Something changed since the desktop was last reported.
    pub dirty: bool,
    changed: Option<Changed>,
}

/// An id stable for an object's life, unique among live objects.
pub(crate) fn key(proxy: &impl Proxy) -> u64 {
    u64::from(proxy.id().protocol_id())
}

impl Model {
    pub fn output_name(&self, output: &WlOutput) -> String {
        self.outputs
            .iter()
            .find(|(o, _)| o == output)
            .map(|(_, n)| n.clone())
            .unwrap_or_default()
    }

    pub fn handles(&self) -> std::sync::MutexGuard<'_, Handles> {
        self.handles.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The output with the keyboard: the only one, else the one showing the
    /// focused window. Neither protocol says so directly.
    fn focused_output(&self) -> Option<String> {
        if let [(_, name)] = self.outputs.as_slice() {
            return Some(name.clone());
        }
        let window = self.toplevels.iter().find(|t| t.activated)?;
        window.outputs.first().map(|o| self.output_name(o))
    }

    fn desktop(&self) -> Desktop {
        let focused = self.focused_output();
        let mut desktop = Desktop {
            outputs: self
                .outputs
                .iter()
                .map(|(_, name)| Output {
                    name: name.clone(),
                    focused: focused.as_ref() == Some(name),
                })
                .collect(),
            ..Desktop::default()
        };
        self.add_workspaces(&mut desktop, focused.as_deref());
        desktop.windows = self.toplevels.iter().map(Toplevel::window).collect();
        desktop
    }
}

pub struct Wayland {
    connection: Connection,
    workspaces: Option<ExtWorkspaceManagerV1>,
    seat: Option<WlSeat>,
    handles: Arc<Mutex<Handles>>,
    queue: Mutex<Option<(EventQueue<Model>, Model)>>,
}

impl Wayland {
    /// Connects to the Wayland display and binds whichever of the workspace
    /// and toplevel managers the compositor has; fails without both.
    pub fn connect() -> io::Result<Self> {
        let other = |e: &dyn std::fmt::Display| io::Error::other(e.to_string());
        let connection = Connection::connect_to_env().map_err(|e| other(&e))?;
        let (globals, queue) = registry_queue_init::<Model>(&connection).map_err(|e| other(&e))?;
        let qh = queue.handle();
        let workspaces: Option<ExtWorkspaceManagerV1> = globals.bind(&qh, 1..=1, ()).ok();
        let toplevels: Option<ZwlrForeignToplevelManagerV1> = globals.bind(&qh, 1..=3, ()).ok();
        if workspaces.is_none() && toplevels.is_none() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "the compositor has neither ext-workspace-v1 nor wlr-foreign-toplevel-management",
            ));
        }
        // Activating a window names the seat whose keyboard it gets.
        let seat = toplevels
            .is_some()
            .then(|| globals.bind::<WlSeat, _, _>(&qh, 1..=1, ()).ok())
            .flatten();
        let handles = Arc::default();
        let mut model = Model {
            outputs: Vec::new(),
            groups: Vec::new(),
            spaces: Vec::new(),
            toplevels: Vec::new(),
            handles: Arc::clone(&handles),
            dirty: false,
            changed: None,
        };
        let registry = globals.registry();
        for global in globals.contents().clone_list() {
            if global.interface == WlOutput::interface().name {
                let output: WlOutput = registry.bind(global.name, global.version.min(4), &qh, ());
                model.outputs.push((output, String::new()));
            }
        }
        Ok(Self {
            connection,
            workspaces,
            seat,
            handles,
            queue: Mutex::new(Some((queue, model))),
        })
    }

    /// Calls `changed` from a thread after each batch of updates; once, with
    /// an empty desktop, when the connection ends.
    pub fn subscribe(&self, changed: impl FnMut(Desktop) + Send + 'static) -> io::Result<()> {
        let (mut queue, mut model) = self
            .queue
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
            .ok_or_else(|| io::Error::other("already subscribed"))?;
        model.changed = Some(Box::new(changed));
        std::thread::Builder::new()
            .name("palettes-wayland-desktop".into())
            .spawn(move || {
                // A batch holds a whole update (every window's state up to its
                // `done`), so reporting after each one coalesces them.
                while queue.blocking_dispatch(&mut model).is_ok() {
                    if std::mem::take(&mut model.dirty) {
                        let desktop = model.desktop();
                        if let Some(changed) = model.changed.as_mut() {
                            changed(desktop);
                        }
                    }
                }
                if let Some(changed) = model.changed.as_mut() {
                    changed(Desktop::default());
                }
            })?;
        Ok(())
    }

    fn handles(&self) -> std::sync::MutexGuard<'_, Handles> {
        self.handles.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Asks to show a workspace.
    pub fn focus_workspace(&self, id: u64) -> bool {
        let handles = self.handles();
        let (Some(manager), Some((_, workspace))) = (
            &self.workspaces,
            handles.workspaces.iter().find(|(k, _)| *k == id),
        ) else {
            return false;
        };
        workspace.activate();
        manager.commit();
        let _ = self.connection.flush();
        true
    }

    /// Asks to focus a window, or to close it.
    pub fn window(&self, id: u64, close: bool) -> bool {
        let handles = self.handles();
        let Some((_, window)) = handles.windows.iter().find(|(k, _)| *k == id) else {
            return false;
        };
        match (close, &self.seat) {
            (true, _) => window.close(),
            (false, Some(seat)) => window.activate(seat),
            (false, None) => return false,
        }
        let _ = self.connection.flush();
        true
    }
}

impl Dispatch<WlRegistry, GlobalListContents> for Model {
    fn event(
        model: &mut Self,
        registry: &WlRegistry,
        event: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
            && interface == WlOutput::interface().name
        {
            let output: WlOutput = registry.bind(name, version.min(4), qh, ());
            model.outputs.push((output, String::new()));
        }
    }
}

impl Dispatch<WlOutput, ()> for Model {
    fn event(
        model: &mut Self,
        output: &WlOutput,
        event: wl_output::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_output::Event::Name { name } = event
            && let Some(entry) = model.outputs.iter_mut().find(|(o, _)| o == output)
        {
            entry.1 = name;
            model.dirty = true;
        }
    }
}

delegate_noop!(Model: ignore WlSeat);
