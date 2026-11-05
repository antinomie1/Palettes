//! ext-workspace-v1: workspaces, their outputs, and which are shown.

use crate::{
    Desktop, Workspace,
    wayland::{Model, key},
};
use wayland_client::{
    Connection, Dispatch, QueueHandle, WEnum, event_created_child, protocol::wl_output::WlOutput,
};
use wayland_protocols::ext::workspace::v1::client::{
    ext_workspace_group_handle_v1::{self as group, ExtWorkspaceGroupHandleV1},
    ext_workspace_handle_v1::{self as handle, ExtWorkspaceHandleV1},
    ext_workspace_manager_v1::{self as manager, ExtWorkspaceManagerV1},
};

pub(crate) struct Group {
    proxy: ExtWorkspaceGroupHandleV1,
    outputs: Vec<WlOutput>,
}

pub(crate) struct Space {
    proxy: ExtWorkspaceHandleV1,
    group: Option<ExtWorkspaceGroupHandleV1>,
    name: String,
    coordinates: Vec<i32>,
    state: u32,
}

impl Model {
    /// Grouped workspaces by group, then any without a group.
    pub(crate) fn add_workspaces(&self, desktop: &mut Desktop, focused: Option<&str>) {
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
                    focused: active && focused == Some(output.as_str()),
                    ..Workspace::default()
                });
            }
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
                model
                    .handles()
                    .workspaces
                    .push((key(&workspace), workspace.clone()));
                model.spaces.push(Space {
                    proxy: workspace,
                    group: None,
                    name: String::new(),
                    coordinates: Vec::new(),
                    state: 0,
                });
            }
            manager::Event::Done => model.dirty = true,
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
                model.handles().workspaces.retain(|(k, _)| *k != id);
                proxy.destroy();
            }
            _ => {}
        }
    }
}
