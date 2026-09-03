//! Poster thumbnails. Two paths:
//!  - kitty graphics protocol (Ghostty, kitty, WezTerm): the real image, crisp.
//!  - half-block cells through the frame diff pipeline: works everywhere.

use base64::Engine;
use ftui::layout::Rect;
use ftui::widgets::Widget;
use ftui::{Frame, PackedRgba};
use ftui_extras::canvas::{CanvasRef, Mode, Painter};
use std::io::Write;

/// Cell footprint the poster is prepared for. Posters are 2:3, so 24x36 px = 24x18 cells.
pub const CELL_W: u16 = 24;
pub const CELL_H: u16 = 18;

#[derive(Debug, Clone)]
pub struct Poster {
    /// Half-block thumbnail (height in pixels, two per row).
    pub width: u16,
    pub height: u16,
    pub pixels: Vec<[u8; 3]>,
    /// PNG for the graphics protocol path.
    pub png: Vec<u8>,
}

impl Poster {
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        let img = image::load_from_memory(bytes).ok()?;
        let (w, h) = (CELL_W as u32, CELL_H as u32 * 2);
        let rgb = img.resize_to_fill(w, h, image::imageops::FilterType::Triangle).to_rgb8();
        let pixels = rgb.pixels().map(|p| [p[0], p[1], p[2]]).collect();
        let mut png = Vec::new();
        let full = img.resize(360, 540, image::imageops::FilterType::CatmullRom).to_rgb8();
        let _ = image::DynamicImage::ImageRgb8(full).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png);
        Some(Self { width: w as u16, height: h as u16, pixels, png })
    }

    /// Half-block rendering into the frame buffer.
    pub fn render_blocks(&self, area: Rect, frame: &mut Frame) {
        if area.width == 0 || area.height == 0 { return; }
        let area = Rect::new(area.x, area.y, area.width.min(self.width), area.height.min(self.height / 2));
        let mut painter = Painter::for_area(area, Mode::HalfBlock);
        for y in 0..(area.height as u32 * 2).min(self.height as u32) {
            for x in 0..(area.width as u32).min(self.width as u32) {
                let p = self.pixels[(y * self.width as u32 + x) as usize];
                painter.point_colored(x as i32, y as i32, PackedRgba::rgb(p[0], p[1], p[2]));
            }
        }
        CanvasRef::from_painter(&painter).render(area, frame);
    }
}

/// Does this terminal speak the kitty graphics protocol? Env sniffing only; the
/// protocol has no cheap capability query we can run mid-frame.
pub fn kitty_supported() -> bool {
    if std::env::var_os("TMUX").is_some() { return false; }
    let term = std::env::var("TERM").unwrap_or_default().to_ascii_lowercase();
    let prog = std::env::var("TERM_PROGRAM").unwrap_or_default().to_ascii_lowercase();
    term.contains("kitty") || term.contains("ghostty") || prog.contains("ghostty") || prog.contains("wezterm")
        || std::env::var_os("KITTY_WINDOW_ID").is_some() || std::env::var_os("GHOSTTY_RESOURCES_DIR").is_some()
}

/// Owns the single on-screen poster placement. The runtime never touches the
/// graphics layer, so we write the escape sequences ourselves and restore the
/// cursor so the presenter's cursor tracking stays valid.
#[derive(Default)]
pub struct KittyPlacer {
    current: Option<(String, Rect)>,
    next_id: u32,
}

impl KittyPlacer {
    /// Show `poster` for `key` at `area` (cells). No-op when already placed there.
    pub fn place(&mut self, key: &str, poster: &Poster, area: Rect) {
        if area.width == 0 || area.height == 0 { return; }
        if let Some((k, r)) = &self.current { if k == key && *r == area { return; } }
        self.clear();
        self.next_id = self.next_id.wrapping_add(1).max(1);
        let id = self.next_id;
        let b64 = base64::engine::general_purpose::STANDARD.encode(&poster.png);
        let mut out = String::with_capacity(b64.len() + 256);
        out.push_str("\x1b7"); // save cursor
        out.push_str(&format!("\x1b[{};{}H", area.y + 1, area.x + 1));
        let chunks: Vec<&[u8]> = b64.as_bytes().chunks(4096).collect();
        for (i, chunk) in chunks.iter().enumerate() {
            let more = if i + 1 < chunks.len() { 1 } else { 0 };
            if i == 0 {
                out.push_str(&format!("\x1b_Ga=T,f=100,t=d,i={id},c={},r={},q=2,m={more};", area.width, area.height));
            } else {
                out.push_str(&format!("\x1b_Gm={more};"));
            }
            out.push_str(std::str::from_utf8(chunk).unwrap_or(""));
            out.push_str("\x1b\\");
        }
        out.push_str("\x1b8"); // restore cursor
        let mut so = std::io::stdout().lock();
        let _ = so.write_all(out.as_bytes());
        let _ = so.flush();
        self.current = Some((key.to_string(), area));
    }

    /// Remove the placement (and its data) if any.
    pub fn clear(&mut self) {
        if self.current.take().is_some() {
            let mut so = std::io::stdout().lock();
            let _ = so.write_all(b"\x1b_Ga=d,d=A,q=2\x1b\\");
            let _ = so.flush();
        }
    }

}
