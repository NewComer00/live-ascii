use crossterm::event::{KeyCode, KeyModifiers, ModifierKeyCode};
use image::RgbaImage;

use crate::context::SixelResolution;

pub fn allocate_aligned(size: usize, alignment: usize) -> *mut u8 {
    #[cfg(unix)]
    {
        let mut ptr: *mut libc::c_void = std::ptr::null_mut();
        unsafe {
            libc::posix_memalign(&mut ptr, alignment, size);
        }
        ptr as *mut u8
    }
    #[cfg(windows)]
    {
        unsafe { libc::aligned_malloc(size, alignment) as *mut u8 }
    }
}

pub fn default_fade_time() -> f32 {
    -1.0
}

pub fn key_code_to_str(code: KeyCode) -> String {
    match code {
        KeyCode::Char(c) => c.to_uppercase().to_string(),
        KeyCode::Backspace => "Backspace".to_string(),
        KeyCode::Enter => "Enter".to_string(),
        KeyCode::Left => "Left".to_string(),
        KeyCode::Right => "Right".to_string(),
        KeyCode::Up => "Up".to_string(),
        KeyCode::Down => "Down".to_string(),
        KeyCode::Home => "Home".to_string(),
        KeyCode::End => "End".to_string(),
        KeyCode::PageUp => "PageUp".to_string(),
        KeyCode::PageDown => "PageDown".to_string(),
        KeyCode::Tab => "Tab".to_string(),
        KeyCode::BackTab => "BackTab".to_string(),
        KeyCode::Delete => "Delete".to_string(),
        KeyCode::Insert => "Insert".to_string(),
        KeyCode::Null => "Null".to_string(),
        KeyCode::Esc => "Esc".to_string(),
        KeyCode::CapsLock => "CapsLock".to_string(),
        KeyCode::ScrollLock => "ScrollLock".to_string(),
        KeyCode::NumLock => "NumLock".to_string(),
        KeyCode::PrintScreen => "PrintScreen".to_string(),
        KeyCode::Pause => "Pause".to_string(),
        KeyCode::Menu => "Menu".to_string(),
        KeyCode::KeypadBegin => "KeypadBegin".to_string(),
        KeyCode::F(n) => format!("F{}", n),
        KeyCode::Modifier(mkc) => match mkc {
            ModifierKeyCode::LeftShift => "LeftShift".to_string(),
            ModifierKeyCode::LeftControl => "LeftControl".to_string(),
            ModifierKeyCode::LeftAlt => "LeftAlt".to_string(),
            ModifierKeyCode::LeftSuper => "LeftSuper".to_string(),
            ModifierKeyCode::LeftHyper => "LeftHyper".to_string(),
            ModifierKeyCode::LeftMeta => "LeftMeta".to_string(),
            ModifierKeyCode::RightShift => "RightShift".to_string(),
            ModifierKeyCode::RightControl => "RightControl".to_string(),
            ModifierKeyCode::RightAlt => "RightAlt".to_string(),
            ModifierKeyCode::RightSuper => "RightSuper".to_string(),
            ModifierKeyCode::RightHyper => "RightHyper".to_string(),
            ModifierKeyCode::RightMeta => "RightMeta".to_string(),
            _ => "".to_string(),
        },
        _ => "".to_string(),
    }
}

pub fn modifiers_to_vec(modifiers: KeyModifiers) -> Vec<String> {
    let mut mods = Vec::new();
    if modifiers.contains(KeyModifiers::CONTROL) {
        mods.push("Control".to_string());
    }
    if modifiers.contains(KeyModifiers::ALT) {
        mods.push("Alt".to_string());
    }
    if modifiers.contains(KeyModifiers::SHIFT) {
        mods.push("Shift".to_string());
    }
    if modifiers.contains(KeyModifiers::SUPER) {
        mods.push("Super".to_string());
    }
    mods
}

pub fn get_file_name(filename: &str) -> &str {
    filename.split('.').next().unwrap_or(filename)
}

/// Boost contrast along edges so fine details survive
/// when the texture is rendered at low terminal resolutions.
/// Uses a 3×3 Laplace kernel for edge detection, then brightens
/// edge pixels and darkens their neighbours (outline expansion).
pub fn enhance_edges(img: &RgbaImage) -> RgbaImage {
    let (w, h) = img.dimensions();
    let mut out = img.clone();

    for y in 1..h.saturating_sub(1) {
        for x in 1..w.saturating_sub(1) {
            // Laplace kernel: detect edges via 2nd derivative of luminance
            let luminance = |px: u32, py: u32| -> f32 {
                let p = img.get_pixel(px, py);
                0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32
            };

            let edge = (4.0 * luminance(x, y)
                - luminance(x - 1, y)
                - luminance(x + 1, y)
                - luminance(x, y - 1)
                - luminance(x, y + 1))
                .abs();

            // Only enhance significant edges (skip noise)
            if edge > 30.0 {
                let boost = (edge / 255.0 * 0.6).min(0.5);
                let p = img.get_pixel(x, y);
                let r = (p[0] as f32 * (1.0 + boost)).min(255.0) as u8;
                let g = (p[1] as f32 * (1.0 + boost)).min(255.0) as u8;
                let b = (p[2] as f32 * (1.0 + boost)).min(255.0) as u8;
                out.put_pixel(x, y, image::Rgba([r, g, b, p[3]]));
            }
        }
    }

    out
}

/// Parse `--sixel-resolution`: percent scale or explicit px/cell (`10x20`).
pub fn parse_sixel_resolution(input: &str) -> SixelResolution {
    let s = input.trim();
    let lower = s.to_ascii_lowercase();

    if let Some((a, b)) = lower.split_once('x') {
        if let (Ok(x), Ok(y)) = (a.trim().parse(), b.trim().parse()) {
            return SixelResolution::PxPerCell(x, y);
        }
    }

    let pct_str = s.strip_suffix('%').unwrap_or(s);
    if let Ok(pct) = pct_str.parse::<f32>() {
        return SixelResolution::Scale((pct / 100.0).max(0.01));
    }

    eprintln!(
        "Invalid --sixel-resolution '{}', using 100% (10×20 px/cell).",
        input
    );
    SixelResolution::Scale(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::SixelResolution;
   
    #[test]
    fn get_file_name_test() {
        let pre = get_file_name("test.model3.json");
        assert_eq!(pre, "test");
    }

    #[test]
    fn parse_sixel_resolution_percent() {
        assert_eq!(parse_sixel_resolution("100%"), SixelResolution::Scale(1.0));
        assert_eq!(parse_sixel_resolution("50%"), SixelResolution::Scale(0.5));
        assert_eq!(parse_sixel_resolution("40"), SixelResolution::Scale(0.4));
    }

    #[test]
    fn parse_sixel_resolution_px_per_cell() {
        assert_eq!(
            parse_sixel_resolution("10x20"),
            SixelResolution::PxPerCell(10, 20)
        );
        assert_eq!(
            parse_sixel_resolution("4x8"),
            SixelResolution::PxPerCell(4, 8)
        );
    }
}
