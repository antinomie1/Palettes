//! What a surface tells the compositor about its shape: where it takes input,
//! and where the desktop behind it is blurred.

use aegle_platform::WindowId;
use aegle_types::PixelRect;
use aegle_ui::{Node, Result};
use palettes_wayland::{Wayland, rounded_rects};

#[derive(Default)]
pub(crate) struct Shape {
    /// Nodes that take input; none means the root.
    pub input: Vec<Node>,
    /// Nodes the compositor blurs behind.
    pub blur: Vec<Node>,
    /// Last sent; `None` is the whole surface, as a new one starts.
    input_sent: Option<Vec<PixelRect>>,
    blur_sent: Vec<PixelRect>,
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
        rounded_rects(node.bounds(), node.appearance().radius, &mut rects);
    }
    rects
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
        if whole.width > 0 && whole.height > 0 {
            self.input.retain(Node::is_alive);
            let rects = match self.input.is_empty() {
                true => region(std::slice::from_ref(root)),
                false => region(&self.input),
            };
            let rects = (rects != [whole]).then_some(rects);
            if rects != self.input_sent {
                platform.set_input(id, rects.as_deref())?;
                self.input_sent = rects;
                changed = true;
            }
        }
        if !self.blur.is_empty() || !self.blur_sent.is_empty() {
            self.blur.retain(Node::is_alive);
            let rects = region(&self.blur);
            if rects != self.blur_sent {
                platform.set_blur(id, &rects)?;
                self.blur_sent = rects;
                changed = true;
            }
        }
        Ok(changed)
    }
}
