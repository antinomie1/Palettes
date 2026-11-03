//! Palettes' markup elements: `Panel`, the root of a panel document, and
//! `Icon`, an icon by theme name.

use aegle_loader::{Elements, element};
use aegle_ui::{ColorSlot, Container, Node, NodeId, Theme};
use aegle_widgets::{ImageView, Widgets};
use palettes_shell::{KeyboardInteractivity, Layer, PanelOptions, Surface};
use std::{cell::RefCell, collections::HashMap};

/// The edge of an output a panel sits on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    /// A bar along the top.
    Top,
    /// A bar along the bottom.
    Bottom,
    /// A bar along the left.
    Left,
    /// A bar along the right.
    Right,
    /// Centered above everything, like a launcher.
    Center,
    /// Covering the output below every window.
    Background,
}

/// Where a `Panel` document's surface goes, as its properties say.
#[derive(Clone, Debug, PartialEq)]
pub struct Placement {
    /// The edge.
    pub edge: Edge,
    /// Thickness of a bar, or height of a centered panel, in dp.
    pub size: u32,
    /// Length along the edge (or width of a centered panel); 0 stretches.
    pub span: u32,
    /// Stacking layer; by default overlay for centered panels, background
    /// for backgrounds and top for bars.
    pub layer: Option<Layer>,
    /// Reserves its space so windows avoid it; by default bars do.
    pub exclusive: Option<bool>,
    /// Keyboard policy.
    pub keyboard: KeyboardInteractivity,
    /// Distance from the anchored edges, in dp.
    pub offset: i32,
    /// The only output it appears on; empty for every output.
    pub output: String,
}

impl Default for Placement {
    fn default() -> Self {
        Self {
            edge: Edge::Top,
            size: 32,
            span: 0,
            layer: None,
            exclusive: None,
            keyboard: KeyboardInteractivity::None,
            offset: 0,
            output: String::new(),
        }
    }
}

impl Placement {
    /// The surface placement this describes.
    pub fn options(&self) -> PanelOptions {
        use palettes_shell::Anchor as A;
        let (size, span) = (self.size, self.span);
        let bar = |anchor, stretched: PanelOptions, length: (u32, u32)| {
            if span == 0 {
                stretched
            } else {
                PanelOptions::floating(anchor, length.0, length.1).exclusive(size as i32)
            }
        };
        let mut options = match self.edge {
            Edge::Top => bar(A::TOP, PanelOptions::top(size), (span, size)),
            Edge::Bottom => bar(A::BOTTOM, PanelOptions::bottom(size), (span, size)),
            Edge::Left => bar(A::LEFT, PanelOptions::left(size), (size, span)),
            Edge::Right => bar(A::RIGHT, PanelOptions::right(size), (size, span)),
            Edge::Center => PanelOptions::overlay(if span == 0 { 480 } else { span }, size)
                .keyboard(KeyboardInteractivity::None),
            Edge::Background => PanelOptions::background(),
        };
        if let Some(layer) = self.layer {
            options = options.layer(layer);
        }
        if self.exclusive == Some(false) {
            options = options.exclusive(0);
        }
        options.keyboard(self.keyboard).margin([self.offset; 4])
    }
}

struct Registered {
    node: Node,
    placement: Placement,
    /// The compositor blurs the desktop behind the panel's shape.
    blur: bool,
    surface: Option<Surface>,
}

thread_local! {
    /// Every live `Panel` element, by node.
    static PANELS: RefCell<HashMap<NodeId, Registered>> = RefCell::default();
}

fn panel(parent: &Container) -> Container {
    let column = parent.column();
    // The panel is the visible shape, filling a transparent surface: its own
    // `background`, `radius` and `opacity` decide what shows of the desktop.
    column.set_grow(1.0);
    column.bind_color(ColorSlot::Background, Theme::BACKGROUND);
    PANELS.with_borrow_mut(|panels| {
        panels.retain(|_, p| p.node.is_alive());
        let node = (*column).clone();
        panels.insert(
            node.id(),
            Registered {
                node,
                placement: Placement::default(),
                blur: false,
                surface: None,
            },
        );
    });
    column
}

/// Changes a panel's placement, moving its surface if it has one.
fn update(container: &Container, change: impl FnOnce(&mut Placement)) {
    let moved = PANELS.with_borrow_mut(|panels| {
        let panel = panels.get_mut(&container.id())?;
        let before = panel.placement.clone();
        change(&mut panel.placement);
        let surface = panel
            .surface
            .as_ref()
            .filter(|_| panel.placement != before)?;
        Some((surface.clone(), panel.placement.options()))
    });
    if let Some((surface, options)) = moved
        && let Err(e) = surface.set_placement(options)
    {
        eprintln!("palettes: moving a panel: {e}");
    }
}

/// Turns the compositor's blur behind a panel on or off.
fn set_blur(container: &Container, blur: bool) {
    let surface = PANELS.with_borrow_mut(|panels| {
        let panel = panels.get_mut(&container.id())?;
        panel.blur = blur;
        panel.surface.clone()
    });
    if let Some(surface) = surface
        && let Err(e) = surface.set_blur(container, blur)
    {
        eprintln!("palettes: blurring a panel: {e}");
    }
}

