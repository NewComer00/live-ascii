//! Live2D renderer — own the model, run the main loop, and coordinate
//! submodules for input handling, action dispatch, and rasterization.
#![allow(dead_code)]

use std::error::Error;
use std::ffi::CStr;
use std::io::stdout;
use std::time::Duration;
use std::time::Instant;

use crossterm::{
    cursor,
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyEvent, KeyEventKind, MouseEvent},
    execute,
    terminal::{self},
};
use image::DynamicImage;
use ratatui::style::Color;
use ratatui::{Terminal, backend::CrosstermBackend};

mod action;
mod input;
mod raster;

use crate::context::*;
use crate::controller::*;
use crate::effect::pose::*;
use crate::expression::manager::*;
use crate::ffi::*;
use crate::geometry::*;
use crate::model::*;
use crate::model_setting::ModelSetting;
use crate::motion::manager::*;
use crate::physics::*;
use crate::shader::*;
use crate::ui::{popup::*, *};
use input::DragState;

pub struct Renderer {
    pub count: usize,
    pub model: Model,
    constant_flags: *const u8,
    texture_indices: *const i32,
    vertex_counts: *const i32,
    vertex_positions: *const *const CsmVector2,
    vertex_uvs: *const *const CsmVector2,
    index_counts: *const i32,
    indices: *const *const u16,
    multiply_colors: *const CsmVector4,
    screen_colors: *const CsmVector4,
    shader_manager: ShaderManager,
    mask_counts: *const i32,
    masks: *const *const i32,
    textures: Vec<DynamicImage>,
    blend_modes: *const i32,
    // View transform — mutated by keyboard and mouse
    offset_x: f32,
    offset_y: f32,
    scale: f32,
    // Persistent drag state (pan-by-mouse)
    drag: DragState,
    start_time: Instant,
}

impl Renderer {
    pub fn new(
        model_ptr: *mut CsmModel,
        textures: Vec<DynamicImage>,
        shader_manager: ShaderManager,
    ) -> Self {
        let model = Model::new(model_ptr);
        unsafe {
            Self {
                model,
                count: csmGetDrawableCount(model_ptr) as usize,
                constant_flags: csmGetDrawableConstantFlags(model_ptr),
                texture_indices: csmGetDrawableTextureIndices(model_ptr),
                vertex_counts: csmGetDrawableVertexCounts(model_ptr),
                vertex_positions: csmGetDrawableVertexPositions(model_ptr),
                vertex_uvs: csmGetDrawableVertexUvs(model_ptr),
                index_counts: csmGetDrawableIndexCounts(model_ptr),
                indices: csmGetDrawableIndices(model_ptr),
                multiply_colors: csmGetDrawableMultiplyColors(model_ptr),
                screen_colors: csmGetDrawableScreenColors(model_ptr),
                shader_manager,
                mask_counts: csmGetDrawableMaskCounts(model_ptr),
                masks: csmGetDrawableMasks(model_ptr),
                blend_modes: csmGetDrawableBlendModes(model_ptr),
                textures,
                offset_x: 0.0,
                offset_y: 0.0,
                scale: 1.0,
                drag: None,
                start_time: Instant::now(),
            }
        }
    }

