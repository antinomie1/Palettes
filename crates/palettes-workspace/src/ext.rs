//! Any compositor with ext-workspace-v1: workspaces and their outputs, and
//! switching. The protocol knows nothing of windows, so they stay empty.

use crate::{Desktop, Output, Workspace};
use std::{
    io,
    sync::{Arc, Mutex},
};
use wayland_client::{
    Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum, event_created_child,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{
        wl_output::{self, WlOutput},
        wl_registry::{self, WlRegistry},
    },
};
use wayland_protocols::ext::workspace::v1::client::{
    ext_workspace_group_handle_v1::{self as group, ExtWorkspaceGroupHandleV1},
    ext_workspace_handle_v1::{self as handle, ExtWorkspaceHandleV1},
    ext_workspace_manager_v1::{self as manager, ExtWorkspaceManagerV1},
};

type Changed = Box<dyn FnMut(Desktop) + Send>;

struct Group {
    proxy: ExtWorkspaceGroupHandleV1,
    outputs: Vec<WlOutput>,
}

struct Space {
    proxy: ExtWorkspaceHandleV1,
    group: Option<ExtWorkspaceGroupHandleV1>,
    name: String,
    coordinates: Vec<i32>,
    state: u32,
}

/// What the protocol has said so far.
struct Model {
    outputs: Vec<(WlOutput, String)>,
    groups: Vec<Group>,
    spaces: Vec<Space>,
    /// The handles requests go through, shared with the caller's thread.
    handles: Arc<Mutex<Vec<(u64, ExtWorkspaceHandleV1)>>>,
    changed: Option<Changed>,
}

fn key(proxy: &impl Proxy) -> u64 {
    u64::from(proxy.id().protocol_id())
}

impl Model {
    fn output_name(&self, output: &WlOutput) -> String {
        self.outputs
            .iter()
            .find(|(o, _)| o == output)
            .map(|(_, n)| n.clone())
            .unwrap_or_default()
    }

    fn desktop(&self) -> Desktop {
        let single = self.groups.len() == 1;
        let mut desktop = Desktop {
            outputs: self
                .outputs
                .iter()
                .map(|(_, name)| Output {
                    name: name.clone(),
                    focused: self.outputs.len() == 1,
                })
                .collect(),
            ..Desktop::default()
        };
        // Grouped workspaces by group, then any without a group.
        let groups = self.groups.iter().map(|g| Some(&g.proxy)).chain([None]);
        for group in groups {
            let output = group
                .and_then(|g| self.groups.iter().find(|x| &x.proxy == g))
                .and_then(|g| g.outputs.first())
                .map(|o| self.output_name(o))
                .unwrap_or_default();
            let mut members: Vec<&Space> = self
                .spaces
                .iter()
                .filter(|s| {
                    s.group.as_ref() == group && s.state & handle::State::Hidden.bits() == 0
                })
                .collect();
            members.sort_by_key(|s| s.coordinates.clone());
            for (index, space) in members.into_iter().enumerate() {
                let active = space.state & handle::State::Active.bits() != 0;
                desktop.workspaces.push(Workspace {
                    id: key(&space.proxy),
                    name: space.name.clone(),
                    index: index as u32 + 1,
                    output: output.clone(),
                    active,
                    // Which output has the keyboard is not part of the protocol.
                    focused: active && single,
                    windows: 0,
                });
            }
        }
        desktop
    }
}

pub struct Ext {
    connection: Connection,
    manager: ExtWorkspaceManagerV1,
    handles: Arc<Mutex<Vec<(u64, ExtWorkspaceHandleV1)>>>,
    queue: Mutex<Option<(EventQueue<Model>, Model)>>,
}