/// The placement of the `Panel` at `node`, if it is one.
pub(crate) fn placement(node: &Node) -> Option<Placement> {
    PANELS.with_borrow(|panels| panels.get(&node.id()).map(|p| p.placement.clone()))
}

/// Ties the `Panel` at `node` to its surface, so later changes move it.
pub(crate) fn attach(node: &Node, surface: &Surface) {
    let blur = PANELS.with_borrow_mut(|panels| {
        let panel = panels.get_mut(&node.id())?;
        panel.surface = Some(surface.clone());
        Some(panel.blur)
    });
    let Some(blur) = blur else { return };
    // The panel is the surface's shape: clicks around its corners go through.
    let shaped = surface
        .set_input(node, true)
        .and_then(|()| surface.set_blur(node, blur));
    if let Err(e) = shaped {
        eprintln!("palettes: shaping a panel: {e}");
    }
}

fn edge(name: &str) -> Edge {
    match name {
        "top" => Edge::Top,
        "bottom" => Edge::Bottom,
        "left" => Edge::Left,
        "right" => Edge::Right,
        "center" => Edge::Center,
        "background" => Edge::Background,
        other => unreachable!("checked edge `{other}`"),
    }
}

fn layer(name: &str) -> Layer {
    match name {
        "background" => Layer::Background,
        "bottom" => Layer::Bottom,
        "top" => Layer::Top,
        "overlay" => Layer::Overlay,
        other => unreachable!("checked layer `{other}`"),
    }
}

fn keyboard(name: &str) -> KeyboardInteractivity {
    match name {
        "none" => KeyboardInteractivity::None,
        "exclusive" => KeyboardInteractivity::Exclusive,
        "on_demand" => KeyboardInteractivity::OnDemand,
        other => unreachable!("checked keyboard `{other}`"),
    }
}

/// Shows the icon `name`: a theme icon or application id (`firefox`,
/// `audio-volume-high`), or a path, relative to the asset roots. Empty clears it.
fn show_icon(view: &ImageView, name: &str) {
    if name.is_empty() {
        view.clear();
        return;
    }
    let looks_like_path = name.contains('/') || name.ends_with(".png") || name.ends_with(".svg");
    let found = if looks_like_path {
        crate::markup::roots().resolve(name)
    } else {
        let icons = crate::services::icons();
        icons.lookup(name, 64).or_else(|| icons.for_app(name, 64))
    };
    match found {
        Some(path) => {
            if let Err(e) = load(view, &path) {
                eprintln!("palettes: icon {}: {e}", path.display());
            }
        }
        None => view.clear(),
    }
}

/// Icon size in pixels SVG icons are rasterized at: crisp up to 32 dp at 2×.
const ICON_PIXELS: u32 = 64;

/// Loads a bitmap, or rasterizes an SVG, into `view`.
fn load(view: &ImageView, path: &std::path::Path) -> aegle_ui::Result<()> {
    #[cfg(feature = "svg")]
    if path.extension().is_some_and(|e| e == "svg") {
        let bytes = std::fs::read(path)?;
        let image = aegle_image::svg::rasterize(&bytes, ICON_PIXELS, ICON_PIXELS)?;
        view.set_image(&image);
        return Ok(());
    }
    view.load(path)
}

fn icon_view(parent: &Container) -> ImageView {
    let view = parent.image_view();
    view.set_width(16.0);
    view.set_height(16.0);
    view
}

element! {
    /// The root of a panel document: a column that becomes a layer surface
    /// on every output (or on `output`), placed by its properties.
    pub Panel(Container) {
        layout flex;
        create |parent| panel(parent);
        set edge: choice(top, bottom, left, right, center, background) => |p, v| update(p, |s| s.edge = edge(v));
        set size: length => |p, v| update(p, |s| s.size = v.max(1.0) as u32);
        set span: length => |p, v| update(p, |s| s.span = v.max(0.0) as u32);
        set layer: choice(background, bottom, top, overlay) => |p, v| update(p, |s| s.layer = Some(layer(v)));
        set exclusive: bool => |p, v| update(p, |s| s.exclusive = Some(v));
        set keyboard: choice(none, exclusive, on_demand) => |p, v| update(p, |s| s.keyboard = keyboard(v));
        set offset: length => |p, v| update(p, |s| s.offset = v as i32);
        set output: line => |p, v| update(p, |s| s.output = v.to_owned());
        set blur: bool => set_blur;
    }
    /// An icon by theme name or application id, or an image path; `size` dp square.
    pub Icon(ImageView) {
        create |parent| icon_view(parent);
        set name: line => show_icon;
        set size: length => |view, size| { view.set_width(size); view.set_height(size) };
    }
}

/// Aegle's built-in elements and Palettes' own, plus am3's `Md*` elements
/// with the `am3` feature.
pub fn elements() -> Elements {
    #[cfg(feature = "am3")]
    let base = am3::elements();
    #[cfg(not(feature = "am3"))]
    let base = Elements::new();
    base.with::<Panel>().with::<Icon>()
}
