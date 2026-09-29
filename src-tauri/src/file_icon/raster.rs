//! RGBA ↔ PNG. Cached icons are one 96×96 PNG, scaled in the webview.

use std::io::Cursor;

pub const ICON_PX: u32 = 96;

pub fn encode(width: u32, height: u32, rgba: &[u8]) -> Option<Vec<u8>> {
    if width == 0 || height == 0 || rgba.len() < (width as usize) * (height as usize) * 4 {
        return None;
    }
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().ok()?;
    writer.write_image_data(rgba).ok()?;
    drop(writer);
    Some(out)
}

/// Square 96×96 PNG. Smaller sources are centered; larger ones are box-filtered down.
pub fn standardize(width: u32, height: u32, rgba: &[u8]) -> Option<Vec<u8>> {
    if width == ICON_PX && height == ICON_PX {
        return encode(width, height, rgba);
    }
    let fitted = fit(width, height, rgba)?;
    encode(ICON_PX, ICON_PX, &fitted)
}

pub fn is_png(bytes: &[u8]) -> bool {
    if bytes.len() < 32 || !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return false;
    }
    let decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.read_info().is_ok()
}

/// Keep a PNG that is already ≤ 96px. Larger PNGs are decoded and refit.
pub fn standardize_png(bytes: &[u8]) -> Option<Vec<u8>> {
    let (w, h, rgba) = decode_rgba(bytes)?;
    if w <= ICON_PX && h <= ICON_PX && w == h {
        return Some(bytes.to_vec());
    }
    standardize(w, h, &rgba)
}

pub fn decode_rgba(bytes: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0u8; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).ok()?;
    let pixels = &buf[..info.buffer_size()];
    let rgba = to_rgba(info.width, info.height, info.color_type, pixels)?;
    Some((info.width, info.height, rgba))
}

fn to_rgba(width: u32, height: u32, color: png::ColorType, pixels: &[u8]) -> Option<Vec<u8>> {
    let n = (width as usize).checked_mul(height as usize)?;
    let mut out = Vec::with_capacity(n * 4);
    match color {
        png::ColorType::Rgba => {
            if pixels.len() < n * 4 {
                return None;
            }
            out.extend_from_slice(&pixels[..n * 4]);
        }
        png::ColorType::Rgb => {
            if pixels.len() < n * 3 {
                return None;
            }
            for px in pixels[..n * 3].chunks_exact(3) {
                out.extend_from_slice(&[px[0], px[1], px[2], 255]);
            }
        }
        png::ColorType::GrayscaleAlpha => {
            if pixels.len() < n * 2 {
                return None;
            }
            for px in pixels[..n * 2].chunks_exact(2) {
                out.extend_from_slice(&[px[0], px[0], px[0], px[1]]);
            }
        }
        png::ColorType::Grayscale => {
            if pixels.len() < n {
                return None;
            }
            for &g in &pixels[..n] {
                out.extend_from_slice(&[g, g, g, 255]);
            }
        }
        _ => return None,
    }
    Some(out)
}

fn fit(width: u32, height: u32, rgba: &[u8]) -> Option<Vec<u8>> {
    let sw = width as usize;
    let sh = height as usize;
    if rgba.len() < sw * sh * 4 || sw == 0 || sh == 0 {
        return None;
    }
    let side = ICON_PX as usize;
    let mut out = vec![0u8; side * side * 4];
    // Source covers the destination square; leftover source is cropped from the center
    // only when the aspect is extreme. Most system icons are already square.
    for y in 0..side {
        for x in 0..side {
            let sx0 = x * sw / side;
            let sx1 = ((x + 1) * sw / side).max(sx0 + 1).min(sw);
            let sy0 = y * sh / side;
            let sy1 = ((y + 1) * sh / side).max(sy0 + 1).min(sh);
            let mut acc = [0u32; 4];
            let mut count = 0u32;
            for sy in sy0..sy1 {
                for sx in sx0..sx1 {
                    let i = (sy * sw + sx) * 4;
                    for c in 0..4 {
                        acc[c] += rgba[i + c] as u32;
                    }
                    count += 1;
                }
            }
            if count == 0 {
                continue;
            }
            let o = (y * side + x) * 4;
            for c in 0..4 {
                out[o + c] = (acc[c] / count) as u8;
            }
        }
    }
    Some(out)
}

/// BGRA from GDI, with alpha forced on when the icon has no alpha plane.
#[cfg(target_os = "windows")]
pub fn bgra_to_rgba(pixels: &mut [u8]) {
    let mut any_alpha = false;
    for px in pixels.chunks_exact_mut(4) {
        px.swap(0, 2);
        if px[3] != 0 {
            any_alpha = true;
        }
    }
    if !any_alpha {
        for px in pixels.chunks_exact_mut(4) {
            px[3] = 255;
        }
    }
}
