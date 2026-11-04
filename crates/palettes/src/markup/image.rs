//! Image files for `Image` and `Icon`: bitmaps, SVGs rasterized at the
//! pixel size they show at on the densest output, and bitmaps larger than
//! [`palettes_shell::MAX_IMAGE_PIXELS`] shrunk once, since a surface could
//! not upload them. A view shows its file again when the file changes.

use super::follow::Followers;
use aegle_ui::{NodeId, scene::Image};
use aegle_widgets::ImageView;
use std::{cell::Cell, cell::RefCell, collections::HashMap, path::Path};

thread_local! {
    static SHOWN: Followers<ImageView> = const { Followers::new() };
    /// The square pixel size each view rasterizes SVGs at.
    static PIXELS: RefCell<HashMap<NodeId, u32>> = RefCell::default();
    /// The largest output scale.
    static SCALE: Cell<f32> = const { Cell::new(1.0) };
}

/// Pixels SVGs in an `Image` are rasterized at.
const IMAGE_PIXELS: u32 = 64;

/// The largest output scale, which icons are rasterized for.
pub(crate) fn scale() -> f32 {
    SCALE.get()
}

/// Remembers the largest output scale; true when it changed.
pub(crate) fn set_scale(scale: f32) -> bool {
    let scale = if scale.is_finite() {
        scale.max(1.0)
    } else {
        1.0
    };
    SCALE.replace(scale) != scale
}

/// Device pixels of `dp` logical pixels on the densest output.
pub(crate) fn pixels(dp: f32) -> u32 {
    (dp * scale()).ceil().clamp(1.0, 1024.0) as u32
}

/// Shows the file at `path` in `view` and keeps it shown when the file
/// changes. Failures reach the view's error handlers.
pub(crate) fn show(view: &ImageView, path: &Path) -> aegle_ui::Result<()> {
    show_at(view, path, IMAGE_PIXELS)
}

/// [`show`], an SVG rasterized `pixels` square.
pub(crate) fn show_at(view: &ImageView, path: &Path, pixels: u32) -> aegle_ui::Result<()> {
    let path = SHOWN.with(|shown| shown.follow(view, path));
    PIXELS.with_borrow_mut(|sizes| sizes.insert(view.id(), pixels));
    load(view, &path, pixels)
}

/// Empties `view` and stops following its file.
pub(crate) fn clear(view: &ImageView) {
    SHOWN.with(|shown| shown.forget(view));
    PIXELS.with_borrow_mut(|sizes| sizes.remove(&view.id()));
    view.clear();
}

/// Loads `path` again into every view showing it.
pub(crate) fn changed(path: &Path) {
    for view in SHOWN.with(|shown| shown.of(path)) {
        let pixels = PIXELS.with_borrow(|sizes| sizes.get(&view.id()).copied());
        if let Err(e) = load(&view, path, pixels.unwrap_or(IMAGE_PIXELS)) {
            eprintln!("palettes: image {}: {e}", path.display());
        }
    }
}

/// Loads a bitmap, or rasterizes an SVG `pixels` square, into `view`.
/// Failures reach the view's error handlers.
fn load(view: &ImageView, path: &Path, pixels: u32) -> aegle_ui::Result<()> {
    #[cfg(feature = "svg")]
    if path.extension().is_some_and(|e| e == "svg") {
        let bytes = std::fs::read(path)?;
        let image = aegle_image::svg::rasterize(&bytes, pixels, pixels)?;
        view.set_image(&image);
        return Ok(());
    }
    #[cfg(not(feature = "svg"))]
    let _ = pixels;
    view.load(path)?;
    let max = palettes_shell::MAX_IMAGE_PIXELS;
    if let Some(image) = view.image().filter(|i| i.width() * i.height() > max) {
        view.set_image(&shrink(&image, max));
    }
    Ok(())
}

/// `image` scaled down by the smallest whole factor that leaves at most
/// `max_pixels`, each pixel the alpha-weighted mean of the block it covers.
pub fn shrink(image: &Image, max_pixels: u32) -> Image {
    let (width, height) = (image.width(), image.height());
    let ratio = f64::from(width) * f64::from(height) / f64::from(max_pixels.max(1));
    let factor = (ratio.sqrt().ceil() as u32).max(1);
    if factor == 1 {
        return image.clone();
    }
    let (out_width, out_height) = (width.div_ceil(factor), height.div_ceil(factor));
    let source = image.pixels();
    let mut pixels = Vec::with_capacity((out_width * out_height * 4) as usize);
    for out_y in 0..out_height {
        for out_x in 0..out_width {
            let mut sum = [0u64; 4];
            let mut count = 0u64;
            for y in out_y * factor..((out_y + 1) * factor).min(height) {
                for x in out_x * factor..((out_x + 1) * factor).min(width) {
                    let at = ((y * width + x) * 4) as usize;
                    let alpha = u64::from(source[at + 3]);
                    for channel in 0..3 {
                        sum[channel] += u64::from(source[at + channel]) * alpha;
                    }
                    sum[3] += alpha;
                    count += 1;
                }
            }
            for channel in 0..3 {
                pixels.push(sum[channel].checked_div(sum[3]).unwrap_or(0) as u8);
            }
            pixels.push((sum[3] / count) as u8);
        }
    }
    Image::new(out_width, out_height, pixels).expect("smaller than a valid image")
}
