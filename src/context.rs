use std::sync::atomic::{AtomicU32, Ordering};
use std::collections::HashMap;
use std::error::Error;
use std::io::Write;
use std::sync::{
    Arc,
    mpsc::{Sender, Receiver, channel}
};

use ratatui::widgets::ListState;

use crossterm::terminal;
use icy_sixel::{EncodeOptions, QuantizeMethod};
use ratatui::style::{Color as RatatuiColor, Style};
use ratatui::text::{Line, Span, Text};

use crate::live::json::*;
use crate::tracker::*;
use crate::model_setting::ModelSetting;
use crate::ui::popup::*;
use crate::receiver::*;

/// Global atomic counter for Kitty image IDs, preventing too many resident images in the terminal.
static KITTY_IMAGE_ID: AtomicU32 = AtomicU32::new(1);

/// VT340 / Windows Terminal reference scale (10×20 px per cell).
pub const SIXEL_REFERENCE_PX_PER_CELL_X: u16 = 10;
pub const SIXEL_REFERENCE_PX_PER_CELL_Y: u16 = 20;

const KITTY_PX_PER_CELL_X: u16 = 10;
const KITTY_PX_PER_CELL_Y: u16 = 20;

/// How sixel encode resolution is chosen (see `--sixel-resolution`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SixelResolution {
    /// Scale relative to reference (100% = 10×20 px/cell).
    Scale(f32),
    /// Fixed pixels per terminal cell, e.g. `10x20`.
    PxPerCell(u16, u16),
}

/// Supported image output protocol.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ImageProtocol {
    HalfBlock,
    Sixel,
    Kitty,
}

#[derive(Debug)]
pub enum OpPanel {
    None,
    Motions,
}

#[derive(Debug)]
pub enum DebugPanel {
    None,
    Parameters,
    PartOpacities,
    AppliedExp,
    ActionQueue,
    Camera,
    Manager,
}

#[derive(Debug)]
pub enum Panel {
    None,
    Op,
    Debug,
}


#[derive(Debug)]
pub struct Context {
    pub width: u16,
    pub height: u16,
    /// Per-pixel color buffer at render_width × render_height resolution.
    /// Stored as [r, g, b, a] arrays — guaranteed contiguous layout, compatible
    /// with bytemuck::cast_slice for zero-copy &[u8] reinterpretation.
    pub pixel_buffer: Vec<[u8; 4]>,
    /// Scratch buffer for sixel encoding: holds RGBA bytes with alpha forced to
    /// 255. Reused across frames to avoid per-frame heap allocation.
    pub sixel_scratch: Vec<u8>,
    /// Reused ANSI output for half-block direct terminal writes.
    pub half_block_scratch: Vec<u8>,
    pub image: bool,
    pub base_dir: Arc<str>,
    pub model_setting: ModelSetting,
    // motion panel
    pub motion_list_state: ListState,
    // parameter debug panel
    pub param_list_state: ListState,
    // camera debug offset
    pub camera_offset: u16,
    pub context_offset: u16,
    pub current_op_panel: OpPanel,
    pub current_debug_panel: DebugPanel,
    pub current_panel: Panel,
    pub live_setting: Option<Live>,
    pub action_queue: Vec<Action>,
    pub active_expressions: std::collections::HashMap<String, usize>,

    pub tracker: Tracker,
    pub camera: bool,
    pub receiver: Option<MsgReceiver>,
    pub msg_chan: (Sender<Msg>, Receiver<Msg>),
    pub use_physics: bool,
    pub popups: Popups,
    pub image_protocol: ImageProtocol,
    /// Sixel encode resolution (`--sixel-resolution`).
    pub sixel_resolution: SixelResolution,
    pub mouse: bool,
    pub bg_color: (u8, u8, u8, u8),
}

impl Context {
    pub fn pixel_height(&self) -> u16 {
        self.height * 2
    }

    /// Width in pixels for rasterization target.
    pub fn render_width(&self) -> u16 {
        match self.image_protocol {
            ImageProtocol::Sixel => self.width * SIXEL_REFERENCE_PX_PER_CELL_X,
            ImageProtocol::Kitty => self.width * KITTY_PX_PER_CELL_X,
            ImageProtocol::HalfBlock => self.width,
        }
    }

    /// Height in pixels for rasterization target.
    pub fn render_height(&self) -> u16 {
        match self.image_protocol {
            ImageProtocol::Sixel => self.height * SIXEL_REFERENCE_PX_PER_CELL_Y,
            ImageProtocol::Kitty => self.height * KITTY_PX_PER_CELL_Y,
            ImageProtocol::HalfBlock => self.pixel_height(),
        }
    }

