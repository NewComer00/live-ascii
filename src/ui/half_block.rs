use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::widgets::Widget;

use crate::context::Context;

/// Renders `pixel_buffer` as half-block (▀) terminal cells, writing directly
/// into the ratatui buffer instead of building a Paragraph with per-cell Spans.
pub struct HalfBlockImage<'a> {
    context: &'a Context,
}

impl<'a> HalfBlockImage<'a> {
    pub fn new(context: &'a Context) -> Self {
        Self { context }
    }
}

impl Widget for HalfBlockImage<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let w = self.context.width as usize;
        let ph = self.context.pixel_height() as usize;
        let cells_h = self.context.height as usize;
        let pb = &self.context.pixel_buffer;

        let render_h = (area.height as usize).min(cells_h);
        let render_w = (area.width as usize).min(w);

        for cell_y in 0..render_h {
            let top_row = cell_y * 2;
            let bot_row = top_row + 1;
            let buf_y = area.y + cell_y as u16;

            for x in 0..render_w {
                let top_idx = top_row * w + x;
                let [tr, tg, tb, ta] = pb[top_idx];
                let [br, bg, bb, ba] = if bot_row < ph {
                    pb[bot_row * w + x]
                } else {
                    [0, 0, 0, 0]
                };

                let cell = &mut buf[(area.x + x as u16, buf_y)];
                if ta == 0 && ba == 0 {
                    cell.reset();
                    cell.set_symbol(" ");
                } else {
                    cell.set_symbol("▀")
                        .set_fg(Color::Rgb(tr, tg, tb))
                        .set_bg(Color::Rgb(br, bg, bb));
                }
            }
        }
    }
}