impl Ext {
    /// Connects to the Wayland display and binds the workspace manager.
    pub fn connect() -> io::Result<Self> {
        let other = |e: &dyn std::fmt::Display| io::Error::other(e.to_string());
        let connection = Connection::connect_to_env().map_err(|e| other(&e))?;
        let (globals, queue) = registry_queue_init::<Model>(&connection).map_err(|e| other(&e))?;
        let qh = queue.handle();
        let manager: ExtWorkspaceManagerV1 = globals.bind(&qh, 1..=1, ()).map_err(|_| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "the compositor has no ext-workspace-v1",
            )
        })?;
        let handles = Arc::default();
        let mut model = Model {
            outputs: Vec::new(),
            groups: Vec::new(),
            spaces: Vec::new(),
            handles: Arc::clone(&handles),
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
            manager,
            handles,
            queue: Mutex::new(Some((queue, model))),
        })
    }

    /// Calls `changed` from a thread after each update; once, with an empty
    /// desktop, when the connection ends.
    pub fn subscribe(&self, changed: impl FnMut(Desktop) + Send + 'static) -> io::Result<()> {
        let (mut queue, mut model) = self
            .queue
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
            .ok_or_else(|| io::Error::other("already subscribed"))?;
        model.changed = Some(Box::new(changed));
        std::thread::Builder::new()
            .name("palettes-ext-workspace".into())
            .spawn(move || {
                while queue.blocking_dispatch(&mut model).is_ok() {}
                if let Some(changed) = model.changed.as_mut() {
                    changed(Desktop::default());
                }
            })?;
        Ok(())
    }

    /// Asks to show a workspace.
    pub fn focus_workspace(&self, id: u64) {
        let handles = self.handles.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((_, workspace)) = handles.iter().find(|(k, _)| *k == id) {
            workspace.activate();
            self.manager.commit();
            let _ = self.connection.flush();
        }
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
        }
    }
}

impl Dispatch<ExtWorkspaceManagerV1, ()> for Model {
    fn event(
        model: &mut Self,
        _: &ExtWorkspaceManagerV1,
        event: manager::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            manager::Event::WorkspaceGroup { workspace_group } => model.groups.push(Group {
                proxy: workspace_group,
                outputs: Vec::new(),
            }),
            manager::Event::Workspace { workspace } => {
                let mut handles = model.handles.lock().unwrap_or_else(|e| e.into_inner());
                handles.push((key(&workspace), workspace.clone()));
                drop(handles);
                model.spaces.push(Space {
                    proxy: workspace,
                    group: None,
                    name: String::new(),
                    coordinates: Vec::new(),
                    state: 0,
                });
            }
            manager::Event::Done => {
                let desktop = model.desktop();
                if let Some(changed) = model.changed.as_mut() {
                    changed(desktop);
                }
            }
            _ => {}
        }
    }

    event_created_child!(Model, ExtWorkspaceManagerV1, [
        manager::EVT_WORKSPACE_GROUP_OPCODE => (ExtWorkspaceGroupHandleV1, ()),
        manager::EVT_WORKSPACE_OPCODE => (ExtWorkspaceHandleV1, ()),
    ]);
}

impl Dispatch<ExtWorkspaceGroupHandleV1, ()> for Model {
    fn event(
        model: &mut Self,
        proxy: &ExtWorkspaceGroupHandleV1,
        event: group::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let space = |model: &mut Self, workspace: &ExtWorkspaceHandleV1| {
            model.spaces.iter_mut().position(|s| &s.proxy == workspace)
        };
        match event {
            group::Event::OutputEnter { output } => {
                if let Some(g) = model.groups.iter_mut().find(|g| &g.proxy == proxy) {
                    g.outputs.push(output);
                }
            }
            group::Event::OutputLeave { output } => {
                if let Some(g) = model.groups.iter_mut().find(|g| &g.proxy == proxy) {
                    g.outputs.retain(|o| o != &output);
                }
            }
            group::Event::WorkspaceEnter { workspace } => {
                if let Some(at) = space(model, &workspace) {
                    model.spaces[at].group = Some(proxy.clone());
                }
            }
            group::Event::WorkspaceLeave { workspace } => {
                if let Some(at) = space(model, &workspace) {
                    model.spaces[at].group = None;
                }
            }
            group::Event::Removed => {
                model.groups.retain(|g| &g.proxy != proxy);
                proxy.destroy();
            }
            _ => {}
        }
    }
}

impl Dispatch<ExtWorkspaceHandleV1, ()> for Model {
    fn event(
        model: &mut Self,
        proxy: &ExtWorkspaceHandleV1,
        event: handle::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(at) = model.spaces.iter().position(|s| &s.proxy == proxy) else {
            return;
        };
        match event {
            handle::Event::Name { name } => model.spaces[at].name = name,
            handle::Event::Coordinates { coordinates } => {
                model.spaces[at].coordinates = coordinates
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|c| i32::from_ne_bytes([c[0], c[1], c[2], c[3]]))
                    .collect();
            }
            handle::Event::State {
                state: WEnum::Value(state),
            } => model.spaces[at].state = state.bits(),
            handle::Event::Removed => {
                model.spaces.remove(at);
                let id = key(proxy);
                model
                    .handles
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .retain(|(k, _)| *k != id);
                proxy.destroy();
            }
            _ => {}
        }
    }
}
