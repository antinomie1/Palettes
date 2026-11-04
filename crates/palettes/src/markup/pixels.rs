//! Pixel work on decoded images: shrinking, blurring and picking a palette.
//! Each works on straight RGBA and weighs color by alpha.

use aegle_ui::scene::Image;

/// `image` scaled down by the smallest whole factor that leaves at most
/// `max_pixels`, each pixel the alpha-weighted mean of the block it covers.
pub fn shrink(image: &Image, max_pixels: u32) -> Image {
    let ratio = f64::from(image.width()) * f64::from(image.height()) / f64::from(max_pixels.max(1));
    shrink_by(image, (ratio.sqrt().ceil() as u32).max(1))
}

/// `image` scaled down by `factor`, each pixel the alpha-weighted mean of
/// the block it covers.
fn shrink_by(image: &Image, factor: u32) -> Image {
    if factor <= 1 {
        return image.clone();
    }
    let (width, height) = (image.width(), image.height());
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

/// `image` blurred by about `radius` of its pixels, close to a Gaussian:
/// three box blurs each way. A blur needs no detail, so it runs on a copy
/// shrunk until the radius is a few pixels; the result is that small too
/// and is scaled up where it shows.
pub fn blur(image: &Image, radius: f32) -> Image {
    if radius.is_nan() || radius < 1.0 {
        return image.clone();
    }
    let factor = ((radius / 4.0) as u32).max(1);
    let small = shrink_by(image, factor);
    let (width, height) = (small.width() as usize, small.height() as usize);
    let reach = ((radius / factor as f32).round() as usize).max(1);
    // Premultiplied, so transparent pixels lend no color.
    let mut planes: Vec<[u32; 4]> = small
        .pixels()
        .as_chunks::<4>()
        .0
        .iter()
        .map(|p| {
            let a = u32::from(p[3]);
            [
                u32::from(p[0]) * a,
                u32::from(p[1]) * a,
                u32::from(p[2]) * a,
                a * 255,
            ]
        })
        .collect();
    let mut line = Vec::new();
    for _ in 0..3 {
        for y in 0..height {
            box_line(&mut planes, y * width, 1, width, reach, &mut line);
        }
        for x in 0..width {
            box_line(&mut planes, x, width, height, reach, &mut line);
        }
    }
    let pixels = planes
        .iter()
        .flat_map(|p| {
            let alpha = p[3] / 255;
            let color = |c: u32| c.checked_div(alpha).unwrap_or(0).min(255) as u8;
            [color(p[0]), color(p[1]), color(p[2]), alpha.min(255) as u8]
        })
        .collect();
    Image::new(width as u32, height as u32, pixels).expect("same size as a valid image")
}

/// Replaces `count` pixels from `start`, `stride` apart, by the mean of the
/// `2 * reach + 1` around each, edges repeated.
fn box_line(
    planes: &mut [[u32; 4]],
    start: usize,
    stride: usize,
    count: usize,
    reach: usize,
    line: &mut Vec<[u32; 4]>,
) {
    line.clear();
    line.extend((0..count).map(|i| planes[start + i * stride]));
    let at = |i: isize| line[i.clamp(0, count as isize - 1) as usize];
    let span = (2 * reach + 1) as u64;
    let mut sum = [0u64; 4];
    for i in -(reach as isize)..=reach as isize {
        for (s, v) in sum.iter_mut().zip(at(i)) {
            *s += u64::from(v);
        }
    }
    for i in 0..count {
        let out = &mut planes[start + i * stride];
        for channel in 0..4 {
            out[channel] = (sum[channel] / span) as u32;
        }
        let (enter, leave) = (
            at((i + reach + 1) as isize),
            at(i as isize - reach as isize),
        );
        for channel in 0..4 {
            sum[channel] = sum[channel] + u64::from(enter[channel]) - u64::from(leave[channel]);
        }
    }
}

/// Up to `count` colors that cover most of `image`, the most common first,
/// each clearly different from those before it. Transparent pixels count
/// for nothing.
pub fn palette(image: &Image, count: usize) -> Vec<[u8; 3]> {
    let small = shrink(image, 4096);
    // Colors to 4 bits a channel: sums of what fell in each bin, and weight.
    let mut bins = vec![[0u64; 4]; 4096];
    for p in small.pixels().as_chunks::<4>().0.iter() {
        let weight = u64::from(p[3]);
        let bin = &mut bins[(usize::from(p[0] >> 4) << 8)
            | (usize::from(p[1] >> 4) << 4)
            | usize::from(p[2] >> 4)];
        for channel in 0..3 {
            bin[channel] += u64::from(p[channel]) * weight;
        }
        bin[3] += weight;
    }
    let mut colors: Vec<(u64, [u8; 3])> = bins
        .iter()
        .filter(|b| b[3] > 0)
        .map(|b| (b[3], [0, 1, 2].map(|c| (b[c] / b[3]) as u8)))
        .collect();
    colors.sort_by_key(|c| std::cmp::Reverse(c.0));
    let distance = |a: [u8; 3], b: [u8; 3]| -> u32 {
        (0..3)
            .map(|c| (i32::from(a[c]) - i32::from(b[c])).pow(2) as u32)
            .sum()
    };
    let mut picked: Vec<[u8; 3]> = Vec::new();
    for (_, color) in colors {
        if picked.len() == count {
            break;
        }
        if picked.iter().all(|&p| distance(p, color) > 48 * 48) {
            picked.push(color);
        }
    }
    picked
}
