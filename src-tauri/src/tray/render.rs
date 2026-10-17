#![cfg_attr(not(windows), allow(dead_code))]
//! Runtime-rendered tray tile: brand-red rounded square with the current
//! value in white (Windows has no text next to tray icons).

use fontdue::{Font, FontSettings};

static FONT: &[u8] = include_bytes!("../../assets/fonts/BarlowCondensed-Bold.ttf");

pub const BRAND_RED: [u8; 3] = [0xE4, 0x00, 0x2B];
pub const GREY: [u8; 3] = [0x6B, 0x6A, 0x6A];

pub struct TileRenderer {
    font: Font,
}

/// Coverage of a rounded square of side `s` and corner radius `r` at (x, y).
fn coverage(x: f32, y: f32, s: f32, r: f32) -> f32 {
    let half = s / 2.0;
    let qx = (x - half).abs() - (half - r);
    let qy = (y - half).abs() - (half - r);
    let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
    let inside = qx.max(qy).min(0.0);
    (0.5 - (outside + inside - r)).clamp(0.0, 1.0)
}

impl TileRenderer {
    pub fn new() -> Option<Self> {
        Font::from_bytes(FONT, FontSettings::default())
            .ok()
            .map(|font| Self { font })
    }

    fn text_width(&self, text: &str, px: f32) -> f32 {
        text.chars()
            .map(|c| self.font.metrics(c, px).advance_width)
            .sum()
    }

    /// RGBA pixels (straight alpha) of a `size`x`size` tile.
    pub fn render(&self, text: &str, size: u32, bg: [u8; 3]) -> Vec<u8> {
        let s = size as f32;
        let mut px = vec![0u8; (size * size * 4) as usize];
        let radius = s * 0.22;
        for y in 0..size {
            for x in 0..size {
                let c = coverage(x as f32 + 0.5, y as f32 + 0.5, s, radius);
                let i = ((y * size + x) * 4) as usize;
                px[i..i + 3].copy_from_slice(&bg);
                px[i + 3] = (c * 255.0).round() as u8;
            }
        }

        // Largest size whose text fits the tile.
        let max_w = s * 0.92;
        let mut font_px = s * 0.95;
        while font_px > 6.0 && self.text_width(text, font_px) > max_w {
            font_px -= 0.5;
        }
        let width = self.text_width(text, font_px);
        let digit_h = self.font.metrics('0', font_px).height as f32;
        let baseline = ((s + digit_h) / 2.0).round();
        let mut pen = (s - width) / 2.0;

        for ch in text.chars() {
            let (m, bitmap) = self.font.rasterize(ch, font_px);
            let gx = (pen + m.xmin as f32).round() as i32;
            let gy = (baseline - m.height as f32 - m.ymin as f32) as i32;
            for row in 0..m.height {
                for col in 0..m.width {
                    let a = f32::from(bitmap[row * m.width + col]) / 255.0;
                    if a <= 0.0 {
                        continue;
                    }
                    let x = gx + col as i32;
                    let y = gy + row as i32;
                    if x < 0 || y < 0 || x >= size as i32 || y >= size as i32 {
                        continue;
                    }
                    let i = ((y as u32 * size + x as u32) * 4) as usize;
                    for k in 0..3 {
                        let d = f32::from(px[i + k]);
                        px[i + k] = (d + (255.0 - d) * a).round() as u8;
                    }
                    let da = f32::from(px[i + 3]);
                    px[i + 3] = (da + (255.0 - da) * a).round() as u8;
                }
            }
            pen += m.advance_width;
        }
        px
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_tile() {
        let Some(r) = TileRenderer::new() else {
            panic!("font failed to load");
        };
        for size in [16, 24, 32] {
            let px = r.render("23", size, BRAND_RED);
            assert_eq!(px.len(), (size * size * 4) as usize);
            // Centre pixel area should contain some white text pixels.
            let white = px.chunks(4).filter(|p| p[0] > 200 && p[1] > 200).count();
            assert!(white > 0, "no text pixels at {size}");
            // Corners are transparent.
            assert_eq!(px[3], 0);
        }
    }
}
