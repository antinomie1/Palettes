//! Shell-wide settings, panel placement and popup placement.

use aegle_platform::{Anchor, KeyboardInteractivity, Layer, LayerOptions, OutputId, Side};
use aegle_ui::{Rect, Theme, Transition};

/// Settings shared by every surface of a [`Shell`](crate::Shell).
#[derive(Clone, Debug)]
pub struct ShellOptions {
    /// Layer namespace compositors use to tell shells apart.
    pub namespace: String,
    /// Theme while the system prefers neither dark nor high contrast.
    pub theme: Theme,
    /// Theme while the system prefers dark; `None` ignores the preference.
    pub dark_theme: Option<Theme>,
    /// Transition given to interactive controls; `None` for none.
    pub transition: Option<Transition>,
    /// Bytes of Vulkan device memory each surface may allocate: a limit,
    /// by default room to upload one wallpaper-sized image (its atlas page
    /// and its staging copy) while the one it replaces is still resident.
    pub memory_budget: u64,
}

impl Default for ShellOptions {
    fn default() -> Self {
        Self {
            namespace: "palettes".into(),
            theme: Theme::default(),
            dark_theme: Some(Theme::dark()),
            transition: Some(Transition::default()),
            memory_budget: 128 * 1024 * 1024,
        }
    }
}

/// Where a panel sits: a layer surface, built from one of the constructors
/// and refined with the chained setters.
///
/// ```
/// # use palettes_shell::PanelOptions;
/// let bar = PanelOptions::top(32).margin([4, 8, 0, 8]);
/// ```
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PanelOptions {
    /// Stacking layer, edges, zone, keyboard policy, margin and output.
    pub layer: LayerOptions,
    /// Logical width; ignored along a stretched axis.
    pub width: u32,
    /// Logical height; ignored along a stretched axis.
    pub height: u32,
}

impl PanelOptions {
    fn edge(anchor: Anchor, width: u32, height: u32, zone: u32) -> Self {
        Self {
            layer: LayerOptions {
                layer: Layer::Top,
                anchor,
                exclusive_zone: zone as i32,
                keyboard: KeyboardInteractivity::None,
                margin: [0; 4],
                output: None,
            },
            width,
            height,
        }
    }

    /// A bar along the top edge, `height` tall, reserving its space.
    pub fn top(height: u32) -> Self {
        Self::edge(
            Anchor::TOP | Anchor::LEFT | Anchor::RIGHT,
            0,
            height,
            height,
        )
    }

    /// A bar along the bottom edge.
    pub fn bottom(height: u32) -> Self {
        Self::edge(
            Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT,
            0,
            height,
            height,
        )
    }

    /// A bar along the left edge, `width` wide.
    pub fn left(width: u32) -> Self {
        Self::edge(Anchor::LEFT | Anchor::TOP | Anchor::BOTTOM, width, 0, width)
    }

    /// A bar along the right edge.
    pub fn right(width: u32) -> Self {
        Self::edge(
            Anchor::RIGHT | Anchor::TOP | Anchor::BOTTOM,
            width,
            0,
            width,
        )
    }

    /// A centered surface above everything that takes the keyboard while
    /// shown, like a launcher or a dialog.
    pub fn overlay(width: u32, height: u32) -> Self {
        Self::edge(Anchor::NONE, width, height, 0)
            .layer(Layer::Overlay)
            .keyboard(KeyboardInteractivity::Exclusive)
    }

    /// A surface covering the whole output below every window: a wallpaper
    /// or desktop widgets.
    pub fn background() -> Self {
        let all = Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT;
        let mut options = Self::edge(all, 0, 0, 0).layer(Layer::Background);
        options.layer.exclusive_zone = -1;
        options
    }

    /// A free surface of `width` × `height` attached to `anchor` edges,
    /// reserving nothing, such as a notification in a corner.
    pub fn floating(anchor: Anchor, width: u32, height: u32) -> Self {
        Self::edge(anchor, width, height, 0)
    }

    /// On this output instead of the compositor's choice.
    pub fn on(mut self, output: OutputId) -> Self {
        self.layer.output = Some(output);
        self
    }

    /// In this stacking layer.
    pub fn layer(mut self, layer: Layer) -> Self {
        self.layer.layer = layer;
        self
    }

    /// With this keyboard policy.
    pub fn keyboard(mut self, keyboard: KeyboardInteractivity) -> Self {
        self.layer.keyboard = keyboard;
        self
    }

    /// Reserving `zone` logical pixels along its edge; -1 also ignores
    /// other surfaces' zones.
    pub fn exclusive(mut self, zone: i32) -> Self {
        self.layer.exclusive_zone = zone;
        self
    }

    /// With margins in CSS order: top, right, bottom, left.
    pub fn margin(mut self, margin: [i32; 4]) -> Self {
        self.layer.margin = margin;
        self
    }

    /// With a preferred size.
    pub fn size(mut self, width: u32, height: u32) -> Self {
        self.width = width;
        self.height = height;
        self
    }
}

/// Where a popup opens: next to a rectangle of its parent.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PopupOptions {
    /// The rectangle it attaches to, in the parent's logical coordinates,
    /// such as a control's `bounds()`.
    pub anchor: Rect,
    /// The side of `anchor` it opens on; the compositor flips or slides it
    /// to stay on screen.
    pub side: Side,
    /// Logical width.
    pub width: u32,
    /// Logical height.
    pub height: u32,
    /// Takes the keyboard and closes on a click elsewhere, like a menu.
    /// A passive popup, like a tooltip, takes nothing.
    pub grab: bool,
}

impl PopupOptions {
    /// A grabbing popup of `width` × `height` below `anchor`.
    pub fn below(anchor: Rect, width: u32, height: u32) -> Self {
        Self {
            anchor,
            side: Side::Below,
            width,
            height,
            grab: true,
        }
    }

    /// On another side of the anchor.
    pub fn side(mut self, side: Side) -> Self {
        self.side = side;
        self
    }

    /// Without a grab: no keyboard, not closed by clicks elsewhere.
    pub fn passive(mut self) -> Self {
        self.grab = false;
        self
    }
}
