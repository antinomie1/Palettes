//! Palettes' markup elements: `Panel`, the root of a panel document,
//! `Icon`, an icon by theme name, and `Image` and `Shader`, which replace
//! aegle's to resolve their sources like every other asset and follow
//! their files.

use aegle_loader::{Elements, element};
use aegle_ui::{ColorSlot, Container, Node, NodeId, Theme};
use aegle_widgets::{Fit, ImageView, ShaderView, Widgets};
use palettes_shell::{KeyboardInteractivity, Layer, PanelOptions, PopupOptions, Side, Surface};
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
    /// A popup beside the control under the pointer, opened by
    /// `host.toggle` or `host.open`: `span` wide and `size` tall, or as
    /// large as its content where they are 0.
    Popup,
}

/// Where a `Panel` document's surface goes, as its properties say.
#[derive(Clone, Debug, PartialEq)]
pub struct Placement {
    /// The edge.
    pub edge: Edge,
    /// Thickness of a bar, or height of a centered panel or popup, in dp;
    /// 0 is 32 dp, or a popup's content height.
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
    /// A popup without a grab: no keyboard, and clicks elsewhere keep it.
    pub passive: bool,
}

impl Default for Placement {
    fn default() -> Self {
        Self {
            edge: Edge::Top,
            size: 0,
            span: 0,
            layer: None,
            exclusive: None,
            keyboard: KeyboardInteractivity::None,
            offset: 0,
            output: String::new(),
            passive: false,
        }
    }
}

