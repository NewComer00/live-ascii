//! Sixel encode helpers: quantize at reduced resolution, upsample, emit at display size.

use icy_sixel::{BackgroundMode, EncodeOptions, PixelAspectRatio, SixelImage};
use quantette::deps::palette::Srgb;
use quantette::dither::FloydSteinberg;
use quantette::{ImageRef, PaletteSize, Pipeline};

struct Rgb {
    r: u8,
    g: u8,
    b: u8,
}

/// Encode RGBA at `quant_w`×`quant_h`, upsample to `display_w`×`display_h`, pad to `encode_h`.
pub fn encode_rgba_at_display_size(
    rgba: Vec<u8>,
    quant_w: usize,
    quant_h: usize,
    display_w: usize,
    display_h: usize,
    encode_h: usize,
    opts: &EncodeOptions,
) -> Option<Vec<u8>> {
    if quant_w == display_w && quant_h == display_h {
        let rgba = pad_rgba_bottom(rgba, display_w, display_h, encode_h);
        return SixelImage::try_from_rgba(rgba, display_w, encode_h)
            .ok()
            .map(|image| {
                image
                    .with_background_mode(BackgroundMode::Opaque)
                    .encode_with(opts)
            })
            .and_then(|r| r.ok())
            .map(|s| inject_raster_attributes(s.into_bytes(), display_w, encode_h));
    }

    let opacity_mask: Vec<bool> = rgba.chunks_exact(4).map(|c| c[3] >= 128).collect();
    let rgb_pixels: Vec<Srgb<u8>> = rgba
        .chunks_exact(4)
        .map(|c| Srgb::new(c[0], c[1], c[2]))
        .collect();

    let max_colors = opts.max_colors.clamp(2, 256) as u8;
    let palette_size = PaletteSize::try_from(max_colors).unwrap_or(PaletteSize::MAX);
    let image = ImageRef::new(quant_w as u32, quant_h as u32, &rgb_pixels).ok()?;
    let diffusion = opts.diffusion.clamp(0.0, 1.0);
    let pipeline = Pipeline::new()
        .palette_size(palette_size)
        .quantize_method(opts.quantize_method.clone());

    let indexed = if diffusion <= 0.0 {
        pipeline
            .ditherer(None)
            .input_image(image)
            .output_srgb8_indexed_image()
    } else {
        let ditherer = FloydSteinberg::with_error_diffusion(diffusion).unwrap_or_default();
        pipeline
            .ditherer(ditherer)
            .input_image(image)
            .output_srgb8_indexed_image()
    };

    let palette: Vec<Rgb> = indexed
        .palette()
        .iter()
        .map(|c| Rgb {
            r: c.red,
            g: c.green,
            b: c.blue,
        })
        .collect();
    let indices = indexed.indices();
    let (up_indices, up_opacity) =
        upsample_indexed(indices, &opacity_mask, quant_w, quant_h, display_w, display_h);
    let (up_indices, up_opacity) = pad_indexed_bottom(
        up_indices,
        up_opacity,
        display_w,
        display_h,
        encode_h,
    );

    encode_indexed_to_sixel(
        &palette,
        &up_indices,
        &up_opacity,
        display_w,
        encode_h,
    )
    .ok()
    .map(|s| inject_raster_attributes(s.into_bytes(), display_w, encode_h))
}

/// Insert `"1;1;w;h` raster attributes so terminals (especially Zellij) erase the
/// background rectangle before drawing each frame.
fn inject_raster_attributes(sixel: Vec<u8>, width: usize, height: usize) -> Vec<u8> {
    let Some(p) = sixel.windows(2).position(|w| w == b"\x1bP") else {
        return sixel;
    };
    let Some(rel) = sixel[p + 2..].iter().position(|&b| b == b'q') else {
        return sixel;
    };
    let q_pos = p + 2 + rel;

    let mut attrs = vec![b'"'];
    attrs.extend_from_slice(b"1;1;");
    push_ascii_usize(&mut attrs, width);
    attrs.push(b';');
    push_ascii_usize(&mut attrs, height);

    let mut out = Vec::with_capacity(sixel.len() + attrs.len());
    out.extend_from_slice(&sixel[..=q_pos]);
    out.extend_from_slice(&attrs);
    out.extend_from_slice(&sixel[q_pos + 1..]);
    out
}

fn push_ascii_usize(buf: &mut Vec<u8>, mut n: usize) {
    if n == 0 {
        buf.push(b'0');
        return;
    }
    let mut digits = [0u8; 20];
    let mut i = digits.len();
    while n > 0 {
        i -= 1;
        digits[i] = b'0' + (n % 10) as u8;
        n /= 10;
    }
    buf.extend_from_slice(&digits[i..]);
}

