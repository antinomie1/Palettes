//! Bitmaps too large for a surface's memory budget shrink once; blurs and
//! palettes come from the pixels.

use aegle_ui::scene::Image;
use palettes::markup::image::{blur, palette, parse_color, shrink};
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
    assert_eq!(
        shrink(&wallpaper, MAX_IMAGE_PIXELS).width(),
        3840,
        "a 4K wallpaper fits"
    );
    let eight = Image::new(7680, 4320, vec![0; 7680 * 4320 * 4]).unwrap();
    let fitted = shrink(&eight, MAX_IMAGE_PIXELS);
    assert!(fitted.width() * fitted.height() <= MAX_IMAGE_PIXELS);
    assert_eq!((fitted.width(), fitted.height()), (3840, 2160));
}

/// A `width` × 1 opaque image, black on the left half and white on the right.
fn edge(width: u32) -> Image {
    let pixels = (0..width)
        .flat_map(|x| {
            if x < width / 2 {
                [0, 0, 0, 255]
            } else {
                [255; 4]
            }
        })
        .collect();
    Image::new(width, 1, pixels).unwrap()
}

#[test]
fn blurring_softens_edges_on_a_smaller_copy() {
    let sharp = edge(64);
    let soft = blur(&sharp, 3.0);
    assert_eq!(soft.width(), 64, "small radii keep every pixel");
    let gray = |image: &Image, x: u32| image.pixels()[(x * 4) as usize];
    assert_eq!(gray(&soft, 0), 0);
    assert_eq!(gray(&soft, 63), 255);
    let middle = gray(&soft, 31);
    assert!(middle > 40 && middle < 215, "the edge is soft: {middle}");
    assert!(
        soft.pixels().as_chunks::<4>().0.iter().all(|p| p[3] == 255),
        "still opaque"
    );
    let wide = blur(&edge(400), 40.0);
    assert_eq!(wide.width(), 40, "a large radius runs on a tenth");
    assert_eq!(
        blur(&sharp, 0.0).pixels(),
        sharp.pixels(),
        "no radius, no blur"
    );
}

#[test]
fn palettes_list_distinct_colors_most_common_first() {
    let pixels: Vec<u8> = (0..100)
        .flat_map(|i| match i {
            0..60 => [200, 30, 30, 255],
            60..62 => [205, 32, 28, 255],
            62..90 => [20, 40, 200, 255],
            _ => [0, 255, 0, 0],
        })
        .collect();
    let image = Image::new(10, 10, pixels).unwrap();
    assert_eq!(palette(&image, 5), vec![[200, 30, 30], [20, 40, 200]]);
    assert_eq!(palette(&image, 1), vec![[200, 30, 30]]);
}

#[test]
fn script_colors_parse_with_or_without_alpha() {
    assert_eq!(parse_color("#1e2028"), Some([0x1e, 0x20, 0x28, 255]));
    assert_eq!(parse_color("#a6c8ff80"), Some([0xa6, 0xc8, 0xff, 0x80]));
    for bad in ["1e2028", "#fff", "#1e20", "#1e2028a", "#gggggg", "#ééé"] {
        assert_eq!(parse_color(bad), None, "{bad}");
    }
}
