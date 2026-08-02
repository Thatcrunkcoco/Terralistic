use anyhow::Result;

use crate::libraries::graphics as gfx;
use ab_glyph::{point, Font, FontRef, Glyph, PxScale, ScaleFont};

/// A true-type font renderer. Unlike [`crate::libraries::graphics::Font`] (which
/// can only display a low-res 16x16 pixel atlas), this renders any real TTF/OTF
/// font at arbitrary pixel sizes by rasterizing glyphs with `ab_glyph`.
pub struct TtfFont {
    font: FontRef<'static>,
    px_scale: PxScale,
    // Factor to convert the rasterized (px_scale) size to the intended
    // on-screen size. 1.0 renders at native size; <1.0 downsamples the hi-res
    // glyphs for crisper small text.
    display_ratio: f32,
}

impl TtfFont {
    /// Loads a font from raw TTF/OTF bytes.
    ///
    /// `display_size` is the intended on-screen pixel height. `supersample`
    /// (`>= 1`) rasterizes at `display_size * supersample` resolution so the
    /// scaled-down result stays sharp.
    pub fn new(font_data: &'static [u8], display_size: f32, supersample: f32) -> Result<Self> {
        let font = FontRef::try_from_slice(font_data)?;
        Ok(Self {
            px_scale: PxScale::from(display_size * supersample),
            font,
            display_ratio: 1.0 / supersample,
        })
    }

    /// The scale factor to apply when rendering so the text appears at
    /// `display_size` on screen (independent of the UI zoom).
    pub const fn display_ratio(&self) -> f32 {
        self.display_ratio
    }

    /// The on-screen pixel height of one line of text.
    pub fn line_height(&self) -> f32 {
        self.font.as_scaled(self.px_scale).height() * self.display_ratio
    }

    /// Lays out the glyphs of `text`, handling newlines.
    fn layout(&self, text: &str) -> Vec<Glyph> {
        let scale_font = self.font.as_scaled(self.px_scale);
        let mut glyphs = Vec::new();
        let mut pen_x = 0.0f32;
        let mut pen_y = 0.0f32;
        for c in text.chars() {
            if c == '\n' {
                pen_y += scale_font.height();
                pen_x = 0.0;
                continue;
            }
            let glyph_id = scale_font.glyph_id(c);
            glyphs.push(Glyph {
                id: glyph_id,
                scale: self.px_scale,
                position: point(pen_x, pen_y),
            });
            pen_x += scale_font.h_advance(glyph_id);
        }
        glyphs
    }

    /// Returns the pixel dimensions the given text would occupy when rendered.
    pub fn measure(&self, text: &str) -> gfx::IntSize {
        let (_, min_x, min_y, max_x, max_y) = self.text_bounds(text);
        gfx::IntSize((max_x - min_x).max(1.0) as u32, (max_y - min_y).max(1.0) as u32)
    }

    /// Renders `text` into a [`gfx::Surface`] at the font's pixel size.
    ///
    /// The surface's pixels are white with the glyph coverage in the alpha
    /// channel, so it can be tinted via the texture color when drawn. Glyphs
    /// are baseline-aligned within a fixed line height, so the baseline stays
    /// constant no matter which characters (short or tall) are rendered.
    pub fn render_text(&self, text: &str) -> gfx::Surface {
        let scale_font = self.font.as_scaled(self.px_scale);
        let ascent = scale_font.ascent();
        // `\n` advances the pen by the full line height in `layout`, so size the
        // surface for as many lines as the text has (otherwise multi-line text
        // like /help output would be clipped to only its first line).
        let line_step = scale_font.height();
        let num_lines = text.chars().filter(|c| *c == '\n').count() as f32 + 1.0;
        let height = (num_lines * line_step).ceil().max(1.0) as u32;
        let (_, min_x, _, max_x, _) = self.text_bounds(text);
        let width = (max_x - min_x).max(1.0) as u32;
        let size = gfx::IntSize(width, height);
        let mut surface = gfx::Surface::new(size);

        for glyph in self.layout(text) {
            if let Some(outlined) = scale_font.outline_glyph(glyph) {
                let bounds = outlined.px_bounds();
                // `draw` gives coordinates local to this glyph's pixel bounds.
                // Offset so the baseline of every glyph lands on the same row.
                let offset_x = (bounds.min.x - min_x) as i32;
                let offset_y = (ascent + bounds.min.y) as i32;
                outlined.draw(|x, y, coverage| {
                    let px = offset_x + x as i32;
                    let py = offset_y + y as i32;
                    if px >= 0 && py >= 0 && (px as u32) < size.0 && (py as u32) < size.1 {
                        if let Ok(pixel) = surface.get_pixel_mut(gfx::IntPos(px, py)) {
                            let alpha = (coverage * 255.0) as u8;
                            if alpha > pixel.a {
                                *pixel = gfx::Color::new(255, 255, 255, alpha);
                            }
                        }
                    }
                });
            }
        }

        surface
    }