    /// Pixels per terminal cell for quantette (`--sixel-resolution`).
    fn sixel_quant_px_per_cell(&self) -> (u16, u16) {
        match self.sixel_resolution {
            SixelResolution::Scale(scale) => {
                let scale = scale.max(0.01);
                (
                    (SIXEL_REFERENCE_PX_PER_CELL_X as f32 * scale)
                        .round()
                        .max(1.0) as u16,
                    (SIXEL_REFERENCE_PX_PER_CELL_Y as f32 * scale)
                        .round()
                        .max(1.0) as u16,
                )
            }
            SixelResolution::PxPerCell(x, y) => (x.max(1), y.max(1)),
        }
    }

    fn sixel_quant_width(&self) -> usize {
        let display_w = self.sixel_display_width();
        let (px_x, _) = self.sixel_quant_px_per_cell();
        ((display_w as f32 * px_x as f32) / SIXEL_REFERENCE_PX_PER_CELL_X as f32)
            .round()
            .max(1.0) as usize
    }

    fn sixel_quant_height(&self) -> usize {
        let display_h = self.sixel_display_height();
        let (_, px_y) = self.sixel_quant_px_per_cell();
        ((display_h as f32 * px_y as f32) / SIXEL_REFERENCE_PX_PER_CELL_Y as f32)
            .round()
            .max(1.0) as usize
    }

    fn sixel_display_width(&self) -> usize {
        self.width as usize * SIXEL_REFERENCE_PX_PER_CELL_X as usize
    }

    /// Logical sixel content height in pixels (before band alignment).
    fn sixel_raw_height(&self) -> usize {
        self.sixel_rows() as usize * SIXEL_REFERENCE_PX_PER_CELL_Y as usize
    }

    /// Content height for upsample/quantette. Zellij: trim to sixel band; elsewhere: full height.
    fn sixel_display_height(&self) -> usize {
        let raw = self.sixel_raw_height();
        if Self::in_zellij() {
            raw - raw % 6
        } else {
            raw
        }
    }

    /// Wire encode height. Zellij: same as display; elsewhere: pad up to sixel band with black.
    fn sixel_encode_height(&self) -> usize {
        let display = self.sixel_display_height();
        if Self::in_zellij() {
            display
        } else {
            display.div_ceil(6) * 6
        }
    }

    /// Terminal rows covered by sixel output (Zellij: leave bottom row for pane chrome).
    pub(crate) fn sixel_rows(&self) -> u16 {
        if Self::in_zellij() {
            self.height.saturating_sub(1)
        } else {
            self.height
        }
    }

    fn in_zellij() -> bool {
        std::env::var_os("ZELLIJ").is_some()
    }

    pub fn new(image: bool, model_setting: ModelSetting, base_dir: &str, camera: bool, tracker: Tracker) -> Self {
        Self {
            width: 0,
            height: 0,
            pixel_buffer: vec![],
            sixel_scratch: vec![],
            half_block_scratch: vec![],
            image,
            base_dir: base_dir.into(),
            model_setting,
            motion_list_state: ListState::default().with_selected(Some(0)),
            param_list_state: ListState::default().with_selected(Some(0)),
            camera_offset: 0,
            context_offset: 0,
            current_op_panel: OpPanel::None,
            current_debug_panel: DebugPanel::None,
            current_panel: Panel::None,
            live_setting: None,
            action_queue: vec![],
            active_expressions: HashMap::new(),
            tracker,
            camera,
            receiver: None,
            msg_chan: channel(),
            use_physics: false,
            popups: Popups::new(),
            image_protocol: ImageProtocol::HalfBlock,
            sixel_resolution: SixelResolution::Scale(1.0),
            mouse: false,
            bg_color: (0, 0, 0, 0),
        }
    }

    pub fn set_live_setting(&mut self, live: Live) {
        self.live_setting = Some(live);
    }

    pub fn set_pixel_color(&mut self, x: u16, y: u16, r: u8, g: u8, b: u8, a: u8) {
        let rw = self.render_width();
        let rh = self.render_height();
        if x < rw && y < rh {
            let idx = (y as usize) * (rw as usize) + (x as usize);
            self.pixel_buffer[idx] = [r, g, b, a];
        }
    }

    pub fn get_pixel_color(&self, x: u16, y: u16) -> (u8, u8, u8, u8) {
        let rw = self.render_width();
        let rh = self.render_height();
        if x < rw && y < rh {
            let idx = (y as usize) * (rw as usize) + (x as usize);
            let [r, g, b, a] = self.pixel_buffer[idx];
            (r, g, b, a)
        } else {
            (0, 0, 0, 0)
        }
    }

    pub fn update(&mut self) -> Result<bool, Box<dyn Error>> {
        let (tw, th) = terminal::size()?;
        let resized = self.width != tw || self.height != th;
        if resized {
            self.width = tw;
            self.height = th;
        }
        let rw = self.render_width() as usize;
        let rh = self.render_height() as usize;
        if self.pixel_buffer.len() != rw * rh {
            let bg = [self.bg_color.0, self.bg_color.1, self.bg_color.2, self.bg_color.3];
            self.pixel_buffer.resize(rw * rh, bg);
        }
        Ok(resized)
    }

