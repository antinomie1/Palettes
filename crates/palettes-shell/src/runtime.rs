//! The state behind a [`Shell`](crate::Shell): surfaces, their trees and their
//! Vulkan presenters, the Wayland connection and the fonts.
#![allow(unsafe_code)]

use crate::shape::Shape;
use crate::{ShellOptions, input::EventClock};
use aegle_platform::{
    PixelSize, Platform, Preferences, SeatId, SurfacePlatform, WindowId, WindowOptions,
};
use aegle_render_vulkan::{SharedDevice, WindowRenderer};
use aegle_scene::Affine;
use aegle_ui::{
    Color, ColorSlot, Cursor, Modifiers, Node, Rect, Result, Size, TextSystem, Theme, Ui, Visit,
};
use aegle_widgets::HostTooltip;
use palettes_wayland::{Wayland, WindowSurface};
use std::{cell::RefCell, rc::Rc, time::Instant};

/// Pixels of the largest image a surface uploads: a 3840 × 2400 wallpaper,
/// 35 MiB of RGBA, twice over while it uploads (page and staging), which the
/// default memory budget holds beside the image it replaces and the
/// surface's text. Budgets are limits: smaller images take what they need.
pub const MAX_IMAGE_PIXELS: u32 = 3840 * 2400;

/// One such image plus aegle's default 1 MiB for the frame's glyphs.
const UPLOAD_BYTES: usize = MAX_IMAGE_PIXELS as usize * 4 + 1024 * 1024;

pub(crate) struct Entry {
    pub id: WindowId,
    pub ui: Rc<Ui>,
    /// Dropped before the surface is removed: it holds the swapchain.
    pub renderer: Option<WindowRenderer<WindowSurface>>,
    pub cursor: Cursor,
    pub seat: Option<SeatId>,
    pub modifiers: Modifiers,
    pub clock: EventClock,
    /// Recent scroll samples for kinetic scrolling.
    pub flick: Vec<(u32, aegle_ui::Point)>,
    /// The platform allows a frame now.
    pub ready: bool,
    pub parent: Option<WindowId>,
    pub shape: Shape,
    /// The node whose laid-out size the surface takes (a popup's content).
    pub fit: Option<Node>,
    /// The size last asked for to fit; frames wait until it is configured.
    pub fitting: Option<PixelSize>,
    /// The passive popup showing this surface's tooltip.
    pub tooltip: Option<WindowId>,
    /// A session lock surface.
    pub lock: bool,
    /// Its last frame failed to draw.
    pub failed: bool,
}

pub(crate) struct Runtime {
    // Fields drop in order: presenters before the device, before the connection.
    pub entries: Vec<Entry>,
    pub device: Option<SharedDevice>,
    pub platform: Wayland,
    pub fonts: Rc<RefCell<TextSystem>>,
    pub options: ShellOptions,
    pub preferences: Preferences,
    pub clock: Instant,
    /// What to run when a surface closes, by surface.
    pub closers: Vec<(WindowId, Box<dyn FnOnce()>)>,
    /// Closers of closed surfaces, run outside the runtime's borrow.
    pub closed: Vec<Box<dyn FnOnce()>>,
    /// Color tokens every surface takes.
    pub tokens: Vec<(aegle_ui::Token<Color>, Color)>,
}

/// Scales the UI's logical scenes to device pixels.
fn draw(ui: &Ui, factor: f32, frame: &mut aegle_render_vulkan::Frame<'_>) -> Result<()> {
    let scale = Affine::scale(factor, factor);
    // Clips snap to whole device pixels like a scissor rectangle.
    let snap = |rect: Rect| {
        let (left, top) = (
            (rect.origin.x * factor).round(),
            (rect.origin.y * factor).round(),
        );
        let right = ((rect.origin.x + rect.size.width) * factor).round();
        let bottom = ((rect.origin.y + rect.size.height) * factor).round();
        Rect::new(left, top, right - left, bottom - top)
    };
    ui.visit_scenes(|visit| match visit {
        Visit::Scene {
            scene,
            transform,
            clip,
        } => Ok(frame.draw_clipped(scene, transform.then(scale)?, clip.map(snap))?),
        Visit::PushLayer(layer) => {
            let clip = layer.clip().map(snap);
            Ok(frame.push_layer(&layer.then(scale)?.with_clip(clip)?)?)
        }
        Visit::PopLayer => Ok(frame.pop_layer()?),
    })
}

