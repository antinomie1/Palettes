//! Image files for `Image` and `Icon`: bitmaps, SVGs rasterized at the
//! pixel size they show at on the densest output, and bitmaps larger than
//! [`palettes_shell::MAX_IMAGE_PIXELS`] shrunk once, since a surface could
//! not upload them. An `Image` may be blurred, and may give the shell its
//! palette. A view shows its file again when the file changes.

use super::follow::Followers;
pub use super::pixels::{blur, palette, shrink};
use aegle_ui::{Color, Node, NodeId, Theme, Token};
use aegle_widgets::ImageView;
use palettes_shell::Shell;
use std::{cell::Cell, cell::RefCell, collections::HashMap, path::Path};

/// How a view shows its file.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Look {
    /// The square size SVGs are rasterized at.
    pub pixels: u32,
    /// Blur radius in the image's pixels.
    pub blur: f32,
    /// Its colors become the shell's palette.
    pub palette: bool,
}

impl Default for Look {
    fn default() -> Self {
        Self {
            pixels: IMAGE_PIXELS,
            blur: 0.0,
            palette: false,
        }
    }
}

/// Follows the palette for its owner.
type Follower = (Node, Box<dyn FnMut(&[String])>);

thread_local! {
    static SHOWN: Followers<ImageView> = const { Followers::new() };
    static LOOKS: RefCell<HashMap<NodeId, (ImageView, Look)>> = RefCell::default();
    /// The largest output scale.
    static SCALE: Cell<f32> = const { Cell::new(1.0) };
    /// The shell whose surfaces take the palette's tokens.
    static SHELL: RefCell<Option<Shell>> = const { RefCell::new(None) };
    /// The latest palette, as `#rrggbb`, and who follows it.
    static PALETTE: RefCell<(Vec<String>, Vec<Follower>)> = RefCell::default();
}

/// Pixels SVGs in an `Image` are rasterized at.
const IMAGE_PIXELS: u32 = 64;

/// Colors in the palette: tokens `palette.1` to `palette.5`.
pub const PALETTE_SIZE: usize = 5;

/// The palette's tokens, which follow the theme's accent until an image
/// gives them colors.
fn tokens() -> aegle_ui::Result<Vec<Token<Color>>> {
    (1..=PALETTE_SIZE)
        .map(|n| aegle_ui::register_token(&format!("palette.{n}"), |t: &Theme| t.accent))
        .collect()
}

/// Registers the palette's tokens, and gives `shell` the palette.
pub(crate) fn start(shell: Option<&Shell>) -> aegle_ui::Result<()> {
    tokens()?;
    SHELL.with_borrow_mut(|slot| *slot = shell.cloned());
    Ok(())
}

/// Follows the palette while `owner` lives.
pub(crate) fn follow_palette(owner: &Node, mut f: impl FnMut(&[String]) + 'static) {
    PALETTE.with_borrow_mut(|(latest, followers)| {
        if !latest.is_empty() {
            f(latest);
        }
        followers.retain(|(node, _)| node.is_alive());
        followers.push((owner.clone(), Box::new(f)));
    });
}

/// Makes `image`'s colors the palette, if they changed.
fn publish(image: &aegle_ui::scene::Image) -> aegle_ui::Result<()> {
    let colors = palette(image, PALETTE_SIZE);
    let Some(&first) = colors.first() else {
        return Ok(());
    };
    let hex: Vec<String> = (colors.iter())
        .map(|[r, g, b]| format!("#{r:02x}{g:02x}{b:02x}"))
        .collect();
    let mut followers = PALETTE.with_borrow_mut(|(latest, followers)| {
        if *latest == hex {
            return None;
        }
        *latest = hex.clone();
        Some(std::mem::take(followers))
    });
    let Some(followers) = followers.as_mut() else {
        return Ok(());
    };
    if let Some(shell) = SHELL.with_borrow(Clone::clone) {
        for (n, token) in tokens()?.into_iter().enumerate() {
            let [r, g, b] = colors.get(n).copied().unwrap_or(first);
            shell.set_color_token(token, Some(Color::rgb(r, g, b)));
        }
    }
    followers.retain_mut(|(owner, follow)| {
        let alive = owner.is_alive();
        if alive {
            follow(&hex);
        }
        alive
    });
    PALETTE.with_borrow_mut(|(_, kept)| {
        followers.append(kept);
        *kept = std::mem::take(followers);
    });
    Ok(())
}

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
    let path = SHOWN.with(|shown| shown.follow(view, path));
    load(view, &path, look(view))
}