    pub fn clear(&mut self) {
        let bg = [self.bg_color.0, self.bg_color.1, self.bg_color.2, self.bg_color.3];
        self.pixel_buffer.fill(bg);
    }

    /// True when motion/debug panels or popups need ratatui overlay rendering.
    pub fn has_overlay_ui(&self) -> bool {
        !matches!(self.current_op_panel, OpPanel::None)
            || !matches!(self.current_debug_panel, DebugPanel::None)
            || !self.popups.inner.is_empty()
    }

    /// Write half-block pixels directly to the terminal as batched ANSI,
    /// bypassing ratatui's per-cell buffer diff.
    pub fn write_half_block(&mut self, out: &mut impl Write) -> std::io::Result<()> {
        let w = self.width as usize;
        let ph = self.pixel_height() as usize;
        let cells_h = self.height as usize;

        self.half_block_scratch.clear();
        let buf = &mut self.half_block_scratch;

        let pb = &self.pixel_buffer;

        let mut cur_fg: Option<[u8; 3]> = None;
        let mut cur_bg: Option<[u8; 3]> = None;
        let mut plain_space = false;

        for cell_y in 0..cells_h {
            // Use CUP per row instead of \n — on Linux/WSL LF does not
            // carriage-return, so \n leaves the cursor mid-line and breaks alignment.
            push_cursor(buf, cell_y + 1, 1);
            let top_row = cell_y * 2;
            let bot_row = top_row + 1;

            for x in 0..w {
                let top_idx = top_row * w + x;
                let [tr, tg, tb, ta] = pb[top_idx];
                let [br, bg, bb, ba] = if bot_row < ph {
                    pb[bot_row * w + x]
                } else {
                    [0, 0, 0, 0]
                };

                if ta == 0 && ba == 0 {
                    if !plain_space || cur_fg.is_some() || cur_bg.is_some() {
                        buf.extend_from_slice(b"\x1b[0m");
                        cur_fg = None;
                        cur_bg = None;
                        plain_space = true;
                    }
                    buf.push(b' ');
                } else {
                    plain_space = false;
                    let fg = [tr, tg, tb];
                    let bg_rgb = [br, bg, bb];
                    if cur_fg != Some(fg) {
                        push_true_color(buf, b"38", tr, tg, tb);
                        cur_fg = Some(fg);
                    }
                    if cur_bg != Some(bg_rgb) {
                        push_true_color(buf, b"48", br, bg, bb);
                        cur_bg = Some(bg_rgb);
                    }
                    buf.extend_from_slice(b"\xE2\x96\x80"); // ▀
                }
            }
        }

        buf.extend_from_slice(b"\x1b[0m");
        out.write_all(buf)?;
        out.flush()
    }

    pub fn buffer_to_text(&self) -> Text<'static> {
        let ph = self.pixel_height() as usize;
        let w = self.width as usize;
        let mut lines = Vec::with_capacity(self.height as usize);

