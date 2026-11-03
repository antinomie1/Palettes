//! Input and blur regions: rounded rectangles as the whole-pixel
//! rectangles a wl_region takes.

use aegle_platform::{PixelRect, Rect};
use palettes_wayland::rounded_rects;

fn covered(rects: &[PixelRect], x: u32, y: u32) -> bool {
    rects
        .iter()
        .any(|r| (r.x..r.x + r.width).contains(&x) && (r.y..r.y + r.height).contains(&y))
}

#[test]
fn a_square_corner_is_one_rectangle() {
    let mut rects = Vec::new();
    rounded_rects(Rect::new(4.0, 2.0, 100.0, 30.0), 0.0, &mut rects);
    assert_eq!(
        rects,
        [PixelRect {
            x: 4,
            y: 2,
            width: 100,
            height: 30
        }]
    );
}

#[test]
fn rounded_corners_leave_their_corners_out() {
    let mut rects = Vec::new();
    rounded_rects(Rect::new(0.0, 0.0, 200.0, 40.0), 12.0, &mut rects);
    assert!(
        !covered(&rects, 0, 0) && !covered(&rects, 199, 39),
        "corners"
    );
    assert!(
        !covered(&rects, 2, 2),
        "inside the corner's square, outside its arc"
    );
    assert!(covered(&rects, 6, 6) && covered(&rects, 0, 20) && covered(&rects, 100, 0));
    let area: u32 = rects.iter().map(|r| r.width * r.height).sum();
    let exact = 200.0 * 40.0 - (4.0 - std::f32::consts::PI) * 144.0;
    assert!((area as f32 - exact).abs() < 40.0, "{area} vs {exact}");
    // Rectangles never overlap, so the region is exact.
    let cells: u32 = (0..200)
        .flat_map(|x| (0..40).map(move |y| (x, y)))
        .filter(|&(x, y)| covered(&rects, x, y))
        .count() as u32;
    assert_eq!(cells, area);
}

#[test]
fn a_pill_rounds_its_whole_ends_and_nothing_is_empty() {
    let mut rects = Vec::new();
    rounded_rects(Rect::new(10.0, 10.0, 80.0, 24.0), 100.0, &mut rects);
    assert!(rects.iter().all(|r| r.width > 0 && r.height > 0));
    assert!(covered(&rects, 10, 22) && !covered(&rects, 10, 10));
    rects.clear();
    rounded_rects(Rect::new(0.0, 0.0, 0.0, 24.0), 4.0, &mut rects);
    assert!(rects.is_empty());
}
