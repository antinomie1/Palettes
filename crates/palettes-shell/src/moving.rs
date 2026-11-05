//! Surfaces the user drags around their output by a press on no control,
//! as a window by its title bar.

use crate::{Result, runtime::Runtime};
use aegle_platform::{Anchor, OutputInfo, Platform, PointerKind as Pointer, WindowId};
use aegle_ui::Point;

impl Runtime {
    /// Follows a pointer event the surface `id` has already handled. A
    /// primary press that reached no control starts a move; motion moves the
    /// surface while the button is held.
    pub(crate) fn follow_drag(
        &mut self,
        id: WindowId,
        kind: &Pointer,
        position: Point,
    ) -> Result<()> {
        let Some(entry) = self.entries.iter_mut().find(|e| e.id == id && e.movable) else {
            return Ok(());
        };
        match kind {
            Pointer::Down { .. } if entry.ui.with_state(|s| Ok(s.hover_target()))?.is_none() => {
                entry.moving = Some(position);
            }
            Pointer::Up | Pointer::Leave => entry.moving = None,
            // Positions are relative to where the surface was when the
            // compositor sent them: until it has applied the last move they
            // may not be, so they wait for the next.
            Pointer::Move if self.platform.placed(id) => {
                if let Some(at) = entry.moving {
                    self.move_by(id, Point::new(position.x - at.x, position.y - at.y))?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Moves a layer surface by `delta`, keeping it on its output: anchored
    /// to its top-left corner from then on.
    fn move_by(&mut self, id: WindowId, delta: Point) -> Result<()> {
        let Some((mut layer, size)) = self.platform.layer(id) else {
            return Ok(());
        };
        let outputs = self.platform.output_list();
        let Some(output) = (outputs.iter())
            .find(|o| Some(o.id) == layer.output)
            .or(outputs.first())
        else {
            return Ok(());
        };
        let shown = self.platform.window_info(id)?.size;
        let (width, height) = (shown.width as i32, shown.height as i32);
        let [x, y] = top_left(&layer, output, width, height);
        let x = (x + delta.x.round() as i32).clamp(0, (output.size.width as i32 - width).max(0));
        let y = (y + delta.y.round() as i32).clamp(0, (output.size.height as i32 - height).max(0));
        if [x, y] == top_left(&layer, output, width, height) {
            return Ok(());
        }
        layer.anchor = Anchor::TOP | Anchor::LEFT;
        layer.margin = [y, 0, 0, x];
        Ok(self.platform.set_layer(id, layer, size)?)
    }
}

/// Where a `width` × `height` layer surface sits on `output`: at its
/// anchored edges' margins, else centered.
fn top_left(
    layer: &aegle_platform::LayerOptions,
    output: &OutputInfo,
    width: i32,
    height: i32,
) -> [i32; 2] {
    let [top, right, bottom, left] = layer.margin;
    let place =
        |start: bool, end: bool, from_start: i32, from_end: i32, room: i32, size: i32| match (
            start, end,
        ) {
            (true, _) => from_start,
            (false, true) => room - size - from_end,
            (false, false) => (room - size) / 2,
        };
    let anchor = layer.anchor;
    [
        place(
            anchor.contains(Anchor::LEFT),
            anchor.contains(Anchor::RIGHT),
            left,
            right,
            output.size.width as i32,
            width,
        ),
        place(
            anchor.contains(Anchor::TOP),
            anchor.contains(Anchor::BOTTOM),
            top,
            bottom,
            output.size.height as i32,
            height,
        ),
    ]
}
