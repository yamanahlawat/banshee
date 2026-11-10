//! The capsule's words on Linux: one line shaped from the bundled Archivo,
//! cut to fit, and returned as an outline in logical pixels.

use std::sync::OnceLock;

use harfrust::{Buffer, ShapeOptions, ShaperFont, font::Font};
use skrifa::{DrawMemory, GlyphId, PathSink, ScalerFont};
use tiny_skia::{Path, PathBuilder};

use crate::card::{self, Cut, Measure};
use crate::chip::Trim;
use crate::figure::{WORDS_SIZE, WORDS_TRACKING, WORDS_WEIGHT, WORDS_WIDTH};

const ARCHIVO: &[u8] = include_bytes!("../../../../assets/fonts/archivo-latin.ttf");

/// A shaped line. The outline's baseline sits `ascent` below its top edge.
pub struct Line {
    pub path: Option<Path>,
    pub width: f32,
    pub ascent: f32,
    pub descent: f32,
}

pub fn line(text: &str, trim: Trim, room: f32) -> Line {
    let font = capsule_font();
    let text = fit(font, text, trim, room);
    outline(font, &text, WORDS_SIZE as f32, WORDS_TRACKING as f32)
}

/// `text` set in `cut`, uncut, as an outline.
pub fn shaped(text: &str, cut: Cut) -> Line {
    outline(
        cut_font(cut),
        text,
        cut.size() as f32,
        cut.tracking() as f32,
    )
}

/// One Archivo instance per card cut.
pub struct Fonts;

impl Measure for Fonts {
    fn width(&self, text: &str, cut: Cut) -> f64 {
        f64::from(width_of(
            cut_font(cut),
            text,
            cut.size() as f32,
            cut.tracking() as f32,
        ))
    }

    fn narrowest(&self, cut: Cut) -> f64 {
        static NARROWEST: [OnceLock<f64>; Cut::ALL.len()] =
            [const { OnceLock::new() }; Cut::ALL.len()];
        *NARROWEST[cut as usize].get_or_init(|| card::narrowest_ascii(|one| self.width(one, cut)))
    }
}

fn cut_font(cut: Cut) -> &'static Font {
    static FONTS: [OnceLock<Font>; Cut::ALL.len()] = [const { OnceLock::new() }; Cut::ALL.len()];
    FONTS[cut as usize].get_or_init(|| font(cut.weight() as f32, cut.width() as f32))
}

/// `text` as an outline in `font`, `size` pixels high, with `tracking` em added to each glyph.
fn outline(font: &Font, text: &str, size: f32, tracking: f32) -> Line {
    let scale = scale_of(font, size);
    let metrics = font.metrics();
    let h_line = metrics.h_line().unwrap_or_default();
    let ascent = h_line.ascender.to_f32() * scale;
    let descent = -h_line.descender.to_f32() * scale;

    let scaler = ScalerFont::unhinted(font.clone(), None);
    let mut pen = Pen {
        builder: PathBuilder::new(),
        scale,
        ascent,
        x: 0.0,
    };
    for (glyph, advance) in shape(font, text, size, tracking) {
        if let Some(outline) = scaler.outline(glyph) {
            // A glyph that fails to draw leaves a gap; the advance still holds its place.
            let _ = outline.draw(DrawMemory::Scratch, &mut pen);
        }
        pen.x += advance;
    }
    Line {
        path: pen.builder.finish(),
        width: pen.x,
        ascent,
        descent,
    }
}

fn capsule_font() -> &'static Font {
    static FONT: OnceLock<Font> = OnceLock::new();
    FONT.get_or_init(|| font(WORDS_WEIGHT as f32, WORDS_WIDTH as f32))
}

/// The bundled Archivo at one weight and width. Shaping and outlines both read
/// this instance, so they share one variation location.
fn font(weight: f32, width: f32) -> Font {
    Font::new(ARCHIVO, 0)
        .expect("the bundled Archivo parses")
        .instance_builder()
        .variations([("wght", weight), ("wdth", width)])
        .build()
}

fn scale_of(font: &Font, size: f32) -> f32 {
    size / f32::from(font.units_per_em())
}

/// Each glyph the text shapes to, with its advance in pixels. A glyph the
/// font lacks is dropped.
fn shape(font: &Font, text: &str, size: f32, tracking: f32) -> Vec<(GlyphId, f32)> {
    let mut buffer = Buffer::new();
    buffer.push_str(text);
    buffer.guess_segment_properties();
    let shaper = ShaperFont::new(font);
    if harfrust::shape(&shaper, &mut buffer, ShapeOptions::new()).is_err() {
        return Vec::new();
    }
    let tracking = tracking * size;
    let scale = scale_of(font, size);
    buffer
        .glyph_infos()
        .iter()
        .zip(buffer.glyph_positions())
        .filter(|(info, _)| info.glyph_id != 0)
        .map(|(info, position)| {
            let id = GlyphId::new(info.glyph_id);
            (id, position.x_advance as f32 * scale + tracking)
        })
        .collect()
}

fn width_of(font: &Font, text: &str, size: f32, tracking: f32) -> f32 {
    shape(font, text, size, tracking)
        .iter()
        .map(|(_, advance)| advance)
        .sum()
}