impl Placement {
    /// The surface placement this describes.
    pub fn options(&self) -> PanelOptions {
        use palettes_shell::Anchor as A;
        let (size, span) = (if self.size == 0 { 32 } else { self.size }, self.span);
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
            // Popups are not layer surfaces; see `popup_options`.
            Edge::Popup => PanelOptions::overlay(self.popup_width(), size),
        };
        if let Some(layer) = self.layer {
            options = options.layer(layer);
        }
        if self.exclusive == Some(false) {
            options = options.exclusive(0);
        }
        options.keyboard(self.keyboard).margin([self.offset; 4])
    }

    /// A popup's first width: wide enough that only long text wraps, until
    /// it takes its content's size.
    fn popup_width(&self) -> u32 {
        if self.span == 0 { 480 } else { self.span }
    }

    /// A popup's placement next to `anchor` on a surface placed by `parent`:
    /// away from the parent's edge, so a bottom bar's popups open above it.
    pub fn popup_options(&self, anchor: aegle_ui::Rect, parent: &Placement) -> PopupOptions {
        let side = match parent.edge {
            Edge::Bottom => Side::Above,
            Edge::Left | Edge::Popup => Side::Right,
            Edge::Right => Side::Left,
            Edge::Top | Edge::Center | Edge::Background => Side::Below,
        };
        let options = PopupOptions::below(anchor, self.popup_width(), self.size.max(1)).side(side);
        if self.passive {
            options.passive()
        } else {
            options
        }
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
            .filter(|_| panel.placement != before && panel.placement.edge != Edge::Popup)?;
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
        "popup" => Edge::Popup,
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

thread_local! {
    /// Every live `Icon`'s view, name and size in dp, to show it again at a
    /// new size or scale.
    static ICONS: RefCell<HashMap<NodeId, (ImageView, String, f32)>> = RefCell::default();
}

/// Size of an `Icon` without `size`, in dp.
const ICON_SIZE: f32 = 16.0;

/// Shows the icon `name`: a theme icon or application id (`firefox`,
/// `audio-volume-high`), or a path, relative to the asset roots, at its
/// size on the densest output. Empty clears it; a change to its file shows
/// the new one.
fn show_icon(view: &ImageView, name: &str) {
    let size = ICONS.with_borrow_mut(|icons| {
        icons.retain(|_, (v, _, _)| v.is_alive());
        let entry = icons
            .entry(view.id())
            .or_insert_with(|| (view.clone(), String::new(), ICON_SIZE));
        entry.1 = name.to_owned();
        entry.2
    });
    if name.is_empty() {
        crate::markup::image::clear(view);
        return;
    }
    let pixels = crate::markup::image::pixels(size);
    let looks_like_path = name.contains('/') || name.ends_with(".png") || name.ends_with(".svg");
    let found = if looks_like_path {
        crate::markup::roots().resolve(name)
    } else {
        let icons = crate::services::icons();
        icons
            .lookup(name, pixels)
            .or_else(|| icons.for_app(name, pixels))
    };
    match found {
        Some(path) => {
            if let Err(e) = crate::markup::image::show_at(view, &path, pixels) {
                eprintln!("palettes: icon {}: {e}", path.display());
            }
        }
        None => crate::markup::image::clear(view),
    }
}

/// Shows the image at `source`, relative to the asset roots; empty clears it.
/// A failure reaches the element's `failed` event; a change to the file shows
/// the new one.
fn show_image(view: &ImageView, source: &str) {
    if source.is_empty() {
        crate::markup::image::clear(view);
        return;
    }
    let path = crate::markup::roots()
        .resolve(source)
        .unwrap_or_else(|| source.into());
    let _ = crate::markup::image::show(view, &path);
}

/// Shows the program at `source`, relative to the asset roots; empty clears it.
fn show_shader(view: &ShaderView, source: &str) {
    crate::markup::shader::show(view, source);
}

fn image_blur(view: &ImageView, radius: f64) {
    crate::markup::image::restyle(view, |look| look.blur = radius as f32, true);
}

fn image_palette(view: &ImageView, palette: bool) {
    crate::markup::image::restyle(view, |look| look.palette = palette, true);
}

fn fit(name: &str) -> Fit {
    match name {
        "fill" => Fit::Fill,
        "contain" => Fit::Contain,
        "cover" => Fit::Cover,
        other => unreachable!("checked fit `{other}`"),
    }
}

fn icon_view(parent: &Container) -> ImageView {
    let view = parent.image_view();
    view.set_width(ICON_SIZE);
    view.set_height(ICON_SIZE);
    view
}

/// Sizes an `Icon`, showing its icon again for the new size.
fn icon_size(view: &ImageView, size: f32) {
    view.set_width(size);
    view.set_height(size);
    let name = ICONS.with_borrow_mut(|icons| {
        let entry = icons.get_mut(&view.id())?;
        entry.2 = size;
        Some(entry.1.clone())
    });
    if let Some(name) = name.filter(|n| !n.is_empty()) {
        show_icon(view, &name);
    }
}

/// Shows every icon again once the densest output's scale changed.
pub(crate) fn rescale(scale: f32) {
    if !crate::markup::image::set_scale(scale) {
        return;
    }
    let shown: Vec<_> = ICONS.with_borrow(|icons| {
        (icons
            .values()
            .filter(|(v, name, _)| v.is_alive() && !name.is_empty()))
        .map(|(v, name, _)| (v.clone(), name.clone()))
        .collect()
    });
    for (view, name) in shown {
        show_icon(&view, &name);
    }
}

element! {
    /// The root of a panel document: a column that becomes a layer surface
    /// on every output (or on `output`), placed by its properties.
    pub Panel(Container) {
        layout flex;
        create |parent| panel(parent);
        set edge: choice(top, bottom, left, right, center, background, popup) => |p, v| update(p, |s| s.edge = edge(v));
        set size: length => |p, v| update(p, |s| s.size = v.max(0.0) as u32);
        set span: length => |p, v| update(p, |s| s.span = v.max(0.0) as u32);
        set layer: choice(background, bottom, top, overlay) => |p, v| update(p, |s| s.layer = Some(layer(v)));
        set exclusive: bool => |p, v| update(p, |s| s.exclusive = Some(v));
        set keyboard: choice(none, exclusive, on_demand) => |p, v| update(p, |s| s.keyboard = keyboard(v));
        set offset: length => |p, v| update(p, |s| s.offset = v as i32);
        set output: line => |p, v| update(p, |s| s.output = v.to_owned());
        set passive: bool => |p, v| update(p, |s| s.passive = v);
        set blur: bool => set_blur;
    }
    /// An icon by theme name or application id, or an image path; `size` dp square.
    pub Icon(ImageView) {
        create |parent| icon_view(parent);
        set name: line => show_icon;
        set size: length => icon_size;
    }
    /// An image file relative to the asset roots, placed by `fit`. In a
    /// `Stack` with `fit: cover` and a `radius`, it is a rounded background.
    /// `blur` blurs it by a radius in its own pixels; `palette: true` makes
    /// its main colors the shell's palette.
    pub Image(ImageView) {
        create |parent| parent.image_view();
        set source: line => show_image;
        set fit: choice(fill, contain, cover) => |view, name| view.set_fit(fit(name));
        set blur: float(0, 1000) => image_blur;
        set palette: bool => image_palette;
        event failed => |view, run| view.on_error(move |_| run());
        get error: string => |view| view.error().unwrap_or_default();
    }
    /// A view whose background is the WGSL program at `source`, relative to
    /// the asset roots, drawn with `values` (see aegle's `Shader`); saving
    /// the file shows the new program. A program that does not compile
    /// empties the view, is logged and raises `failed`.
    pub Shader(ShaderView) {
        create |parent| parent.shader_view();
        set source: line => show_shader;
        set values: numbers(8) => |view, values| view.set_values(&values.to_vec());
        event failed => |view, run| view.on_error(move |_| run());
        get error: string => |view| view.error().unwrap_or_default();
    }
}

/// Aegle's built-in elements and Palettes' own, plus am3's `Md*` elements
/// with the `am3` feature.
pub fn elements() -> Elements {
    #[cfg(feature = "am3")]
    let base = am3::elements();
    #[cfg(not(feature = "am3"))]
    let base = Elements::new();
    base.with::<Panel>()
        .with::<Icon>()
        .with::<Image>()
        .with::<Shader>()
}