    pub fn render(
        &mut self,
        context: &mut Context,
        mm: &mut MotionManager,
        model_setting: &mut ModelSetting,
        em: &mut ExpressionManager,
        pose: &mut Option<Pose>,
        physics: &mut Option<Physics>,
    ) -> Result<(), Box<dyn Error>> {
        terminal::enable_raw_mode()?;
        execute!(stdout(), cursor::Hide)?;
        // Clear screen before first frame (handles previous app content,
        // transparent backgrounds, etc.)
        {
            use std::io::Write;
            let mut out = stdout();
            out.write_all(b"\x1b[2J")?;
            out.flush()?;
        }
        let backend = CrosstermBackend::new(stdout());
        let mut terminal = Terminal::new(backend)?;
        if context.mouse {
            execute!(stdout(), EnableMouseCapture)?;
        }

        let mut shader: Shader = self.shader_manager.current_shader().clone();
        let mut _text_chars: Option<Vec<char>> = if let Shader::Text(ref t) = shader {
            Some(t.chars().collect())
        } else {
            None
        };

        let fps = 60.0;
        let target_frame_time = Duration::from_secs_f64(1.0 / fps);
        let mut last_frame = Instant::now();

        if let Some(p) = pose {
            p.reset(&mut self.model);
        }

        let mut mask_buffer =
            vec![false; (context.render_width() as usize) * (context.render_height() as usize)];
        let mut face_controller = FaceController::new(0.3);

        loop {
            let frame_start = Instant::now();

            // ── Events ──────────────────────────────────────────────────────
            if event::poll(Duration::from_millis(1))? {
                match event::read()? {
                    Event::Key(KeyEvent { code, modifiers, kind, .. })
                        if kind == KeyEventKind::Press =>
                    {
                        if input::handle_key(  // renderer::input
                            code,
                            modifiers,
                            context,
                            mm,
                            model_setting,
                            pose,
                            &mut self.model,
                            &mut self.offset_x,
                            &mut self.offset_y,
                            &mut self.scale,
                        ) {
                            break;
                        }
                    }
                    Event::Mouse(MouseEvent { kind, column, row, .. }) if context.mouse => {
                        input::handle_mouse(  // renderer::input
                            kind,
                            column,
                            row,
                            &mut self.drag,
                            &mut self.offset_x,
                            &mut self.offset_y,
                            &mut self.scale,
                            context.width,
                            context.height,
                        );
                    }
                    _ => {}
                }
            }

            // ── Actions ──────────────────────────────────────────────────────
            action::dispatch(  // renderer::action
                context,
                em,
                mm,
                pose,
                &mut self.model,
                &mut self.shader_manager,
                &mut shader,
                &mut _text_chars,
            );

            // ── Model update ─────────────────────────────────────────────────
            context.update()?;
            context.clear();

            let needed =
                (context.render_width() as usize) * (context.render_height() as usize);
            if mask_buffer.len() != needed {
                mask_buffer.resize(needed, false);
            }
            mask_buffer.fill(false);

            let delta_time = last_frame.elapsed().as_secs_f32();
            last_frame = Instant::now();

            self.model.load_parameters();
            mm.update_motion(&mut self.model, delta_time);

            if context.camera {
                if let Some(packet) = context.tracker.latest() {
                    if packet.success == 1 {
                        face_controller.update_parameters(&mut self.model, &packet);
                    }
                }
            }

            if let Some(p) = pose {
                p.update_parameters(&mut self.model, delta_time);
            }
            self.model.save_parameters();

            if let Some(p) = physics {
                if context.use_physics {
                    p.evaluate(&mut self.model, delta_time);
                }
            }

            em.update_motion(&mut self.model, delta_time);

            unsafe {
                csmResetDrawableDynamicFlags(self.model.model);
                csmUpdateModel(self.model.model);
            }

            // ── Rasterization ────────────────────────────────────────────────
            let (dy_flags, opacities, vt_positions, render_orders) = unsafe {
                let dy_flags = csmGetDrawableDynamicFlags(self.model.model);
                let opacities = csmGetDrawableOpacities(self.model.model);
                let vt_positions = csmGetDrawableVertexPositions(self.model.model);
                let render_orders = csmGetRenderOrders(self.model.model);
                self.multiply_colors = csmGetDrawableMultiplyColors(self.model.model);
                self.screen_colors = csmGetDrawableScreenColors(self.model.model);
                self.blend_modes = csmGetDrawableBlendModes(self.model.model);
                (dy_flags, opacities, vt_positions, render_orders)
            };

            let mut drawables: Vec<usize> = (0..self.count).collect();
            drawables.sort_by_key(|&i| unsafe { *render_orders.add(i) });

            let render_w = context.render_width();
            let render_h = context.render_height();
            let transform = |v: CsmVector2| { self.transform_to_screen(v, render_w, render_h) };

            for &drawable_idx in &drawables {
                unsafe {
                    let flags = *dy_flags.add(drawable_idx);
                    if (flags & CSM_IS_VISIBLE) == 0 {
                        continue;
                    }
                    let opacity = *opacities.add(drawable_idx);
                    if opacity <= 0.001 {
                        continue;
                    }

                    let mask_count = *self.mask_counts.add(drawable_idx) as usize;
                    let has_mask = mask_count > 0;

                    if has_mask {
                        mask_buffer.fill(false);
                        let mask_indices_ptr = *self.masks.add(drawable_idx);
                        for m in 0..mask_count {
                            let mask_idx = *mask_indices_ptr.add(m) as usize;
                            raster::rasterize_mask(  // renderer::raster
                                mask_idx,
                                self.index_counts,
                                self.indices,
                                vt_positions,
                                context,
                                &transform,
                                &mut mask_buffer,
                            );
                        }
                    }

                    let tex_idx = *self.texture_indices.add(drawable_idx) as usize;
                    if tex_idx >= self.textures.len() {
                        continue;
                    }

                    raster::rasterize_drawable(  // renderer::raster
                        drawable_idx,
                        self.index_counts,
                        self.indices,
                        vt_positions,
                        self.vertex_uvs,
                        self.multiply_colors,
                        self.screen_colors,
                        self.blend_modes,
                        &self.textures[tex_idx],
                        opacity,
                        has_mask,
                        &mask_buffer,
                        context,
                        &transform,
                    );
                }
            }

            // ── Output ───────────────────────────────────────────────────────
            match context.image_protocol {
                ImageProtocol::Sixel => {
                    let sixel_data = context.buffer_to_sixel();
                    let mut stdout = stdout();
                    use std::io::Write;
                    stdout.write_all(b"\x1b[?80h")?; // DECSDM: no-scroll
                    stdout.write_all(b"\x1b[H")?;
                    stdout.write_all(&sixel_data)?;
                    stdout.flush()?;
                    stdout.write_all(b"\x1b[?80l")?; // restore
                    stdout.flush()?;
                }
                ImageProtocol::Kitty => {
                    let kitty_data = context.buffer_to_kitty();
                    if !kitty_data.is_empty() {
                        let mut stdout = stdout();
                        use std::io::Write;
                        stdout.write_all(b"\x1b[H")?;
                        stdout.write_all(&kitty_data)?;
                        stdout.flush()?;
                    }
                }
                ImageProtocol::HalfBlock => {
                    terminal.draw(|f| {
                        if let Err(e) = ui(f, context, &self.model, &mm, &em) {
                            eprintln!("{:?}", e);
                        }
                    })?;
                }
            }

            // ── Post-frame ───────────────────────────────────────────────────
            while let Ok(msg) = context.msg_chan.1.try_recv() {
                if msg.show {
                    let len = msg.content.len();
                    let display_width = (len + 2).min(msg.max_width as usize);
                    let content_width = display_width.saturating_sub(2).max(1);
                    let row_num =
                        (2 + (len + content_width - 1) / content_width).min(msg.max_height as usize);
                    context.popups.push_or_update(Popup::new_with_id(
                        msg.content,
                        Duration::from_secs_f64(msg.duration),
                        (display_width, row_num),
                        Color::Rgb(msg.color.0, msg.color.1, msg.color.2),
                        msg.id,
                    ));
                } else {
                    context.popups.delete(msg.id);
                }
            }
            context.popups.update();

            let elapsed = frame_start.elapsed();
            if elapsed < target_frame_time {
                std::thread::sleep(target_frame_time - elapsed);
            }
        }

        if context.mouse {
            execute!(stdout(), DisableMouseCapture)?;
        }
        execute!(stdout(), cursor::Show)?;
        terminal::disable_raw_mode()?;
        Ok(())
    }

