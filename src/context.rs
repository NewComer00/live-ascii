use std::sync::atomic::{AtomicU32, Ordering};
use std::collections::HashMap;
use std::error::Error;
use std::sync::{
    Arc,
    mpsc::{Sender, Receiver, channel}
};

use ratatui::widgets::ListState;

use crossterm::terminal;
use ratatui::style::{Color as RatatuiColor, Style};
use ratatui::text::{Line, Span, Text};

use crate::live::json::*;
use crate::tracker::*;
use crate::model_setting::ModelSetting;
use crate::ui::popup::*;
use crate::receiver::*;

/// Global atomic counter for Kitty image IDs, preventing too many resident images in the terminal.
static KITTY_IMAGE_ID: AtomicU32 = AtomicU32::new(1);

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
    // Per-pixel color buffer at width × (height * 2) resolution
    pub pixel_buffer: Vec<(u8, u8, u8, u8)>,
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
    pub mouse: bool,
    pub bg_color: (u8, u8, u8, u8),
}

impl Context {
    pub fn pixel_height(&self) -> u16 {
        self.height * 2
    }

    /// Width in pixels for rasterization target.
    /// Sixel / Kitty modes scale to terminal pixel size (10 px/cell);
    /// half-block uses terminal cells × 1.
    pub fn render_width(&self) -> u16 {
        if self.image_protocol == ImageProtocol::Sixel
            || self.image_protocol == ImageProtocol::Kitty
        {
            self.width * 10
        } else {
            self.width
        }
    }

    /// Height in pixels for rasterization target.
    /// Sixel / Kitty modes scale to terminal pixel size (20 px/cell);
    /// half-block uses cells × 2.
    pub fn render_height(&self) -> u16 {
        if self.image_protocol == ImageProtocol::Sixel
            || self.image_protocol == ImageProtocol::Kitty
        {
            self.height * 20
        } else {
            self.pixel_height()
        }
    }

    pub fn new(image: bool, model_setting: ModelSetting, base_dir: &str, camera: bool, tracker: Tracker) -> Self {
        Self {
            width: 0,
            height: 0,
            pixel_buffer: vec![],
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
            self.pixel_buffer[idx] = (r, g, b, a);
        }
    }

    pub fn get_pixel_color(&self, x: u16, y: u16) -> (u8, u8, u8, u8) {
        let rw = self.render_width();
        let rh = self.render_height();
        if x < rw && y < rh {
            let idx = (y as usize) * (rw as usize) + (x as usize);
            self.pixel_buffer[idx]
        } else {
            (0, 0, 0, 0)
        }
    }

    pub fn update(&mut self) -> Result<(), Box<dyn Error>> {
        let (tw, th) = terminal::size()?;
        if self.width != tw || self.height != th {
            self.width = tw;
            self.height = th;
        }
        let rw = self.render_width() as usize;
        let rh = self.render_height() as usize;
        if self.pixel_buffer.len() != rw * rh {
            self.pixel_buffer.resize(rw * rh, self.bg_color);
        }
        Ok(())
    }

    pub fn clear(&mut self) {
        self.pixel_buffer.fill(self.bg_color);
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
                let (tr, tg, tb, ta) = self.pixel_buffer.get(top_idx).copied().unwrap_or((0, 0, 0, 0));
                let (br, bg, bb, ba) = if bot_row < ph {
                    self.pixel_buffer.get(bot_row * w + x).copied().unwrap_or((0, 0, 0, 0))
                } else {
                    (0, 0, 0, 0)
                };
                if ta == 0 && ba == 0 {
                    spans.push(Span::styled(
                        " ".to_string(),
                        Style::default(),
                    ));
                } else {
                    spans.push(Span::styled(
                        "▀".to_string(),
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

    pub fn buffer_to_sixel(&self) -> Vec<u8> {
        let w = self.render_width() as usize;
        let h = self.render_height() as usize;
        let mut rgba: Vec<u8> = Vec::with_capacity(w * h * 4);
        for i in 0..self.pixel_buffer.len() {
            let (r, g, b, _a) = self.pixel_buffer[i];
            // Always opaque in sixel to avoid previous frames bleeding through;
            // transparency is handled by the other protocols.
            rgba.extend_from_slice(&[r, g, b, 255]);
        }
        icy_sixel::SixelImage::try_from_rgba(rgba, w, h)
            .ok()
            .and_then(|img| img.encode().ok())
            .unwrap_or_default()
            .into_bytes()
    }

    pub fn buffer_to_kitty(&self) -> Vec<u8> {
        use kitty_graphics_protocol::{Command, Action, ImageFormat};

        let prev_id = KITTY_IMAGE_ID.load(Ordering::Relaxed);
        let curr_id = if prev_id >= 255 { 1 } else { prev_id + 1 };
        KITTY_IMAGE_ID.store(curr_id, Ordering::Relaxed);

        let w = self.render_width() as u32;
        let h = self.render_height() as u32;
        let mut rgba = Vec::with_capacity((w * h * 4) as usize);
        for (r, g, b, a) in &self.pixel_buffer {
            rgba.extend_from_slice(&[*r, *g, *b, *a]);
        }

        let cmd = Command::builder()
            .action(Action::TransmitAndDisplay)
            .format(ImageFormat::Rgba)
            .dimensions(w, h)
            .display_area(self.width as u32, self.height as u32)
            .image_id(curr_id)
            .quiet(2)
            .build();

        let mut out: Vec<u8> = cmd.serialize_chunked(&rgba)
            .map(|chunks| chunks.flat_map(|s| s.into_bytes()).collect())
            .unwrap_or_default();

        // delete PREVIOUS id after new frame is already in the sequence
        if let Ok(del) = Command::delete_by_id(prev_id).serialize(&[]) {
            out.extend(del.into_bytes());
        }

        out
    }

    pub fn get_active_expressions(&self) -> Vec<&str> {
        self.active_expressions.keys().map(|s| s.as_str()).collect()
    }

}