        for cell_y in 0..self.height as usize {
            let top_row = cell_y * 2;
            let bot_row = top_row + 1;
            let mut spans = Vec::with_capacity(w);
            for x in 0..w {
                let top_idx = top_row * w + x;
                let [tr, tg, tb, ta] = self.pixel_buffer.get(top_idx).copied().unwrap_or([0, 0, 0, 0]);
                let [br, bg, bb, ba] = if bot_row < ph {
                    self.pixel_buffer.get(bot_row * w + x).copied().unwrap_or([0, 0, 0, 0])
                } else {
                    [0, 0, 0, 0]
                };
                if ta == 0 && ba == 0 {
                    // Fully transparent cell — emit a plain space with no color codes.
                    spans.push(Span::raw(" "));
                } else {
                    spans.push(Span::styled(
                        "▀",
                        Style::default()
                            .fg(RatatuiColor::Rgb(tr, tg, tb))
                            .bg(RatatuiColor::Rgb(br, bg, bb)),
                    ));
                }
            }
            lines.push(Line::from(spans));
        }
        Text::from(lines)
    }

    pub fn buffer_to_sixel(&mut self) -> Vec<u8> {
        let quant_w = self.sixel_quant_width();
        let quant_h = self.sixel_quant_height();
        let display_w = self.sixel_display_width();
        let display_h = self.sixel_display_height();
        let encode_h = self.sixel_encode_height();
        self.fill_sixel_scratch(quant_w, quant_h);

        let rgba = std::mem::take(&mut self.sixel_scratch);
        crate::sixel_encode::encode_rgba_at_display_size(
            rgba,
            quant_w,
            quant_h,
            display_w,
            display_h,
            encode_h,
            &sixel_encode_options(),
        )
        .unwrap_or_default()
    }

    /// Downsample reference raster into sixel_scratch at quantette resolution.
    fn fill_sixel_scratch(&mut self, dst_w: usize, dst_h: usize) {
        let src_w = self.render_width() as usize;
        let src_h = self.render_height() as usize;
        let len = dst_w * dst_h * 4;

        if self.sixel_scratch.len() != len {
            self.sixel_scratch.resize(len, 255);
        }

        let pb = &self.pixel_buffer;
        if dst_w == src_w && dst_h == src_h {
            for (chunk, [r, g, b, _]) in self.sixel_scratch.chunks_mut(4).zip(pb.iter()) {
                chunk[0] = *r;
                chunk[1] = *g;
                chunk[2] = *b;
            }
            return;
        }

        for dy in 0..dst_h {
            let sy = dy * src_h / dst_h;
            let src_row = sy * src_w;
            let out_row = dy * dst_w * 4;
            for dx in 0..dst_w {
                let sx = dx * src_w / dst_w;
                let [r, g, b, _] = pb[src_row + sx];
                let o = out_row + dx * 4;
                self.sixel_scratch[o] = r;
                self.sixel_scratch[o + 1] = g;
                self.sixel_scratch[o + 2] = b;
            }
        }
    }

    pub fn buffer_to_kitty(&self) -> Vec<u8> {
        use kitty_graphics_protocol::{Command, Action, ImageFormat};

        let prev_id = KITTY_IMAGE_ID.load(Ordering::Relaxed);
        let curr_id = if prev_id >= 4096 { 1 } else { prev_id + 1 };
        KITTY_IMAGE_ID.store(curr_id, Ordering::Relaxed);

        let w = self.render_width() as u32;
        let h = self.render_height() as u32;

        // Zero-copy reinterpretation of [u8; 4] pixel buffer as &[u8].
        // Safe because [u8; 4] is guaranteed contiguous with no padding.
        let rgba: &[u8] = bytemuck::cast_slice(&self.pixel_buffer);

        let cmd = Command::builder()
            .action(Action::TransmitAndDisplay)
            .format(ImageFormat::Rgba)
            .dimensions(w, h)
            .display_area(self.width as u32, self.height as u32)
            .image_id(curr_id)
            .quiet(2)
            .build();

        let mut out: Vec<u8> = cmd.serialize_chunked(rgba)
            .map(|chunks| chunks.flat_map(|s| s.into_bytes()).collect())
            .unwrap_or_default();

        // Delete PREVIOUS id after new frame is already in the sequence.
        if let Ok(del) = Command::delete_by_id(prev_id).serialize(&[]) {
            out.extend(del.into_bytes());
        }

        out
    }

    pub fn get_active_expressions(&self) -> Vec<&str> {
        self.active_expressions.keys().map(|s| s.as_str()).collect()
    }
}

/// Sixel encoder settings tuned for live animation (quantette is the hot path).
fn sixel_encode_options() -> EncodeOptions {
    EncodeOptions {
        max_colors: 256,
        diffusion: 0.0,
        quantize_method: QuantizeMethod::Wu,
    }
}

fn push_true_color(buf: &mut Vec<u8>, prefix: &[u8; 2], r: u8, g: u8, b: u8) {
    buf.extend_from_slice(b"\x1b[");
    buf.extend_from_slice(prefix);
    buf.extend_from_slice(b";2;");
    push_u8(buf, r);
    buf.push(b';');
    push_u8(buf, g);
    buf.push(b';');
    push_u8(buf, b);
    buf.push(b'm');
}

/// `\x1b[{row};{col}H` — 1-indexed cursor position (CUP).
fn push_cursor(buf: &mut Vec<u8>, row: usize, col: usize) {
    buf.extend_from_slice(b"\x1b[");
    push_usize(buf, row);
    buf.push(b';');
    push_usize(buf, col);
    buf.push(b'H');
}

fn push_usize(buf: &mut Vec<u8>, n: usize) {
    if n == 0 {
        buf.push(b'0');
        return;
    }
    let mut tmp = [0u8; 20];
    let mut i = 20;
    let mut v = n;
    while v > 0 {
        i -= 1;
        tmp[i] = b'0' + (v % 10) as u8;
        v /= 10;
    }
    buf.extend_from_slice(&tmp[i..]);
}

fn push_u8(buf: &mut Vec<u8>, n: u8) {
    if n >= 100 {
        buf.push(b'0' + n / 100);
        buf.push(b'0' + (n / 10) % 10);
        buf.push(b'0' + n % 10);
    } else if n >= 10 {
        buf.push(b'0' + n / 10);
        buf.push(b'0' + n % 10);
    } else {
        buf.push(b'0' + n);
    }
}
