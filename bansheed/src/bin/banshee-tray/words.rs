//! The capsule's words on Linux: one line shaped from the bundled Archivo,
//! cut to fit, and returned as an outline in logical pixels.

use rustybuzz::{Face, UnicodeBuffer, Variation, ttf_parser};
use tiny_skia::{Path, PathBuilder};

use crate::chip::Cut;
use crate::figure::{WORDS_SIZE, WORDS_TRACKING, WORDS_WEIGHT, WORDS_WIDTH};

const ARCHIVO: &[u8] = include_bytes!("../../../../assets/fonts/archivo-latin.ttf");
const ELLIPSIS: char = '\u{2026}';

/// A shaped line. The outline's baseline sits `ascent` below its top edge.
pub struct Line {
    pub path: Option<Path>,
    pub width: f32,
    pub ascent: f32,
    pub descent: f32,
}

pub fn line(text: &str, cut: Cut, room: f32) -> Line {
    let face = face(WORDS_WEIGHT as f32, WORDS_WIDTH as f32);
    let scale = scale_of(&face);
    let ascent = f32::from(face.ascender()) * scale;
    let descent = -f32::from(face.descender()) * scale;
    let text = fit(&face, text, cut, room);

    let mut pen = Pen {
        builder: PathBuilder::new(),
        scale,
        ascent,
        x: 0.0,
    };
    for (glyph, advance) in shape(&face, &text) {
        face.outline_glyph(glyph, &mut pen);
        pen.x += advance;
    }
    Line {
        path: pen.builder.finish(),
        width: pen.x,
        ascent,
        descent,
    }
}

fn face(weight: f32, width: f32) -> Face<'static> {
    let mut face = Face::from_slice(ARCHIVO, 0).expect("the bundled Archivo parses");
    face.set_variations(&[
        Variation {
            tag: ttf_parser::Tag::from_bytes(b"wght"),
            value: weight,
        },
        Variation {
            tag: ttf_parser::Tag::from_bytes(b"wdth"),
            value: width,
        },
    ]);
    face
}

fn scale_of(face: &Face) -> f32 {
    WORDS_SIZE as f32 / face.units_per_em() as f32
}

/// Each glyph the text shapes to, with its advance in pixels. A glyph the
/// font lacks is dropped.
fn shape(face: &Face, text: &str) -> Vec<(ttf_parser::GlyphId, f32)> {
    let mut buffer = UnicodeBuffer::new();
    buffer.push_str(text);
    let shaped = rustybuzz::shape(face, &[], buffer);
    let tracking = (WORDS_TRACKING * WORDS_SIZE) as f32;
    let scale = scale_of(face);
    shaped
        .glyph_infos()
        .iter()
        .zip(shaped.glyph_positions())
        .filter(|(info, _)| info.glyph_id != 0)
        .map(|(info, position)| {
            let id = ttf_parser::GlyphId(info.glyph_id as u16);
            (id, position.x_advance as f32 * scale + tracking)
        })
        .collect()
}

fn width_of(face: &Face, text: &str) -> f32 {
    shape(face, text).iter().map(|(_, advance)| advance).sum()
}

/// The text cut to `room` pixels, with an ellipsis where the cut fell.
fn fit(face: &Face, text: &str, cut: Cut, room: f32) -> String {
    let chars: Vec<char> = text
        .chars()
        .filter(|&c| face.glyph_index(c).is_some())
        .collect();
    let whole: String = chars.iter().collect();
    if width_of(face, &whole) <= room {
        return whole;
    }
    let keeping = |count: usize| match cut {
        Cut::End => {
            let head: String = chars[..count].iter().collect();
            format!("{}{ELLIPSIS}", head.trim_end())
        }
        Cut::Start => {
            let tail: String = chars[chars.len() - count..].iter().collect();
            format!("{ELLIPSIS}{}", tail.trim_start())
        }
    };
    let (mut fits, mut over) = (0, chars.len());
    while over - fits > 1 {
        let middle = fits + (over - fits) / 2;
        if width_of(face, &keeping(middle)) <= room {
            fits = middle;
        } else {
            over = middle;
        }
    }
    keeping(fits)
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

impl ttf_parser::OutlineBuilder for Pen {
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

    fn capsule_face() -> Face<'static> {
        face(WORDS_WEIGHT as f32, WORDS_WIDTH as f32)
    }

    fn glyph_count(face: &Face, text: &str) -> usize {
        let mut buffer = UnicodeBuffer::new();
        buffer.push_str(text);
        rustybuzz::shape(face, &[], buffer).len()
    }

    const LONG: &str = "this sentence runs well past the room the capsule has for it";

    #[test]
    fn your_turn_shapes_one_glyph_per_letter() {
        let face = capsule_face();
        assert_eq!(glyph_count(&face, "Your turn"), 9);
        assert!(width_of(&face, "Your turn") > 0.0);
    }

    #[test]
    fn a_long_live_line_keeps_its_newest_end() {
        let face = capsule_face();
        let kept = fit(&face, LONG, Cut::Start, 100.0);
        let tail = kept
            .strip_prefix(ELLIPSIS)
            .expect("the cut starts with an ellipsis");
        assert!(tail.len() > 3 && LONG.ends_with(tail));
        assert!(line(LONG, Cut::Start, 100.0).width <= 100.0);
    }

    #[test]
    fn a_long_reason_keeps_its_start() {
        let face = capsule_face();
        let kept = fit(&face, LONG, Cut::End, 100.0);
        let head = kept
            .strip_suffix(ELLIPSIS)
            .expect("the cut ends in an ellipsis");
        assert!(head.len() > 3 && LONG.starts_with(head));
        assert!(line(LONG, Cut::End, 100.0).width <= 100.0);
    }

    #[test]
    fn a_line_that_fits_is_left_whole() {
        let face = capsule_face();
        assert_eq!(fit(&face, "Your turn", Cut::End, 1000.0), "Your turn");
    }

    #[test]
    fn the_width_axis_widens_the_line() {
        let default = face(WORDS_WEIGHT as f32, 100.0);
        assert!(width_of(&capsule_face(), "Your turn") > width_of(&default, "Your turn"));
    }

    #[test]
    fn a_character_archivo_lacks_draws_no_box() {
        let with = line("ok\u{2713}", Cut::End, 1000.0);
        let without = line("ok", Cut::End, 1000.0);
        assert_eq!(with.width, without.width);
    }

    #[test]
    fn a_line_has_an_outline_below_its_top_edge() {
        let drawn = line("Your turn", Cut::End, 1000.0);
        let bounds = drawn.path.expect("letters have an outline").bounds();
        assert!(bounds.top() >= 0.0);
        assert!(bounds.bottom() <= drawn.ascent + drawn.descent);
        assert!(line("", Cut::End, 1000.0).path.is_none());
    }
}