impl Runtime {
    pub fn theme(&self) -> Theme {
        match (self.preferences.dark, self.options.dark_theme) {
            (Some(true), Some(dark)) => dark,
            _ => self.options.theme,
        }
    }

    pub fn entry(&self, id: WindowId) -> Option<&Entry> {
        self.entries.iter().find(|e| e.id == id)
    }

    /// Opens a layer or popup surface with its own tree and presenter.
    pub fn open(
        &mut self,
        options: WindowOptions<'_>,
        size: PixelSize,
    ) -> Result<(WindowId, Rc<Ui>)> {
        let parent = options.popup.map(|p| p.parent);
        self.open_with(size, parent, |platform| platform.create_window(options))
    }

    /// Opens a surface `create` makes, with its own tree and presenter.
    pub fn open_with(
        &mut self,
        size: PixelSize,
        parent: Option<WindowId>,
        create: impl FnOnce(&mut Wayland) -> aegle_platform::Result<WindowId>,
    ) -> Result<(WindowId, Rc<Ui>)> {
        let ui = Rc::new(Ui::with_fonts(self.fonts.clone(), self.theme())?);
        ui.advance_animations(self.clock.elapsed())?;
        ui.set_default_transition(self.options.transition);
        ui.set_reduced_motion(self.preferences.reduced_motion.unwrap_or(false));
        if let Some(interval) = self.preferences.double_click {
            ui.set_double_click(interval, 4.0);
        }
        ui.resize(Size::new(size.width as f32, size.height as f32));
        // Surfaces clear to transparent; the root paints the theme's
        // background, so a translucent or rounded one shows the desktop.
        ui.root()
            .bind_color(ColorSlot::Background, Theme::BACKGROUND);
        // A bar is too small to hold its tooltips: they become popups.
        aegle_widgets::host_tooltips(&ui)?;
        for &(token, color) in &self.tokens {
            ui.set_token(token, color);
        }
        let id = create(&mut self.platform)?;
        let renderer = match self.presenter(id) {
            Ok(renderer) => renderer,
            Err(error) => {
                self.platform.remove_window(id)?;
                return Err(error);
            }
        };
        self.device.get_or_insert_with(|| renderer.shared_device());
        self.entries.push(Entry {
            id,
            ui: ui.clone(),
            renderer: Some(renderer),
            cursor: Cursor::Default,
            seat: None,
            modifiers: Modifiers::default(),
            clock: EventClock::default(),
            flick: Vec::new(),
            ready: false,
            parent,
            shape: Shape::default(),
            fit: None,
            fitting: None,
            tooltip: None,
            lock: false,
            failed: false,
        });
        Ok((id, ui))
    }

    fn presenter(&mut self, id: WindowId) -> Result<WindowRenderer<WindowSurface>> {
        let lease = self.platform.window_surface(id)?;
        let options = aegle_render_vulkan::Options {
            memory_budget: self.options.memory_budget,
            // Shell surfaces are often translucent or rounded.
            transparent: true,
            text: aegle_render_vulkan::TextOptions {
                upload_bytes: UPLOAD_BYTES,
                ..Default::default()
            },
            ..Default::default()
        };
        // SAFETY: the lease keeps the wl_surface and display alive, and the
        // presenter is dropped in `close` before the surface is destroyed.
        let renderer = unsafe {
            match &self.device {
                Some(device) => WindowRenderer::with_device(lease, options, device),
                None => WindowRenderer::new(lease, options),
            }
        }?;
        Ok(renderer)
    }

