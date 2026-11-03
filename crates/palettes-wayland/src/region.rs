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

/// Appends rectangles covering `rect` with corners of `radius`, all in
/// whole logical pixels: one per row of a corner whose inset changes, and
/// one for the straight middle. Edges round to the nearest pixel, so the
/// region stays inside what a renderer would draw.
pub fn rounded_rects(rect: Rect, radius: f32, out: &mut Vec<PixelRect>) {
    let left = rect.origin.x.max(0.0).round();
    let top = rect.origin.y.max(0.0).round();
    let right = (rect.origin.x + rect.size.width).round();
    let bottom = (rect.origin.y + rect.size.height).round();
    if right <= left || bottom <= top {
        return;
    }
    let (width, height) = (right - left, bottom - top);
    let radius = radius.max(0.0).min(width / 2.0).min(height / 2.0);
    let rows = radius.ceil() as u32;
    let mut push = |x: f32, y: f32, w: f32, h: f32| {
        if w > 0.0 && h > 0.0 {
            out.push(PixelRect {
                x: x as u32,
                y: y as u32,
                width: w as u32,
                height: h as u32,
            });
        }
    };
    // Insets of each corner row, from the outer edge inwards, merged into runs.
    let inset = |row: u32| {
        let dy = radius - (row as f32 + 0.5);
        (radius - (radius * radius - dy * dy).max(0.0).sqrt()).round()
    };
    let mut row = 0;
    while row < rows {
        let x = inset(row);
        let run = (row..rows).take_while(|&r| inset(r) == x).count() as u32;
        let (w, h) = (width - 2.0 * x, run as f32);
        push(left + x, top + row as f32, w, h);
        push(left + x, bottom - (row + run) as f32, w, h);
        row += run;
    }
    push(left, top + rows as f32, width, height - 2.0 * rows as f32);
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
