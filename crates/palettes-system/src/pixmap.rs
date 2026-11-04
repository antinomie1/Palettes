//! Images that arrive as pixels (tray icons, notification images) written as
//! PNG files, since records carry paths.

use std::{io, path::PathBuf};

/// Where such files go: `$XDG_RUNTIME_DIR/palettes`, created on first use.
pub(crate) fn cache_dir() -> PathBuf {
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    base.join("palettes")
}

/// A file name part made of `key`'s letters and digits.
pub(crate) fn file_key(key: &str) -> String {
    key.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

/// Writes straight RGBA rows as `name.png` in the cache directory, leaving
/// an identical file alone so nothing showing it reloads. Returns its path.
pub(crate) fn write_rgba(name: &str, width: u32, height: u32, rgba: &[u8]) -> io::Result<PathBuf> {
    if width == 0 || height == 0 || rgba.len() != (width * height * 4) as usize {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "bad pixmap size",
        ));
    }
    let mut bytes = Vec::new();
    let mut encoder = png::Encoder::new(&mut bytes, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(io::Error::other)?;
    writer.write_image_data(rgba).map_err(io::Error::other)?;
    writer.finish().map_err(io::Error::other)?;
    write_file(&format!("{name}.png"), &bytes)
}

/// Writes `bytes` as `name` in the cache directory unless it already holds
/// them; the file is replaced whole so a reader never sees half of it.
pub(crate) fn write_file(name: &str, bytes: &[u8]) -> io::Result<PathBuf> {
    let dir = cache_dir();
    let path = dir.join(name);
    if std::fs::read(&path).is_ok_and(|old| old == bytes) {
        return Ok(path);
    }
    std::fs::create_dir_all(&dir)?;
    let partial = dir.join(format!(".{name}.part"));
    std::fs::write(&partial, bytes)?;
    std::fs::rename(&partial, &path)?;
    Ok(path)
}

/// RGBA from ARGB32 in network byte order, as status items send icons.
pub(crate) fn argb_to_rgba(argb: &[u8]) -> Vec<u8> {
    (argb.as_chunks::<4>().0.iter())
        .flat_map(|p| [p[1], p[2], p[3], p[0]])
        .collect()
}

/// A local path from a path or `file://` URI, percent escapes decoded;
/// `None` for other URIs.
pub fn local_path(text: &str) -> Option<String> {
    if text.starts_with('/') {
        return Some(text.to_owned());
    }
    let rest = text.strip_prefix("file://")?;
    let rest = rest.find('/').map(|at| &rest[at..])?;
    let bytes = rest.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        if bytes[i] == b'%'
            && let (Some(&h), Some(&l)) = (bytes.get(i + 1), bytes.get(i + 2))
            && let (Some(h), Some(l)) = (hex(h), hex(l))
        {
            out.push((h * 16 + l) as u8);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}
