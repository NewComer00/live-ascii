//! Software rasterization — walk triangles with barycentric coordinates,
//! apply texture mapping with alpha blending and Cubism blend modes
//! (normal / additive / multiply), plus screen/multiply color tinting.
use image::DynamicImage;

use crate::context::Context;
use crate::ffi::*;
use crate::geometry::Vec3;

const INV255: f32 = 1.0 / 255.0;

/// Signed edge function for triangle edge (a → b) at point (px, py).
#[inline(always)]
fn edge(a: Vec3, b: Vec3, px: f32, py: f32) -> f32 {
    (b.x - a.x) * (py - a.y) - (b.y - a.y) * (px - a.x)
}

/// Walk every pixel covered by a triangle (defined by three screen-space Vec3 corners)
/// and call `write_pixel(x, y, bary_w0, bary_w1, bary_w2)` for each interior sample.
///
/// The barycentric weights correspond to (v1,v2,p), (v2,v0,p), and (1-w0-w1) respectively,
/// matching the winding used throughout this renderer.
#[inline]
pub fn walk_triangle<F>(
    v0: Vec3,
    v1: Vec3,
    v2: Vec3,
    viewport_w: u16,
    viewport_h: u16,
    mut write_pixel: F,
)
where
    F: FnMut(u16, u16, f32, f32, f32),
{
    let total_area =
        0.5 * ((v1.x - v0.x) * (v2.y - v0.y) - (v1.y - v0.y) * (v2.x - v0.x));
    if total_area == 0.0 {
        return;
    }
    let inv_area = 0.5 / total_area;

    let minx = v0.x.min(v1.x).min(v2.x);
    let miny = v0.y.min(v1.y).min(v2.y);
    let maxx = v0.x.max(v1.x).max(v2.x);
    let maxy = v0.y.max(v1.y).max(v2.y);

    let min_x = minx.max(0.0) as u16;
    let max_x = maxx.min((viewport_w - 1) as f32) as u16;
    let min_y = miny.max(0.0) as u16;
    let max_y = maxy.min((viewport_h - 1) as f32) as u16;

    let step_x0 = v1.y - v2.y;
    let step_x1 = v2.y - v0.y;

    for y in min_y..=max_y {
        let py = y as f32;
        let px = min_x as f32;
        let mut e0 = edge(v1, v2, px, py);
        let mut e1 = edge(v2, v0, px, py);

        for x in min_x..=max_x {
            let w0 = e0 * inv_area;
            let w1 = e1 * inv_area;
            if w0 >= 0.0 && w1 >= 0.0 && w0 + w1 <= 1.0 {
                write_pixel(x, y, w0, w1, 1.0 - w0 - w1);
            }
            e0 += step_x0;
            e1 += step_x1;
        }
    }
}

/// Rasterize mask drawable triangles into `mask_buffer`.
pub unsafe fn rasterize_mask(
    mask_idx: usize,
    index_counts: *const i32,
    indices: *const *const u16,
    vt_positions: *const *const CsmVector2,
    context: &Context,
    transform: &impl Fn(CsmVector2) -> Vec3,
    mask_buffer: &mut Vec<bool>,
) {
    let m_index_count = unsafe { *index_counts.add(mask_idx) } as usize;
    let m_indices_ptr = unsafe { *indices.add(mask_idx) };
    let m_vertices_ptr = unsafe { *vt_positions.add(mask_idx) };
    let rw = context.render_width() as usize;

    for i in (0..m_index_count).step_by(3) {
        let i0 = unsafe { *m_indices_ptr.add(i) } as usize;
        let i1 = unsafe { *m_indices_ptr.add(i + 1) } as usize;
        let i2 = unsafe { *m_indices_ptr.add(i + 2) } as usize;

        let v0 = transform(unsafe { *m_vertices_ptr.add(i0) });
        let v1 = transform(unsafe { *m_vertices_ptr.add(i1) });
        let v2 = transform(unsafe { *m_vertices_ptr.add(i2) });

        walk_triangle(v0, v1, v2, rw as u16, context.render_height(), |x, y, _, _, _| {
            mask_buffer[y as usize * rw + x as usize] = true;
        });
    }
}