/// [`show`], SVGs rasterized `pixels` square.
pub(crate) fn show_at(view: &ImageView, path: &Path, pixels: u32) -> aegle_ui::Result<()> {
    restyle(view, |look| look.pixels = pixels, false);
    show(view, path)
}

fn look(view: &ImageView) -> Look {
    LOOKS.with_borrow(|looks| looks.get(&view.id()).map(|l| l.1).unwrap_or_default())
}

/// Changes how `view` shows its file, showing it again if `reload`.
pub(crate) fn restyle(view: &ImageView, change: impl FnOnce(&mut Look), reload: bool) {
    let before = look(view);
    let mut after = before;
    change(&mut after);
    LOOKS.with_borrow_mut(|looks| {
        looks.retain(|_, (v, _)| v.is_alive());
        looks.insert(view.id(), (view.clone(), after))
    });
    if !reload || after == before {
        return;
    }
    // What is shown serves when it can, rather than reading the file again:
    // properties are set one by one as a document builds.
    let Some(shown) = view.image() else {
        // Nothing loaded: no file yet, or one that failed.
        return;
    };
    let result = match Some(shown) {
        Some(image) if after.pixels == before.pixels && after.blur == before.blur => {
            if after.palette {
                publish(&image)
            } else {
                Ok(())
            }
        }
        Some(image) if after.pixels == before.pixels && before.blur < 1.0 => {
            let image = blur(&image, after.blur);
            view.set_image(&image);
            if after.palette {
                publish(&image)
            } else {
                Ok(())
            }
        }
        _ => match SHOWN.with(|shown| shown.path_of(view)) {
            Some(path) => load(view, &path, after),
            None => Ok(()),
        },
    };
    if let Err(e) = result {
        eprintln!("palettes: image: {e}");
    }
}

/// Empties `view` and stops following its file.
pub(crate) fn clear(view: &ImageView) {
    SHOWN.with(|shown| shown.forget(view));
    view.clear();
}

/// Loads `path` again into every view showing it.
pub(crate) fn changed(path: &Path) {
    for view in SHOWN.with(|shown| shown.of(path)) {
        if let Err(e) = load(&view, path, look(&view)) {
            eprintln!("palettes: image {}: {e}", path.display());
        }
    }
}

/// Loads a bitmap, or rasterizes an SVG, into `view` as `look` says.
/// Failures reach the view's error handlers.
fn load(view: &ImageView, path: &Path, look: Look) -> aegle_ui::Result<()> {
    #[cfg(feature = "svg")]
    if path.extension().is_some_and(|e| e == "svg") {
        let bytes = std::fs::read(path)?;
        let image = aegle_image::svg::rasterize(&bytes, look.pixels, look.pixels)?;
        view.set_image(&image);
        return Ok(());
    }
    view.load(path)?;
    let Some(mut image) = view.image() else {
        return Ok(());
    };
    if look.palette {
        publish(&image)?;
    }
    let max = palettes_shell::MAX_IMAGE_PIXELS;
    let large = image.width() * image.height() > max;
    if large {
        image = shrink(&image, max);
    }
    if look.blur >= 1.0 {
        image = blur(&image, look.blur);
    }
    if large || look.blur >= 1.0 {
        view.set_image(&image);
    }
    Ok(())
}