    /// Returns the axis-aligned pixel bounding box of the rendered text:
    /// `(has_content, min_x, min_y, max_x, max_y)`.
    ///
    /// Glyphs without ink (e.g. spaces) still extend the width by their
    /// horizontal advance, so the measured bounds match the cursor/placement.
    fn text_bounds(&self, text: &str) -> (bool, f32, f32, f32, f32) {
        let scale_font = self.font.as_scaled(self.px_scale);
        let mut max_x = 0.0f32;
        let mut max_y = 0.0f32;
        let mut min_x = 0.0f32;
        let mut min_y = 0.0f32;
        let mut min_set = false;

        for glyph in self.layout(text) {
            let position = glyph.position;
            let id = glyph.id;
            if let Some(outlined) = scale_font.outline_glyph(glyph) {
                let bounds = outlined.px_bounds();
                if !min_set {
                    min_x = bounds.min.x;
                    min_y = bounds.min.y;
                    min_set = true;
                }
                min_x = min_x.min(bounds.min.x);
                min_y = min_y.min(bounds.min.y);
                max_x = max_x.max(bounds.max.x);
                max_y = max_y.max(bounds.max.y);
            } else {
                // advance-only glyph (e.g. space): extend the right edge
                let right = position.x + scale_font.h_advance(id);
                max_x = max_x.max(right);
            }
        }

        if !min_set {
            // text with no ink at all (empty or whitespace-only)
            return (false, 0.0, 0.0, max_x.max(1.0), 1.0);
        }

        (true, min_x, min_y, max_x, max_y)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FONT: &[u8] = include_bytes!("../../Build/Resources/terminal_font.ttf");

    fn make_font() -> TtfFont {
        TtfFont::new(FONT, 28.0, 2.0).expect("font should load")
    }

    #[test]
    fn rasterizes_text_with_visibility() {
        let font = make_font();
        let surface = font.render_text("hello");
        let size = surface.get_size();
        assert!(size.0 > 0 && size.1 > 0, "surface should be non-empty");

        // at least one non-transparent pixel should be drawn
        let mut non_transparent = 0;
        for (_, color) in surface.iter() {
            if color.a > 0 {
                non_transparent += 1;
            }
        }
        assert!(non_transparent > 0, "text should have visible pixels");
    }

    #[test]
    fn mono_is_wider_than_short_word() {
        let font = make_font();
        let width_hello = font.measure("hello").0;
        let width_w = font.measure("w").0;
        assert!(width_hello > width_w, "longer text should be measured wider");
    }

    #[test]
    fn newline_increases_height() {
        let font = make_font();
        let single = font.measure("hi").1;
        let double = font.measure("hi\nhi").1;
        assert!(double > single, "newline should increase measured height");
    }

    #[test]
    fn glyphs_are_not_stacked_at_origin() {
        let font = make_font();
        let surface = font.render_text("hi");
        let width = surface.get_size().0;
        let height = surface.get_size().1;

        let mut has_right_side = false;
        for y in 0..height {
            for x in (width * 2 / 3)..width {
                if let Ok(color) = surface.get_pixel(gfx::IntPos(x as i32, y as i32)) {
                    if color.a > 0 {
                        has_right_side = true;
                    }
                }
            }
        }
        assert!(has_right_side, "second glyph should be drawn towards the right, not stacked at origin");
    }

    #[test]
    fn display_ratio_scales_raster_up() {
        // comparing single-supersample vs 2x: the 2x rasterizes at 2x resolution
        let normal = TtfFont::new(FONT, 28.0, 1.0).unwrap().render_text("A");
        let supersampled = TtfFont::new(FONT, 28.0, 2.0).unwrap().render_text("A");
        let normal_h = normal.get_size().1;
        let super_h = supersampled.get_size().1;
        assert!(super_h > normal_h, "2x supersample should rasterize glyphs larger than 1x");
        assert!((TtfFont::new(FONT, 28.0, 2.0).unwrap().display_ratio() - 0.5).abs() < 1e-4, "2x supersample -> display ratio of 0.5");
    }
}
