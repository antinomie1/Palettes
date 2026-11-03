//! Image files for `Image` and `Icon`: bitmaps, SVGs rasterized at icon
//! size, and bitmaps larger than [`palettes_shell::MAX_IMAGE_PIXELS`]
//! shrunk once, since a surface could not upload them. A view shows its file
//! again when the file changes.

use super::follow::Followers;
use aegle_ui::scene::Image;
use aegle_widgets::ImageView;
use std::path::Path;

thread_local! {
    static SHOWN: Followers<ImageView> = const { Followers::new() };
}

/// Shows the file at `path` in `view` and keeps it shown when the file
/// changes. Failures reach the view's error handlers.
pub(crate) fn show(view: &ImageView, path: &Path) -> aegle_ui::Result<()> {
    let path = SHOWN.with(|shown| shown.follow(view, path));
    load(view, &path)
}

/// Empties `view` and stops following its file.
pub(crate) fn clear(view: &ImageView) {
    SHOWN.with(|shown| shown.forget(view));
    view.clear();
}

/// Loads `path` again into every view showing it.
pub(crate) fn changed(path: &Path) {
    for view in SHOWN.with(|shown| shown.of(path)) {
        if let Err(e) = load(&view, path) {
            eprintln!("palettes: image {}: {e}", path.display());
        }
    }
}

/// Icon size in pixels SVG icons are rasterized at: crisp up to 32 dp at 2×.
#[cfg(feature = "svg")]
const ICON_PIXELS: u32 = 64;

/// Loads a bitmap, or rasterizes an SVG, into `view`. Failures reach the
/// view's error handlers.
fn load(view: &ImageView, path: &Path) -> aegle_ui::Result<()> {
    #[cfg(feature = "svg")]
    if path.extension().is_some_and(|e| e == "svg") {
        let bytes = std::fs::read(path)?;
        let image = aegle_image::svg::rasterize(&bytes, ICON_PIXELS, ICON_PIXELS)?;
        view.set_image(&image);
        return Ok(());
    }
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