    pub fn transform_to_screen(&self, vertex: CsmVector2, width: u16, height: u16) -> Vec3 {
        let w = width as f32;
        let h = height as f32;
        let base_scale = (w / 2.0).min(h / 2.0);
        let final_scale = base_scale * self.scale;
        Vec3 {
            x: vertex.x * final_scale + w / 2.0 + self.offset_x * w,
            y: -vertex.y * final_scale + h / 2.0 + self.offset_y * h,
            z: 0.0,
        }
    }

    /// Apply startup view transform from CLI flags (scale, offset_x, offset_y).
    pub fn apply_startup_transform(&mut self, scale: f32, offset_x: f32, offset_y: f32) {
        self.scale = scale;
        self.offset_x = offset_x;
        self.offset_y = offset_y;
    }

    pub fn find_param_index(&self, target_id: &str) -> Option<usize> {
        unsafe {
            let count = csmGetParameterCount(self.model.model) as usize;
            let ids_ptr = csmGetParameterIds(self.model.model);
            if ids_ptr.is_null() {
                return None;
            }
            for i in 0..count {
                let id_ptr = *ids_ptr.add(i);
                if !id_ptr.is_null() {
                    let c_str = CStr::from_ptr(id_ptr);
                    if c_str.to_str().ok() == Some(target_id) {
                        return Some(i);
                    }
                }
            }
        }
        None
    }

    pub fn find_part_index(&self, target_id: &str) -> Option<usize> {
        unsafe {
            let count = csmGetPartCount(self.model.model) as usize;
            let ids_ptr = csmGetPartIds(self.model.model);
            if ids_ptr.is_null() {
                return None;
            }
            for i in 0..count {
                let id_ptr = *ids_ptr.add(i);
                if !id_ptr.is_null() {
                    let c_str = CStr::from_ptr(id_ptr);
                    if c_str.to_str().ok() == Some(target_id) {
                        return Some(i);
                    }
                }
            }
        }
        None
    }
}
