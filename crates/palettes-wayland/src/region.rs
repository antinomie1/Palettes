//! Surface regions: where a surface takes input, and where the compositor
//! blurs what lies behind it (ext-background-effect-v1).

use crate::state::State;
use aegle_platform::{Error, PixelRect, Rect, Result, WindowId};
use wayland_client::{
    Connection, Dispatch, QueueHandle, delegate_noop, protocol::wl_region::WlRegion,
};
use wayland_protocols::ext::background_effect::v1::client::{
    ext_background_effect_manager_v1::{self, ExtBackgroundEffectManagerV1},
    ext_background_effect_surface_v1::ExtBackgroundEffectSurfaceV1,
};

/// Appends rectangles covering `rect` with top-left, top-right, bottom-right
/// and bottom-left corners of `radii`, all in whole logical pixels: one per
/// run of rows with the same insets, so a corner row whose inset changes
/// starts a rectangle and the straight middle is one. Radii shrink like
/// aegle's, and edges round to the nearest pixel, so the region stays
/// inside what a renderer would draw.
pub fn rounded_rects(rect: Rect, radii: [f32; 4], out: &mut Vec<PixelRect>) {
    let left = rect.origin.x.max(0.0).round();
    let top = rect.origin.y.max(0.0).round();
    let right = (rect.origin.x + rect.size.width).round();
    let bottom = (rect.origin.y + rect.size.height).round();
    if right <= left || bottom <= top {
        return;
    }
    let (width, height) = (right - left, bottom - top);
    let [top_left, top_right, bottom_right, bottom_left] = radii.map(|r| r.max(0.0));
    // Where two corners of a side would overlap, every radius shrinks alike.
    let factor = [
        (width, top_left + top_right),
        (width, bottom_left + bottom_right),
        (height, top_left + bottom_left),
        (height, top_right + bottom_right),
    ]
    .iter()
    .filter(|(_, sum)| *sum > 0.0)
    .fold(1.0f32, |factor, (length, sum)| factor.min(length / sum));
    // The inset of pixel row `row` counted from the edge a corner of `radius`
    // touches, zero below the corner.
    let inset = |radius: f32, row: f32| {
        let radius = radius * factor;
        if row >= radius.ceil() {
            return 0.0;
        }
        let dy = radius - (row + 0.5);
        (radius - (radius * radius - dy * dy).max(0.0).sqrt()).round()
    };
    let insets = |y: f32| {
        let from_bottom = height - 1.0 - y;
        (
            inset(top_left, y).max(inset(bottom_left, from_bottom)),
            inset(top_right, y).max(inset(bottom_right, from_bottom)),
        )
    };
    let mut y = 0.0;
    while y < height {
        let (start, end) = insets(y);
        let mut run = 1.0;
        while y + run < height && insets(y + run) == (start, end) {
            run += 1.0;
        }
        let w = width - start - end;
        if w > 0.0 {
            out.push(PixelRect {
                x: (left + start) as u32,
                y: (top + y) as u32,
                width: w as u32,
                height: run as u32,
            });
        }
        y += run;
    }
}

impl State {
    fn region(&self, rects: &[PixelRect]) -> WlRegion {
        let region = self.globals.compositor.create_region(&self.qh, ());
        for r in rects {
            region.add(r.x as i32, r.y as i32, r.width as i32, r.height as i32);
        }
        region
    }

    /// Sets where a window takes pointer and touch input; `None` for all of
    /// it. Takes effect with the window's next commit.
    pub fn set_input(&mut self, id: WindowId, rects: Option<&[PixelRect]>) -> Result<()> {
        let region = rects.map(|rects| self.region(rects));
        let window = self.window(id).ok_or(Error::InvalidWindow)?;
        window.surface.set_input_region(region.as_ref());
        if let Some(region) = region {
            region.destroy();
        }
        Ok(())
    }

    /// Sets the blur region of a window; empty removes it. Takes effect with
    /// the window's next commit. Without the protocol, nothing happens.
    pub fn set_blur(&mut self, id: WindowId, rects: &[PixelRect]) -> Result<()> {
        let qh = self.qh.clone();
        let Some(manager) = self.globals.background_effect.clone() else {
            self.window(id).ok_or(Error::InvalidWindow)?;
            return Ok(());
        };
        let region = (!rects.is_empty()).then(|| self.region(rects));
        let window = self.window_mut(id).ok_or(Error::InvalidWindow)?;
        if rects.is_empty() && window.effect.is_none() {
            return Ok(());
        }
        let effect = window
            .effect
            .get_or_insert_with(|| manager.get_background_effect(&window.surface, &qh, ()));
        effect.set_blur_region(region.as_ref());
        if let Some(region) = region {
            region.destroy();
        }
        Ok(())
    }
}

/// Capabilities only say whether blur shows right now; regions are kept
/// either way, and the compositor applies them when it can.
impl Dispatch<ExtBackgroundEffectManagerV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &ExtBackgroundEffectManagerV1,
        _: ext_background_effect_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

delegate_noop!(State: ignore ExtBackgroundEffectSurfaceV1);
delegate_noop!(State: ignore WlRegion);