/// The characters `font` has, cut to `room` pixels.
fn fit(font: &Font, text: &str, trim: Trim, room: f32) -> String {
    let drawn: String = text
        .chars()
        .filter(|&c| font.charmap().map_unicode(c).is_some())
        .collect();
    card::fit(&drawn, f64::from(room), trim, |line| {
        f64::from(width_of(
            font,
            line,
            WORDS_SIZE as f32,
            WORDS_TRACKING as f32,
        ))
    })
}

/// Collects a glyph's outline with the y axis flipped, so the baseline sits at
/// `ascent` and the path reads in tiny-skia's y-down space.
struct Pen {
    builder: PathBuilder,
    scale: f32,
    ascent: f32,
    x: f32,
}

impl Pen {
    fn at(&self, x: f32, y: f32) -> (f32, f32) {
        (self.x + x * self.scale, self.ascent - y * self.scale)
    }
}

impl PathSink for Pen {
    fn move_to(&mut self, x: f32, y: f32) {
        let (x, y) = self.at(x, y);
        self.builder.move_to(x, y);
    }

    fn line_to(&mut self, x: f32, y: f32) {
        let (x, y) = self.at(x, y);
        self.builder.line_to(x, y);
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let ((x1, y1), (x, y)) = (self.at(x1, y1), self.at(x, y));
        self.builder.quad_to(x1, y1, x, y);
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let ((x1, y1), (x2, y2), (x, y)) = (self.at(x1, y1), self.at(x2, y2), self.at(x, y));
        self.builder.cubic_to(x1, y1, x2, y2, x, y);
    }

    fn close(&mut self) {
        self.builder.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZE: f32 = WORDS_SIZE as f32;
    const TRACKING: f32 = WORDS_TRACKING as f32;
    const LONG: &str = "this sentence runs well past the room the capsule has for it";

    #[test]
    fn your_turn_shapes_one_glyph_per_letter() {
        let font = capsule_font();
        assert_eq!(shape(font, "Your turn", SIZE, TRACKING).len(), 9);
        assert!(width_of(font, "Your turn", SIZE, TRACKING) > 0.0);
    }

    #[test]
    fn a_long_live_line_keeps_its_newest_end() {
        let font = capsule_font();
        let kept = fit(font, LONG, Trim::Start, 100.0);
        let tail = kept
            .strip_prefix(card::ELLIPSIS)
            .expect("the cut starts with an ellipsis");
        assert!(tail.len() > 3 && LONG.ends_with(tail));
        assert!(line(LONG, Trim::Start, 100.0).width <= 100.0);
    }

    #[test]
    fn a_long_reason_keeps_its_start() {
        let font = capsule_font();
        let kept = fit(font, LONG, Trim::End, 100.0);
        let head = kept
            .strip_suffix(card::ELLIPSIS)
            .expect("the cut ends in an ellipsis");
        assert!(head.len() > 3 && LONG.starts_with(head));
        assert!(line(LONG, Trim::End, 100.0).width <= 100.0);
    }

    #[test]
    fn a_line_that_fits_is_left_whole() {
        let font = capsule_font();
        assert_eq!(fit(font, "Your turn", Trim::End, 1000.0), "Your turn");
    }

    #[test]
    fn the_width_axis_widens_the_line() {
        let default = font(WORDS_WEIGHT as f32, 100.0);
        assert!(
            width_of(capsule_font(), "Your turn", SIZE, TRACKING)
                > width_of(&default, "Your turn", SIZE, TRACKING)
        );
    }

    #[test]
    fn a_character_archivo_lacks_draws_no_box() {
        let with = line("ok\u{2713}", Trim::End, 1000.0);
        let without = line("ok", Trim::End, 1000.0);
        assert_eq!(with.width, without.width);
    }

    #[test]
    fn a_line_has_an_outline_below_its_top_edge() {
        let drawn = line("Your turn", Trim::End, 1000.0);
        let bounds = drawn.path.expect("letters have an outline").bounds();
        assert!(bounds.top() >= 0.0);
        assert!(bounds.bottom() <= drawn.ascent + drawn.descent);
        assert!(line("", Trim::End, 1000.0).path.is_none());
    }

    #[test]
    fn the_outline_spans_the_shaped_width() {
        let drawn = line("Your turn", Trim::End, 1000.0);
        let right = drawn
            .path
            .expect("letters have an outline")
            .bounds()
            .right();
        assert!(
            (drawn.width - right).abs() < 2.0,
            "{} vs {right}",
            drawn.width
        );
    }

    #[test]
    fn each_cut_measures_its_own_width() {
        let width = |cut| Fonts.width("Your turn", cut);
        assert!(width(Cut::Ask) > width(Cut::Context));
        assert!(width(Cut::User) > width(Cut::Context));
        let narrowest = Fonts.narrowest(Cut::Context);
        assert!(narrowest > 0.0 && narrowest <= Fonts.width("i", Cut::Context));
    }

    #[test]
    fn a_shaped_line_spans_its_measured_width() {
        let drawn = shaped("Your turn", Cut::Label);
        let measured = Fonts.width("Your turn", Cut::Label);
        assert!((f64::from(drawn.width) - measured).abs() < 0.01);
        let right = drawn
            .path
            .expect("letters have an outline")
            .bounds()
            .right();
        assert!(
            (drawn.width - right).abs() < 2.0,
            "{} vs {right}",
            drawn.width
        );
    }
}