    /// Closes a surface after its popups, innermost first.
    pub fn close(&mut self, id: WindowId) -> Result<()> {
        while let Some(child) = self.entries.iter().find(|e| e.parent == Some(id)) {
            let child = child.id;
            self.close(child)?;
        }
        let index = self
            .entries
            .iter()
            .position(|e| e.id == id)
            .ok_or(aegle_ui::UiError::DeadHandle)?;
        let mut entry = self.entries.remove(index);
        let (closers, kept) = std::mem::take(&mut self.closers)
            .into_iter()
            .partition::<Vec<_>, _>(|(window, _)| *window == id);
        self.closers = kept;
        self.closed.extend(closers.into_iter().map(|(_, f)| f));
        entry.ui.close();
        drop(entry.renderer.take());
        self.platform.remove_window(id)?;
        Ok(())
    }

    /// Applies new system preferences to surfaces still using the old ones.
    pub fn set_preferences(&mut self, preferences: Preferences) {
        let old = self.theme();
        self.preferences = preferences;
        let theme = self.theme();
        for entry in &self.entries {
            if entry.ui.theme() == old {
                entry.ui.set_theme(theme);
            }
            entry
                .ui
                .set_reduced_motion(preferences.reduced_motion.unwrap_or(false));
        }
    }

    /// Lays out and records changed surfaces, asking for frames where pixels changed.
    pub fn refresh(&mut self) -> Result<()> {
        let now = self.clock.elapsed();
        let mut tooltips = Vec::new();
        for entry in &mut self.entries {
            entry.ui.advance_animations(now)?;
            // Pastes answered at once land before this refresh records them.
            crate::input::sync_requests(&mut self.platform, entry);
            if entry.ui.refresh()? {
                self.platform.request_redraw(entry.id)?;
            }
            crate::input::sync_primary(&mut self.platform, entry);
            fit(&mut self.platform, entry)?;
            if let Some(change) = aegle_widgets::take_tooltip(&entry.ui) {
                tooltips.push((entry.id, change));
            }
            // Regions are double-buffered: the next frame's commit carries them.
            if entry
                .shape
                .sync(&mut self.platform, entry.id, &entry.ui.root())?
            {
                self.platform.request_redraw(entry.id)?;
            }
            let cursor = entry.ui.cursor();
            if cursor != entry.cursor {
                self.platform.set_cursor(entry.id, cursor)?;
                entry.cursor = cursor;
            }
        }
        for (id, tooltip) in tooltips {
            self.show_tooltip(id, tooltip)?;
        }
        Ok(())
    }

    /// Replaces the tooltip popup of surface `id`: `None` only hides it.
    fn show_tooltip(&mut self, id: WindowId, tooltip: Option<HostTooltip>) -> Result<()> {
        let shown = self.entries.iter_mut().find(|e| e.id == id);
        if let Some(old) = shown.and_then(|e| e.tooltip.take())
            && self.entry(old).is_some()
        {
            self.close(old)?;
        }
        let Some(tooltip) = tooltip else {
            return Ok(());
        };
        let mut anchor = tooltip.anchor;
        anchor.size.width = anchor.size.width.max(1.0);
        anchor.size.height = anchor.size.height.max(1.0);
        // Wide enough to wrap only long text; then fitted to it.
        let size = PixelSize {
            width: 320,
            height: 1,
        };
        let (popup, ui) = self.open(
            WindowOptions {
                title: "",
                size,
                popup: Some(aegle_platform::PopupOptions {
                    parent: id,
                    anchor,
                    side: aegle_platform::Side::Below,
                    grab: None,
                }),
                transparent: true,
                ..Default::default()
            },
            size,
        )?;
        let root = ui.root();
        root.set_background(Color::TRANSPARENT);
        root.set_padding(0.0);
        let bubble = root.column();
        bubble.bind_color(ColorSlot::Background, Theme::SURFACE);
        bubble.bind_color(ColorSlot::BorderColor, Theme::DIVIDER);
        bubble.set_border_width(1.0);
        bubble.set_radius(6.0);
        bubble.set_padding(aegle_ui::Insets::new(4.0, 8.0, 4.0, 8.0));
        aegle_widgets::Widgets::text(&bubble, &tooltip.text);
        let entry = self
            .entries
            .iter_mut()
            .find(|e| e.id == popup)
            .expect("just opened");
        fit_node(&bubble);
        entry.fit = Some((*bubble).clone());
        if let Some(parent) = self.entries.iter_mut().find(|e| e.id == id) {
            parent.tooltip = Some(popup);
        }
        Ok(())
    }

