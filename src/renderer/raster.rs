//! Software rasterization — walk triangles with barycentric coordinates,
//! apply texture mapping with alpha blending and Cubism blend modes
//! (normal / additive / multiply), plus screen/multiply color tinting.
use image::{DynamicImage, GenericImageView};

use crate::context::Context;
use crate::ffi::*;
use crate::geometry::*;

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
    let triangle = Triangle::new(v0, v1, v2);
    let total_area = triangle.signed_area();
    if total_area == 0.0 {
        return;
    }

    let bbox = triangle.get_box();
    let min_x = bbox.minx.max(0.0) as u16;
    let max_x = bbox.maxx.min((viewport_w - 1) as f32) as u16;
    let min_y = bbox.miny.max(0.0) as u16;
    let max_y = bbox.maxy.min((viewport_h - 1) as f32) as u16;

    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let p = Vec3 { x: x as f32, y: y as f32, z: 0.0 };
            let w0 = Triangle::new(v1, v2, p).signed_area() / total_area;
            let w1 = Triangle::new(v2, v0, p).signed_area() / total_area;
            let w2 = 1.0 - w0 - w1;
            if w0 >= 0.0 && w1 >= 0.0 && w2 >= 0.0 {
                write_pixel(x, y, w0, w1, w2);
            }
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
    let rw = context.render_width();
    let rh = context.render_height();

    for i in (0..m_index_count).step_by(3) {
        let i0 = unsafe { *m_indices_ptr.add(i) } as usize;
        let i1 = unsafe { *m_indices_ptr.add(i + 1) } as usize;
        let i2 = unsafe { *m_indices_ptr.add(i + 2) } as usize;

        let v0 = transform(unsafe { *m_vertices_ptr.add(i0) });
        let v1 = transform(unsafe { *m_vertices_ptr.add(i1) });
        let v2 = transform(unsafe { *m_vertices_ptr.add(i2) });

        walk_triangle(v0, v1, v2, rw, rh, |x, y, _, _, _| {
            mask_buffer[(y as usize) * (rw as usize) + (x as usize)] = true;
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
    let img_w = texture.width();
    let img_h = texture.height();
    let rw = context.render_width();
    let rh = context.render_height();

    let index_count = unsafe { *index_counts.add(drawable_idx) } as usize;
    let indices_ptr = unsafe { *indices.add(drawable_idx) };
    let vertices_ptr = unsafe { *vt_positions.add(drawable_idx) };
    let uvs_ptr = unsafe { *vertex_uvs.add(drawable_idx) };

    let mc = unsafe { *multiply_colors.add(drawable_idx) };
    let sc = unsafe { *screen_colors.add(drawable_idx) };
    let blend = unsafe { *blend_modes.add(drawable_idx) };
    let mode = (blend & 0xFF) as u8;

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
            if has_mask && !mask_buffer[(y as usize) * (rw as usize) + (x as usize)] {
                return;
            }

            let u = (w0 * uv0.x + w1 * uv1.x + w2 * uv2.x).clamp(0.0, 1.0);
            let v = (w0 * uv0.y + w1 * uv1.y + w2 * uv2.y).clamp(0.0, 1.0);

            let tex_x = (u * (img_w as f32 - 1.0)) as u32;
            let tex_y = ((1.0 - v) * (img_h as f32 - 1.0)) as u32;

            if tex_x >= img_w || tex_y >= img_h {
                return;
            }

            let p = texture.get_pixel(tex_x, tex_y);
            let a = p[3];
            if a == 0 {
                return;
            }

            let final_alpha = (a as f32 / 255.0) * opacity;
            if final_alpha <= 0.004 {
                return;
            }

            let (dr, dg, db) = context.get_pixel_color(x, y);

            // Multiply color tint
            let src_r = (p[0] as f32 / 255.0) * mc.x;
            let src_g = (p[1] as f32 / 255.0) * mc.y;
            let src_b = (p[2] as f32 / 255.0) * mc.z;

            // Blend mode dispatch
            let (out_r, out_g, out_b) = match mode {
                1 | 3 => (
                    // Additive
                    (dr as f32 / 255.0 + src_r * final_alpha).min(1.0),
                    (dg as f32 / 255.0 + src_g * final_alpha).min(1.0),
                    (db as f32 / 255.0 + src_b * final_alpha).min(1.0),
                ),
                2 | 6 => {
                    // Multiply
                    let d0 = dr as f32 / 255.0;
                    let d1 = dg as f32 / 255.0;
                    let d2 = db as f32 / 255.0;
                    (
                        d0 * (1.0 - final_alpha) + d0 * src_r * final_alpha,
                        d1 * (1.0 - final_alpha) + d1 * src_g * final_alpha,
                        d2 * (1.0 - final_alpha) + d2 * src_b * final_alpha,
                    )
                }
                _ => {
                    // Normal alpha composite
                    let inv = 1.0 - final_alpha;
                    (
                        src_r * final_alpha + (dr as f32 / 255.0) * inv,
                        src_g * final_alpha + (dg as f32 / 255.0) * inv,
                        src_b * final_alpha + (db as f32 / 255.0) * inv,
                    )
                }
            };

            // Screen color brightening (post-composite)
            let out_r = 1.0 - (1.0 - out_r) * (1.0 - sc.x);
            let out_g = 1.0 - (1.0 - out_g) * (1.0 - sc.y);
            let out_b = 1.0 - (1.0 - out_b) * (1.0 - sc.z);

            context.set_pixel_color(
                x,
                y,
                (out_r * 255.0).clamp(0.0, 255.0) as u8,
                (out_g * 255.0).clamp(0.0, 255.0) as u8,
                (out_b * 255.0).clamp(0.0, 255.0) as u8,
            );
        });
    }
}
