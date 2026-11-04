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
    Color, ColorSlot, Cursor, Modifiers, Rect, Result, Size, TextSystem, Theme, Ui, Visit,
};
use palettes_wayland::{Wayland, WindowSurface};
use std::{cell::RefCell, rc::Rc, time::Instant};

/// Pixels of the largest image a surface uploads: 4 MiB of RGBA, which the
/// default memory budget holds beside the surface's text.
pub const MAX_IMAGE_PIXELS: u32 = 1024 * 1024;

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
        let id = self.platform.create_window(options)?;
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
            parent: options.popup.map(|p| p.parent),
            shape: Shape::default(),
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
        for entry in &mut self.entries {
            entry.ui.advance_animations(now)?;
            // Pastes answered at once land before this refresh records them.
            crate::input::sync_text(&mut self.platform, entry);
            // No drag and drop: drop requests rather than queue them.
            let _ = entry.ui.take_drag();
            if entry.ui.refresh()? {
                self.platform.request_redraw(entry.id)?;
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
        Ok(())
    }

    /// Presents every surface the platform allowed a frame.
    pub fn present(&mut self) -> Result<()> {
        let Runtime {
            entries, platform, ..
        } = self;
        for entry in entries.iter_mut() {
            if !std::mem::take(&mut entry.ready) {
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
            })?;
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
