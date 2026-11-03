//! What a surface tells the compositor about its shape: where it takes input,
//! and where the desktop behind it is blurred.

use aegle_platform::WindowId;
use aegle_types::PixelRect;
use aegle_ui::{Node, NodeId, Result};
use palettes_wayland::{Wayland, rounded_rects};

#[derive(Default)]
pub(crate) struct Shape {
    /// Nodes that take input; none means the root.
    pub input: Vec<Node>,
    /// Nodes the compositor blurs behind.
    pub blur: Vec<Node>,
    /// Last sent, each with its holes; `None` is the whole surface, as a
    /// new one starts.
    input_sent: Option<(Vec<PixelRect>, Vec<PixelRect>)>,
    blur_sent: (Vec<PixelRect>, Vec<PixelRect>),
    /// The walk's stack, kept between syncs.
    stack: Vec<NodeId>,
}

/// Adds `node` to `nodes`, or with `on` false removes it.
pub(crate) fn toggle(nodes: &mut Vec<Node>, node: &Node, on: bool) {
    nodes.retain(|n| n.id() != node.id());
    if on {
        nodes.push(node.clone());
    }
}

/// The rounded bounds of every visible node.
fn region(nodes: &[Node]) -> Vec<PixelRect> {
    let mut rects = Vec::new();
    for node in nodes.iter().filter(|n| n.is_visible() && n.opacity() > 0.0) {
        let radii = node.corners().radii(node.appearance().radius);
        rounded_rects(node.bounds(), radii, &mut rects);
    }
    rects
}

/// The shown cut nodes of `root`'s tree: the desktop shows through them, so
/// they are holes in every region. One borrow walks the tree; cuts are rare,
/// so only they become handles.
fn holes(root: &Node, stack: &mut Vec<NodeId>) -> Vec<PixelRect> {
    let cuts = root.change(|state, id| {
        let mut cuts = Vec::new();
        stack.push(id);
        while let Some(id) = stack.pop() {
            let element = state.element(id);
            if element.is_shown() {
                if element.is_cut() {
                    cuts.push(root.handle(id));
                }
                stack.extend(state.children(id));
            }
        }
        Ok(cuts)
    });
    region(&cuts)
}

impl Shape {
    /// Sends the regions that changed since the last layout; `true` if any
    /// did, since only the next commit applies them.
    pub fn sync(&mut self, platform: &mut Wayland, id: WindowId, root: &Node) -> Result<bool> {
        let mut changed = false;
        let size = root.bounds().size;
        let whole = PixelRect {
            x: 0,
            y: 0,
            width: size.width.round() as u32,
            height: size.height.round() as u32,
        };
        // Before the first layout there is no shape to speak of.
        if whole.width == 0 || whole.height == 0 {
            return Ok(false);
        }
        let holes = holes(root, &mut self.stack);
        self.input.retain(Node::is_alive);
        let rects = match self.input.is_empty() {
            true => region(std::slice::from_ref(root)),
            false => region(&self.input),
        };
        let input = (rects != [whole] || !holes.is_empty()).then(|| (rects, holes.clone()));
        if input != self.input_sent {
            let rects = input.as_ref().map(|(rects, _)| rects.as_slice());
            platform.set_input(id, rects, &holes)?;
            self.input_sent = input;
            changed = true;
        }
        if !self.blur.is_empty() || !self.blur_sent.0.is_empty() {
            self.blur.retain(Node::is_alive);
            let blur = (region(&self.blur), holes);
            if blur != self.blur_sent {
                platform.set_blur(id, &blur.0, &blur.1)?;
                self.blur_sent = blur;
                changed = true;
            }
        }
        Ok(changed)
    }
}