/// Rasterize one drawable into `context.pixel_buffer` with full blend pipeline.
pub unsafe fn rasterize_drawable(
    drawable_idx: usize,
    index_counts: *const i32,
    indices: *const *const u16,
    vt_positions: *const *const CsmVector2,
    vertex_uvs: *const *const CsmVector2,
    multiply_colors: *const CsmVector4,
    screen_colors: *const CsmVector4,
    blend_modes: *const i32,
    texture: &DynamicImage,
    opacity: f32,
    has_mask: bool,
    mask_buffer: &[bool],
    context: &mut Context,
    transform: &impl Fn(CsmVector2) -> Vec3,
) {
    let tex = texture
        .as_rgba8()
        .expect("Live2D textures are loaded as RGBA8");
    let tex_data = tex.as_raw();
    let img_w = tex.width();
    let img_h = tex.height();
    let u_scale = (img_w - 1) as f32;
    let v_scale = (img_h - 1) as f32;

    let rw = context.render_width();
    let rh = context.render_height();
    let rw_usize = rw as usize;

    let index_count = unsafe { *index_counts.add(drawable_idx) } as usize;
    let indices_ptr = unsafe { *indices.add(drawable_idx) };
    let vertices_ptr = unsafe { *vt_positions.add(drawable_idx) };
    let uvs_ptr = unsafe { *vertex_uvs.add(drawable_idx) };

    let mc = unsafe { *multiply_colors.add(drawable_idx) };
    let sc = unsafe { *screen_colors.add(drawable_idx) };
    let mode = (unsafe { *blend_modes.add(drawable_idx) } & 0xFF) as u8;

    let pixel_buffer = &mut context.pixel_buffer;

    for i in (0..index_count).step_by(3) {
        let i0 = unsafe { *indices_ptr.add(i) } as usize;
        let i1 = unsafe { *indices_ptr.add(i + 1) } as usize;
        let i2 = unsafe { *indices_ptr.add(i + 2) } as usize;

        let v0 = transform(unsafe { *vertices_ptr.add(i0) });
        let v1 = transform(unsafe { *vertices_ptr.add(i1) });
        let v2 = transform(unsafe { *vertices_ptr.add(i2) });

        let uv0 = unsafe { *uvs_ptr.add(i0) };
        let uv1 = unsafe { *uvs_ptr.add(i1) };
        let uv2 = unsafe { *uvs_ptr.add(i2) };

        walk_triangle(v0, v1, v2, rw, rh, |x, y, w0, w1, w2| {
            let buf_idx = y as usize * rw_usize + x as usize;

            if has_mask && !mask_buffer[buf_idx] {
                return;
            }

            let u = (w0 * uv0.x + w1 * uv1.x + w2 * uv2.x).clamp(0.0, 1.0);
            let v = (w0 * uv0.y + w1 * uv1.y + w2 * uv2.y).clamp(0.0, 1.0);

            let tex_x = (u * u_scale) as u32;
            let tex_y = ((1.0 - v) * v_scale) as u32;
            let tex_idx = ((tex_y * img_w + tex_x) as usize) * 4;

            let a = tex_data[tex_idx + 3];
            if a == 0 {
                return;
            }

            let final_alpha = (a as f32 * INV255) * opacity;
            if final_alpha <= 0.004 {
                return;
            }

            let [dr, dg, db, _] = pixel_buffer[buf_idx];

            let src_r = tex_data[tex_idx] as f32 * INV255 * mc.x;
            let src_g = tex_data[tex_idx + 1] as f32 * INV255 * mc.y;
            let src_b = tex_data[tex_idx + 2] as f32 * INV255 * mc.z;

            let (out_r, out_g, out_b) = match mode {
                1 | 3 => {
                    let d0 = dr as f32 * INV255;
                    let d1 = dg as f32 * INV255;
                    let d2 = db as f32 * INV255;
                    (
                        (d0 + src_r * final_alpha).min(1.0),
                        (d1 + src_g * final_alpha).min(1.0),
                        (d2 + src_b * final_alpha).min(1.0),
                    )
                }
                2 | 6 => {
                    let d0 = dr as f32 * INV255;
                    let d1 = dg as f32 * INV255;
                    let d2 = db as f32 * INV255;
                    let blend = final_alpha;
                    (
                        d0 * (1.0 - blend + src_r * blend),
                        d1 * (1.0 - blend + src_g * blend),
                        d2 * (1.0 - blend + src_b * blend),
                    )
                }
                _ => {
                    let inv = 1.0 - final_alpha;
                    let d0 = dr as f32 * INV255;
                    let d1 = dg as f32 * INV255;
                    let d2 = db as f32 * INV255;
                    (
                        src_r * final_alpha + d0 * inv,
                        src_g * final_alpha + d1 * inv,
                        src_b * final_alpha + d2 * inv,
                    )
                }
            };

            let out_r = 1.0 - (1.0 - out_r) * (1.0 - sc.x);
            let out_g = 1.0 - (1.0 - out_g) * (1.0 - sc.y);
            let out_b = 1.0 - (1.0 - out_b) * (1.0 - sc.z);

            pixel_buffer[buf_idx] = [
                (out_r * 255.0).clamp(0.0, 255.0) as u8,
                (out_g * 255.0).clamp(0.0, 255.0) as u8,
                (out_b * 255.0).clamp(0.0, 255.0) as u8,
                255,
            ];
        });
    }
}
