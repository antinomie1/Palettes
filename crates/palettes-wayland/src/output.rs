//! Outputs: names from `wl_output` v4, logical geometry from xdg-output.

use crate::state::{Globals, State};
use aegle_platform::{Event, OutputId, OutputInfo, PixelSize, Point};
use wayland_client::{
    Connection, Dispatch, Proxy, QueueHandle, WEnum,
    protocol::wl_output::{self, Transform, WlOutput},
};
use wayland_protocols::xdg::xdg_output::zv1::client::zxdg_output_v1::{self, ZxdgOutputV1};

pub(crate) struct Output {
    /// The registry name, which is also its [`OutputId`].
    pub global: u32,
    pub output: WlOutput,
    xdg: Option<ZxdgOutputV1>,
    name: String,
    description: String,
    position: (i32, i32),
    logical: Option<(i32, i32)>,
    mode: (i32, i32),
    scale: i32,
    transform: Transform,
}

impl Output {
    pub fn new(global: u32, output: WlOutput, globals: &Globals, qh: &QueueHandle<State>) -> Self {
        let xdg = globals
            .xdg_output
            .as_ref()
            .map(|manager| manager.get_xdg_output(&output, qh, global));
        Self {
            global,
            output,
            xdg,
            name: String::new(),
            description: String::new(),
            position: (0, 0),
            logical: None,
            mode: (0, 0),
            scale: 1,
            transform: Transform::Normal,
        }
    }

    pub fn destroy(self) {
        if let Some(xdg) = self.xdg {
            xdg.destroy();
        }
        if self.output.version() >= 3 {
            self.output.release();
        }
    }

    pub fn id(&self) -> OutputId {
        OutputId(u64::from(self.global))
    }

    pub fn info(&self) -> OutputInfo {
        let rotated = matches!(
            self.transform,
            Transform::_90 | Transform::_270 | Transform::Flipped90 | Transform::Flipped270
        );
        let (mode_w, mode_h) = if rotated {
            (self.mode.1, self.mode.0)
        } else {
            self.mode
        };
        let (width, height) = self
            .logical
            .unwrap_or((mode_w / self.scale.max(1), mode_h / self.scale.max(1)));
        // xdg-output's logical size is the mode over the compositor's real,
        // possibly fractional, scale.
        let scale = if width > 0 && mode_w > 0 {
            mode_w as f32 / width as f32
        } else {
            self.scale as f32
        };
        OutputInfo {
            id: self.id(),
            name: self.name.clone(),
            description: self.description.clone(),
            position: Point::new(self.position.0 as f32, self.position.1 as f32),
            size: PixelSize {
                width: width.max(0) as u32,
                height: height.max(0) as u32,
            },
            scale,
        }
    }
}

impl State {
    fn output_mut(&mut self, global: u32) -> Option<&mut Output> {
        self.outputs.iter_mut().find(|o| o.global == global)
    }
}

impl Dispatch<WlOutput, u32> for State {
    fn event(
        state: &mut Self,
        _: &WlOutput,
        event: wl_output::Event,
        global: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(output) = state.output_mut(*global) else {
            return;
        };
        match event {
            wl_output::Event::Geometry {
                x, y, transform, ..
            } => {
                if output.logical.is_none() {
                    output.position = (x, y);
                }
                if let WEnum::Value(transform) = transform {
                    output.transform = transform;
                }
            }
            wl_output::Event::Mode {
                flags,
                width,
                height,
                ..
            } => {
                if let WEnum::Value(flags) = flags {
                    if flags.contains(wl_output::Mode::Current) {
                        output.mode = (width, height);
                    }
                }
            }
            wl_output::Event::Scale { factor } => output.scale = factor,
            wl_output::Event::Name { name } => output.name = name,
            wl_output::Event::Description { description } => output.description = description,
            wl_output::Event::Done => state.events.push_back(Event::Outputs),
            _ => {}
        }
    }
}

impl Dispatch<ZxdgOutputV1, u32> for State {
    fn event(
        state: &mut Self,
        _: &ZxdgOutputV1,
        event: zxdg_output_v1::Event,
        global: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(output) = state.output_mut(*global) else {
            return;
        };
        match event {
            zxdg_output_v1::Event::LogicalPosition { x, y } => output.position = (x, y),
            zxdg_output_v1::Event::LogicalSize { width, height } => {
                output.logical = Some((width, height));
            }
            // wl_output v4 names win; older compositors name outputs here only.
            zxdg_output_v1::Event::Name { name } if output.name.is_empty() => output.name = name,
            zxdg_output_v1::Event::Description { description } if output.description.is_empty() => {
                output.description = description;
            }
            _ => {}
        }
    }
}