    /// Presents every surface the platform allowed a frame.
    pub fn present(&mut self) -> Result<()> {
        let Runtime {
            entries, platform, ..
        } = self;
        for entry in entries.iter_mut() {
            // A popup about to take its content's size draws at that size.
            if entry.fitting.is_some() || !std::mem::take(&mut entry.ready) {
                continue;
            }
            let id = entry.id;
            let scale = platform.window_info(id)?.scale;
            let (ui, renderer) = (
                &entry.ui,
                entry
                    .renderer
                    .as_mut()
                    .expect("open surfaces have presenters"),
            );
            let presented = platform.present_external(id, |size| -> Result<bool> {
                let mut frame =
                    match renderer.begin_frame(size.width, size.height, Color::TRANSPARENT) {
                        Ok(Some(frame)) => frame,
                        Ok(None) | Err(aegle_render_vulkan::Error::SurfaceOutOfDate) => {
                            return Ok(false);
                        }
                        Err(error) => return Err(error.into()),
                    };
                if let Some(logical) = ui.damage() {
                    let [width, height] = frame.extent();
                    let mut pixels = aegle_types::Region::default();
                    for &rect in logical.rects() {
                        pixels.add(aegle_types::PixelRect::covering(rect, scale, width, height));
                    }
                    frame.set_damage(pixels.rects());
                }
                draw(ui, scale, &mut frame)?;
                match frame.finish() {
                    Ok(()) => Ok(true),
                    Err(aegle_render_vulkan::Error::SurfaceOutOfDate) => Ok(false),
                    Err(error) => Err(error.into()),
                }
            });
            let presented = match presented {
                Ok(presented) => {
                    entry.failed = false;
                    presented
                }
                // One surface's failed frame, such as an image beyond its
                // memory budget, must not end the shell: it frees the
                // surface's images and draws once more from nothing, then
                // waits for its next change.
                Err(aegle_platform::Error::Draw(error)) => {
                    eprintln!("palettes: drawing a surface: {error}");
                    renderer.release_images()?;
                    if !std::mem::replace(&mut entry.failed, true) {
                        platform.request_redraw(id)?;
                    }
                    continue;
                }
                Err(error) => return Err(error.into()),
            };
            if presented {
                entry.ui.clear_damage();
            } else {
                platform.request_redraw(id)?;
            }
            if entry.ui.has_animations() || entry.ui.wants_frames() {
                platform.request_redraw(id)?;
            }
        }
        Ok(())
    }
}

/// Lets `node` keep its content's size inside its surface's root, so the
/// surface can take that size.
pub(crate) fn fit_node(node: &Node) {
    node.set_grow(0.0);
    node.set_shrink(0.0);
    node.set_align_self(aegle_ui::Align::Start);
}

/// Asks for the size of the surface's fitted node once it is laid out.
fn fit(platform: &mut Wayland, entry: &mut Entry) -> Result<()> {
    let Some(node) = entry.fit.as_ref().filter(|n| n.is_alive()) else {
        return Ok(());
    };
    let info = platform.window_info(entry.id)?;
    if !info.configured {
        return Ok(());
    }
    let bounds = node.bounds();
    let wanted = PixelSize {
        width: (bounds.origin.x + bounds.size.width).ceil().max(1.0) as u32,
        height: (bounds.origin.y + bounds.size.height).ceil().max(1.0) as u32,
    };
    if wanted == info.size || entry.fitting.is_some() {
        return Ok(());
    }
    match platform.resize_popup(entry.id, wanted) {
        Ok(()) => entry.fitting = Some(wanted),
        // Too old a compositor: the popup keeps its first size.
        Err(aegle_platform::Error::Unsupported(_)) => entry.fit = None,
        Err(error) => return Err(error.into()),
    }
    Ok(())
}