fn upsample_indexed(
    indices: &[u8],
    opacity: &[bool],
    src_w: usize,
    src_h: usize,
    dst_w: usize,
    dst_h: usize,
) -> (Vec<u8>, Vec<bool>) {
    let mut out_idx = vec![0u8; dst_w * dst_h];
    let mut out_op = vec![false; dst_w * dst_h];
    for dy in 0..dst_h {
        let sy = dy * src_h / dst_h;
        let src_row = sy * src_w;
        let dst_row = dy * dst_w;
        for dx in 0..dst_w {
            let sx = dx * src_w / dst_w;
            let i = src_row + sx;
            out_idx[dst_row + dx] = indices[i];
            out_op[dst_row + dx] = opacity[i];
        }
    }
    (out_idx, out_op)
}

fn pad_rgba_bottom(rgba: Vec<u8>, width: usize, height: usize, encode_h: usize) -> Vec<u8> {
    if encode_h <= height {
        return rgba;
    }
    let mut out = rgba;
    out.resize(width * encode_h * 4, 0);
    for px in (width * height)..(width * encode_h) {
        let o = px * 4;
        out[o + 3] = 255;
    }
    out
}

fn pad_indexed_bottom(
    indices: Vec<u8>,
    opacity: Vec<bool>,
    width: usize,
    height: usize,
    encode_h: usize,
) -> (Vec<u8>, Vec<bool>) {
    if encode_h <= height {
        return (indices, opacity);
    }
    let mut out_idx = indices;
    let mut out_op = opacity;
    out_idx.resize(width * encode_h, 0);
    out_op.resize(width * encode_h, true);
    (out_idx, out_op)
}

/// Indexed sixel wire encoder (adapted from icy_sixel 0.5, MIT).
fn encode_indexed_to_sixel(
    palette: &[Rgb],
    indices: &[u8],
    opacity_mask: &[bool],
    width: usize,
    height: usize,
) -> Result<String, ()> {
    let mut out = String::new();
    out.push_str("\x1bP");
    write_number(&mut out, PixelAspectRatio::Square.to_p1_value() as usize);
    out.push(';');
    write_number(&mut out, BackgroundMode::Opaque.to_p2_value() as usize);
    out.push_str(";0q");

    for (i, c) in palette.iter().enumerate() {
        let r = (c.r as u32 * 100) / 255;
        let g = (c.g as u32 * 100) / 255;
        let b = (c.b as u32 * 100) / 255;
        out.push('#');
        write_number(&mut out, i);
        out.push(';');
        out.push('2');
        out.push(';');
        write_number(&mut out, r as usize);
        out.push(';');
        write_number(&mut out, g as usize);
        out.push(';');
        write_number(&mut out, b as usize);
    }

    let bands = height.div_ceil(6);
    for band in 0..bands {
        let y0 = band * 6;
        let y_max = usize::min(y0 + 6, height);

        let mut colors_used = [false; 256];
        for y in y0..y_max {
            for x in 0..width {
                let pixel_idx = y * width + x;
                if opacity_mask[pixel_idx] {
                    colors_used[indices[pixel_idx] as usize] = true;
                }
            }
        }

        for (color_index, &is_used) in colors_used.iter().enumerate().take(palette.len()) {
            if !is_used {
                continue;
            }

            out.push('#');
            write_number(&mut out, color_index);

            let mut x = 0;
            while x < width {
                let mut bits: u8 = 0;
                for bit in 0..6 {
                    let y = y0 + bit;
                    if y >= y_max {
                        break;
                    }
                    let pixel_idx = y * width + x;
                    if opacity_mask[pixel_idx] && indices[pixel_idx] as usize == color_index {
                        bits |= 1 << bit;
                    }
                }

                let mut run_len = 1usize;
                while x + run_len < width {
                    let mut bits_next: u8 = 0;
                    for bit in 0..6 {
                        let y = y0 + bit;
                        if y >= y_max {
                            break;
                        }
                        let pixel_idx = y * width + (x + run_len);
                        if opacity_mask[pixel_idx] && indices[pixel_idx] as usize == color_index {
                            bits_next |= 1 << bit;
                        }
                    }
                    if bits_next != bits {
                        break;
                    }
                    run_len += 1;
                }

                if run_len > 3 {
                    out.push('!');
                    write_number(&mut out, run_len);
                    out.push((63 + bits) as char);
                } else {
                    let ch = (63 + bits) as char;
                    for _ in 0..run_len {
                        out.push(ch);
                    }
                }
                x += run_len;
            }
            out.push('$');
        }
        out.push('-');
    }

    out.push('\x1b');
    out.push('\\');
    Ok(out)
}

fn write_number(out: &mut String, mut n: usize) {
    if n == 0 {
        out.push('0');
        return;
    }
    let mut buf = [0u8; 20];
    let mut i = buf.len();
    while n > 0 {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
    }
    out.push_str(unsafe { std::str::from_utf8_unchecked(&buf[i..]) });
}
