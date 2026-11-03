//! Bitmaps too large for a surface's memory budget shrink once.

use aegle_ui::scene::Image;
use palettes::markup::image::shrink;
use palettes_shell::MAX_IMAGE_PIXELS;

#[test]
fn shrinking_averages_blocks_weighted_by_alpha() {
    // A 3 × 2 image: an opaque red and a transparent green pixel side by side,
    // then blue, then a column of opaque white.
    let pixels = [
        [255, 0, 0, 255],
        [0, 255, 0, 0],
        [255, 255, 255, 255],
        [0, 0, 255, 255],
        [0, 0, 255, 255],
        [255, 255, 255, 255],
    ];
    let image = Image::new(3, 2, pixels.concat()).unwrap();
    let small = shrink(&image, 2);
    // Factor 2: blocks of 2 × 2 and a 1 × 2 edge column.
    assert_eq!((small.width(), small.height()), (2, 1));
    // The transparent pixel adds no color, only coverage.
    assert_eq!(&small.pixels()[..4], &[85, 0, 170, 191]);
    assert_eq!(&small.pixels()[4..], &[255, 255, 255, 255]);
    assert!(shrink(&image, 6).width() == 3, "small images stay");
    let wallpaper = Image::new(3840, 2160, vec![0; 3840 * 2160 * 4]).unwrap();
    let fitted = shrink(&wallpaper, MAX_IMAGE_PIXELS);
    assert!(fitted.width() * fitted.height() <= MAX_IMAGE_PIXELS);
    assert_eq!((fitted.width(), fitted.height()), (1280, 720));
}
