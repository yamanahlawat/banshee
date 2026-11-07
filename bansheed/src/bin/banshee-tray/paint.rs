//! One capsule frame in tiny-skia, with each live move read at `elapsed`.

use tiny_skia::{
    Color, FillRule, LineCap, LineJoin, Mask, Paint, Path, PathBuilder, Pixmap, PixmapPaint, Rect,
    Stroke, StrokeDash, Transform,
};

use crate::chip::Show;
use crate::figure::{
    self, BAND, BAND_GAP, BEHIND, BONE, CAPSULE, CUP_GAP, CUP_RADII, CUPS, FIGURE, HALO, HEADBAND,
    INSET, KINDLE_AFTER, LIGHT, OUTLINE, PIVOT, Pose, RING, RING_STRENGTH, RUST, Segment, TRAVEL,
    WORDS_BLEED, WORDS_GAP, WORDS_TAIL, WORKING_DEPTH,
};
use crate::gesture::{self, Track};
use crate::scenes::{self, Move, holds_the_voice};
use crate::words::Line;

/// The figure's own 100-unit square has its centre here.
const CENTRE: (f64, f64) = (50.0, 50.0);

pub struct Frame<'a> {
    pub show: &'a Show,
    /// Each live move with the seconds since it started.
    pub moves: &'a [(Move, f64)],
    /// The microphone's pose. Only a show that holds the voice reads it.
    pub pose: Pose,
    pub line: Option<&'a Line>,
    /// The width the capsule grows or shrinks from, with the seconds since
    /// the change. `None` once no width transition runs.
    pub width_from: Option<(f64, f64)>,
    pub scale: u32,
}

/// The span the capsule may reach during a width transition, and the height
/// of the capsule with the clear space below it, in logical pixels.
pub fn size(frame: &Frame) -> (u32, u32) {
    let from = frame.width_from.map_or(0.0, |(from, _)| from);
    let span = target_width(frame).max(from).ceil();
    (span as u32, (TRAVEL + CAPSULE) as u32)
}

/// The width `line` gives the capsule once any transition ends.
pub fn width(line: Option<&Line>) -> f64 {
    line.filter(|line| line.path.is_some())
        .map_or(CAPSULE, |line| {
            INSET + FIGURE + WORDS_GAP + f64::from(line.width) + WORDS_TAIL
        })
        .ceil()
}

fn target_width(frame: &Frame) -> f64 {
    width(frame.line)
}

/// The capsule's width as it shows now, part way through a transition or not.
pub fn shown_width(frame: &Frame) -> f64 {
    let to = target_width(frame);
    frame.width_from.map_or(to, |(from, seconds)| {
        let [width] = &gesture::chip_width(from, to)[..] else {
            unreachable!("a width transition moves the width alone");
        };
        gesture::at(width, seconds, to)
    })
}

/// The rest pose outside the voice states, and the deeper hem while Working.
fn pose_of(frame: &Frame) -> Pose {
    let rest = figure::pose(0.0, 0.0, false);
    match frame.show {
        show if holds_the_voice(show) => frame.pose,
        Show::Working => Pose {
            depth: WORKING_DEPTH,
            ..rest
        },
        _ => rest,
    }
}

fn words<'a>(frame: &Frame<'a>) -> Option<(&'a Line, &'a Path)> {
    let line = frame.line?;
    Some((line, line.path.as_ref()?))
}

pub fn paint(frame: &Frame) -> Pixmap {
    let (width, height) = size(frame);
    let scale = frame.scale;
    let mut pixmap =
        Pixmap::new(width * scale, height * scale).expect("a frame at a scale of 1 or more");
    let live = Live::of(frame);
    let shown = shown_width(frame);
    let left = scenes::centred(f64::from(width) / 2.0, shown);
    let capsule = Transform::from_scale(scale as f32, scale as f32)
        .pre_translate(left as f32, -live.capsule.y as f32);
    let shape = stadium(shown as f32);
    let mut clip = Mask::new(pixmap.width(), pixmap.height()).expect("the size of the frame");
    clip.fill_path(&shape, FillRule::Winding, true, capsule);
    group(&mut pixmap, live.capsule.opacity, |canvas| {
        canvas.fill_path(&shape, &ink(RUST, 1.0), FillRule::Winding, capsule, None);
        let opacity = live.exit.opacity * live.entry.opacity;
        group(canvas, opacity, |canvas| {
            paint_figure(canvas, frame, &live, capsule, &clip)
        });
        if let Some((line, path)) = words(frame) {
            paint_words(canvas, line, path, &live, capsule, &clip);
        }
    });
    pixmap
}

fn paint_figure(canvas: &mut Pixmap, frame: &Frame, live: &Live, capsule: Transform, clip: &Mask) {
    let show = frame.show;
    let pose = pose_of(frame);
    let working = matches!(show, Show::Working);
    let figure = capsule
        .pre_translate((INSET + FIGURE / 2.0) as f32, (CAPSULE / 2.0) as f32)
        .pre_scale((FIGURE / 100.0) as f32, (FIGURE / 100.0) as f32)
        .pre_translate(-CENTRE.0 as f32, -CENTRE.1 as f32);
    let entry = live.entry.about(live.exit.about(figure, CENTRE), CENTRE);
    let held = Motion {
        y: pose.lift,
        scale: pose.scale,
        ..STILL
    };
    let voice = held.about(live.gesture.about(entry, PIVOT), PIVOT);

    let body = path(&figure::shroud(pose.depth, pose.sway));
    let halo = oval((HALO.0, HALO.1), (HALO.2, HALO.3));
    let light_length = figure::ellipse_length(HALO.2, HALO.3);
    let halo_half = |canvas: &mut Pixmap, near: bool| {
        let half = if near {
            Rect::from_xywh(-40.0, HALO.1 as f32, 180.0, 100.0)
        } else {
            Rect::from_xywh(-40.0, -60.0, 180.0, 60.0 + HALO.1 as f32)
        };
        let mut mask = clip.clone();
        let half = PathBuilder::from_rect(half.expect("a finite half"));
        mask.intersect_path(&half, FillRule::Winding, false, voice);
        let ring = ink(BONE, RING_STRENGTH);
        canvas.stroke_path(&halo, &ring, &stroke(RING), voice, Some(&mask));
        let light = Stroke {
            line_cap: LineCap::Round,
            dash: StrokeDash::new(
                figure::light_dash(light_length)
                    .map(|at| at as f32)
                    .to_vec(),
                live.light_phase as f32,
            ),
            ..stroke(LIGHT)
        };
        canvas.stroke_path(&halo, &ink(BONE, 1.0), &light, voice, Some(&mask));
    };

    if working {
        halo_half(canvas, false);
        canvas.fill_path(&body, &ink(RUST, 1.0), FillRule::Winding, voice, Some(clip));
        canvas.stroke_path(&body, &ink(RUST, 1.0), &stroke(BEHIND), voice, Some(clip));
    }
    let [dash, gap] = live.dash;
    let outline = Stroke {
        dash: (gap > 0.0)
            .then(|| StrokeDash::new(vec![dash as f32, gap as f32], 0.0))
            .flatten(),
        ..stroke(OUTLINE)
    };
    canvas.stroke_path(&body, &ink(BONE, 1.0), &outline, voice, Some(clip));
    if holds_the_voice(show) {
        group(canvas, live.fill.opacity, |canvas| {
            let fill = live.fill.about(voice, PIVOT);
            let strength = ink(BONE, pose.fill);
            canvas.fill_path(&body, &strength, FillRule::Winding, fill, Some(clip));
            canvas.stroke_path(&body, &strength, &stroke(OUTLINE), fill, Some(clip));
        });
    }
    if working {
        halo_half(canvas, true);
    }
    if matches!(show, Show::YourTurn | Show::Answering(_)) {
        paint_headphones(canvas, live, voice, clip);
    }
}

fn paint_headphones(canvas: &mut Pixmap, live: &Live, voice: Transform, clip: &Mask) {
    let band = path(&HEADBAND);
    for (width, colour) in [(BAND_GAP, RUST), (BAND, BONE)] {
        let line = Stroke {
            line_cap: LineCap::Round,
            ..stroke(width)
        };
        canvas.stroke_path(&band, &ink(colour, 1.0), &line, voice, Some(clip));
    }
    for (centre, press) in CUPS.into_iter().zip(live.cups) {
        let cup = oval(centre, CUP_RADII);
        let at = press.about(voice, CENTRE);
        let gap = ink(RUST, 1.0);
        canvas.fill_path(&cup, &gap, FillRule::Winding, at, Some(clip));
        canvas.stroke_path(&cup, &gap, &stroke(CUP_GAP), at, Some(clip));
        canvas.fill_path(&cup, &ink(BONE, 1.0), FillRule::Winding, at, Some(clip));
    }
}

/// The words with their baseline centred in the capsule, behind a mask over
/// their ink that the reveal widens from the left.
fn paint_words(
    canvas: &mut Pixmap,
    line: &Line,
    path: &Path,
    live: &Live,
    capsule: Transform,
    clip: &Mask,
) {
    if live.words_width <= 0.0 {
        return;
    }
    let top = (CAPSULE - f64::from(line.ascent + line.descent)) / 2.0;
    let at = capsule.pre_translate((INSET + FIGURE + WORDS_GAP) as f32, top as f32);
    let ink_box = path.bounds();
    let bleed = WORDS_BLEED as f32;
    let reveal = Rect::from_xywh(
        ink_box.left() - bleed,
        ink_box.top() - bleed,
        live.words_width as f32,
        ink_box.height() + 2.0 * bleed,
    )
    .expect("a finite reveal");
    let mut mask = clip.clone();
    mask.intersect_path(
        &PathBuilder::from_rect(reveal),
        FillRule::Winding,
        false,
        at,
    );
    let words = ink(BONE, live.words.opacity);
    canvas.fill_path(path, &words, FillRule::Winding, at, Some(&mask));
}

/// Draws into its own layer, then lays that layer on `canvas` at `opacity`,
/// as a layer's opacity applies to it and its sublayers together.
fn group(canvas: &mut Pixmap, opacity: f64, draw: impl FnOnce(&mut Pixmap)) {
    if opacity >= 1.0 {
        return draw(canvas);
    }
    let mut layer = Pixmap::new(canvas.width(), canvas.height()).expect("the canvas's size");
    draw(&mut layer);
    let paint = PixmapPaint {
        opacity: opacity as f32,
        ..PixmapPaint::default()
    };
    canvas.draw_pixmap(0, 0, layer.as_ref(), &paint, Transform::identity(), None);
}

/// A layer's transform: scaled, turned, then moved, about its anchor.
#[derive(Clone, Copy)]
struct Motion {
    x: f64,
    y: f64,
    turn: f64,
    scale: f64,
    opacity: f64,
}

const STILL: Motion = Motion {
    x: 0.0,
    y: 0.0,
    turn: 0.0,
    scale: 1.0,
    opacity: 1.0,
};

impl Motion {
    fn about(&self, outer: Transform, (x, y): (f64, f64)) -> Transform {
        outer
            .pre_translate((x + self.x) as f32, (y + self.y) as f32)
            .pre_rotate(self.turn.to_degrees() as f32)
            .pre_scale(self.scale as f32, self.scale as f32)
            .pre_translate(-x as f32, -y as f32)
    }
}

#[derive(Clone, Copy, Debug)]
enum Layer {
    Capsule,
    Exit,
    Entry,
    Gesture,
    Fill,
    Outline,
    Lights,
    Cup(usize),
    WordsMask,
}

/// Each animated value: the model, overwritten by every move in order, as a
/// later Core Animation on the same key path covers an earlier one.
struct Live {
    capsule: Motion,
    exit: Motion,
    entry: Motion,
    gesture: Motion,
    fill: Motion,
    cups: [Motion; 2],
    words: Motion,
    words_width: f64,
    light_phase: f64,
    dash: [f64; 2],
}

impl Live {
    fn of(frame: &Frame) -> Self {
        let outline = figure::path_length(&figure::shroud(0.0, 0.0));
        let failed = matches!(frame.show, Show::NothingHeard(_) | Show::Broken(_));
        let words_width = words(frame).map_or(0.0, |(_, path)| {
            f64::from(path.bounds().width()) + 2.0 * WORDS_BLEED
        });
        let mut live = Live {
            capsule: STILL,
            exit: STILL,
            entry: STILL,
            gesture: STILL,
            fill: STILL,
            cups: [STILL; 2],
            words: STILL,
            words_width,
            light_phase: 0.0,
            dash: if failed {
                figure::outline_dash(outline)
            } else {
                [outline, 0.0]
            },
        };
        let light = figure::ellipse_length(HALO.2, HALO.3);
        for &(step, seconds) in frame.moves {
            for (layer, tracks) in tracks(step, outline, light, words_width) {
                for track in &tracks {
                    let value = live.slot(layer, track.property);
                    *value = gesture::at(track, seconds, *value);
                }
            }
        }
        live
    }

    fn slot(&mut self, layer: Layer, property: &str) -> &mut f64 {
        let motion = match (layer, property) {
            (Layer::Outline, "dash") => return &mut self.dash[0],
            (Layer::Outline, "gap") => return &mut self.dash[1],
            (Layer::Lights, "lineDashPhase") => return &mut self.light_phase,
            (Layer::WordsMask, "bounds.size.width") => return &mut self.words_width,
            (Layer::Capsule, _) => &mut self.capsule,
            (Layer::Exit, _) => &mut self.exit,
            (Layer::Entry, _) => &mut self.entry,
            (Layer::Gesture, _) => &mut self.gesture,
            (Layer::Fill, _) => &mut self.fill,
            (Layer::Cup(side), _) => &mut self.cups[side],
            (Layer::WordsMask, _) => &mut self.words,
            (Layer::Outline | Layer::Lights, _) => unreachable!("{layer:?} moves no {property}"),
        };
        match property {
            "transform.translation.x" => &mut motion.x,
            "transform.translation.y" => &mut motion.y,
            "transform.rotation.z" => &mut motion.turn,
            "transform.scale" => &mut motion.scale,
            "opacity" => &mut motion.opacity,
            _ => unreachable!("{layer:?} moves no {property}"),
        }
    }
}

/// The layers each move animates. `outline`, `light` and `words` are the
/// lengths its tracks run over.
fn tracks(step: Move, outline: f64, light: f64, words: f64) -> Vec<(Layer, Vec<Track>)> {
    let (layer, tracks) = match step {
        Move::Rise => (Layer::Capsule, gesture::chip_rise()),
        Move::FadeIn => (Layer::Capsule, gesture::fade_in(0.0)),
        Move::Emerge => (Layer::Entry, gesture::figure_emerge()),
        Move::Kindle => (Layer::Fill, gesture::fill_kindle(KINDLE_AFTER)),
        Move::KindleFade => (Layer::Fill, gesture::fade_in(KINDLE_AFTER)),
        Move::Hover => (Layer::Gesture, gesture::working_hover()),
        Move::Orbit => (Layer::Lights, gesture::halo_light(light)),
        Move::Tilt => (Layer::Gesture, gesture::head_tilt()),
        Move::Open => (Layer::Fill, gesture::fill_kindle(0.0)),
        Move::OpenFade => (Layer::Fill, gesture::fade_in(0.0)),
        Move::Cups => {
            return vec![
                (Layer::Cup(0), gesture::cups_press(1.0)),
                (Layer::Cup(1), gesture::cups_press(-1.0)),
            ];
        }
        Move::Shake => (Layer::Gesture, gesture::head_shake()),
        Move::Strain => (Layer::Gesture, gesture::strain()),
        Move::Breaks => (Layer::Outline, gesture::outline_breaks(outline)),
        Move::Words => (Layer::WordsMask, gesture::words_reveal(words)),
        Move::WordsFade => (Layer::WordsMask, gesture::fade_in(0.0)),
        Move::Float => (Layer::Exit, gesture::float_out()),
        Move::FrameSink => (Layer::Capsule, gesture::frame_sink()),
        Move::FailureExit => (Layer::Capsule, gesture::failure_exit()),
        Move::FadeOut => (Layer::Capsule, gesture::fade_out()),
    };
    vec![(layer, tracks)]
}

/// How long `step` runs before its frames stop changing. `None` for a move that
/// repeats.
#[cfg(any(target_os = "linux", test))]
pub fn lasts(step: Move) -> Option<f64> {
    tracks(step, 0.0, 0.0, 0.0)
        .iter()
        .try_fold(0.0, |longest: f64, (_, tracks)| {
            Some(longest.max(gesture::lasts(tracks)?))
        })
}

fn ink(hex: u32, alpha: f64) -> Paint<'static> {
    let channel = |shift: u32| ((hex >> shift) & 0xFF) as u8;
    let mut colour = Color::from_rgba8(channel(16), channel(8), channel(0), 0xFF);
    colour.set_alpha(alpha as f32);
    let mut paint = Paint::default();
    paint.set_color(colour);
    paint
}

fn stroke(width: f64) -> Stroke {
    Stroke {
        width: width as f32,
        line_join: LineJoin::Round,
        ..Stroke::default()
    }
}

fn path(segments: &[Segment]) -> Path {
    let mut builder = PathBuilder::new();
    for segment in segments {
        match *segment {
            Segment::Move(x, y) => builder.move_to(x as f32, y as f32),
            Segment::Line(x, y) => builder.line_to(x as f32, y as f32),
            Segment::Cubic(points) => {
                let [x1, y1, x2, y2, x, y] = points.map(|at| at as f32);
                builder.cubic_to(x1, y1, x2, y2, x, y);
            }
            Segment::Close => builder.close(),
        }
    }
    builder.finish().expect("the figure's shapes have ink")
}

fn oval((x, y): (f64, f64), (rx, ry): (f64, f64)) -> Path {
    let bounds = Rect::from_ltrb(
        (x - rx) as f32,
        (y - ry) as f32,
        (x + rx) as f32,
        (y + ry) as f32,
    );
    PathBuilder::from_oval(bounds.expect("a finite oval")).expect("an oval with area")
}

/// The capsule `width` wide with fully rounded ends.
fn stadium(width: f32) -> Path {
    let radius = (CAPSULE / 2.0) as f32;
    let mut builder = PathBuilder::new();
    builder.push_circle(radius, radius, radius);
    builder.push_circle(width - radius, radius, radius);
    let middle = Rect::from_ltrb(radius, 0.0, width - radius, 2.0 * radius);
    builder.push_rect(middle.expect("a capsule at least as wide as it is tall"));
    builder.finish().expect("a capsule has ink")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chip::{Cut, Done, words_of};
    use crate::figure::{
        CAPSULE, FIGURE, INSET, Segment, TRAVEL, WORDS_GAP, WORDS_TAIL, pose, shroud,
    };
    use crate::words;

    const RUST: [u8; 4] = [0x8A, 0x2A, 0x0D, 0xFF];
    const BONE: [u8; 4] = [0xF2, 0xEF, 0xE9, 0xFF];

    fn every_show() -> Vec<Show> {
        vec![
            Show::Recording(String::new()),
            Show::Recording("hello there".into()),
            Show::Working,
            Show::YourTurn,
            Show::Answering("yes please".into()),
            Show::NothingHeard("Nothing heard".into()),
            Show::Broken("Banshee cannot record".into()),
            Show::Done(Done::Typed),
            Show::Copied,
        ]
    }

    fn line_of(show: &Show) -> Option<Line> {
        words_of(show).map(|(text, cut)| words::line(text, cut, 1000.0))
    }

    fn frame<'a>(show: &'a Show, line: Option<&'a Line>, moves: &'a [(Move, f64)]) -> Frame<'a> {
        Frame {
            show,
            moves,
            pose: pose(0.0, 0.0, false),
            line,
            width_from: None,
            scale: 1,
        }
    }

    fn rgba(pixmap: &Pixmap, x: u32, y: u32) -> [u8; 4] {
        let pixel = pixmap.pixel(x, y).expect("inside the pixmap");
        [pixel.red(), pixel.green(), pixel.blue(), pixel.alpha()]
    }

    fn near(a: [u8; 4], b: [u8; 4]) -> bool {
        a.iter().zip(b).all(|(&a, b)| a.abs_diff(b) <= 6)
    }

    /// How far each pixel has gone from rust toward bone, weighted over the
    /// figure's square at the capsule's left end.
    fn bone_centre(pixmap: &Pixmap) -> (f64, f64) {
        let (mut x_sum, mut y_sum, mut total) = (0.0, 0.0, 0.0);
        for y in 0..CAPSULE as u32 {
            for x in 0..CAPSULE as u32 {
                let [red, ..] = rgba(pixmap, x, y);
                let weight = ((f64::from(red) - f64::from(RUST[0])) / 104.0).clamp(0.0, 1.0);
                x_sum += f64::from(x) * weight;
                y_sum += f64::from(y) * weight;
                total += weight;
            }
        }
        (x_sum / total, y_sum / total)
    }

    fn alpha_centre_y(pixmap: &Pixmap) -> f64 {
        let (mut sum, mut total) = (0.0, 0.0);
        for y in 0..pixmap.height() {
            for x in 0..pixmap.width() {
                let alpha = f64::from(rgba(pixmap, x, y)[3]);
                sum += f64::from(y) * alpha;
                total += alpha;
            }
        }
        sum / total
    }

    #[test]
    fn every_show_paints_rust_at_the_capsule_left_end_and_leaves_the_corners_clear() {
        for show in every_show() {
            let line = line_of(&show);
            let frame = frame(&show, line.as_ref(), &[]);
            let pixmap = paint(&frame);
            let (width, height) = size(&frame);
            let middle = CAPSULE as u32 / 2;
            assert!(near(rgba(&pixmap, 3, middle), RUST), "{show:?}");
            let bottom = CAPSULE as u32 - 1;
            for (x, y) in [(0, 0), (width - 1, 0), (0, bottom), (width - 1, bottom)] {
                assert_eq!(rgba(&pixmap, x, y)[3], 0, "{show:?} at ({x}, {y})");
            }
            assert_eq!(rgba(&pixmap, width / 2, height - 1)[3], 0, "{show:?}");
        }
    }

    #[test]
    fn the_shroud_is_bone_while_recording_and_rust_while_working() {
        let centre = (CAPSULE / 2.0) as u32;
        let recording = Show::Recording(String::new());
        let pixmap = paint(&frame(&recording, None, &[]));
        assert!(near(rgba(&pixmap, centre, centre), BONE));
        let pixmap = paint(&frame(&Show::Working, None, &[]));
        assert!(near(rgba(&pixmap, centre, centre), RUST));
    }

    /// Points along the resting shroud, in the device pixels of a frame at `scale`.
    fn along_the_outline(scale: u32) -> Vec<(u32, u32)> {
        let to_pixel = |(u, v): (f64, f64)| {
            let at =
                |unit: f64| (CAPSULE / 2.0 + (unit - 50.0) * FIGURE / 100.0) * f64::from(scale);
            (at(u) as u32, at(v) as u32)
        };
        let mut points = Vec::new();
        let (mut start, mut at) = ((0.0, 0.0), (0.0, 0.0));
        for segment in shroud(0.0, 0.0) {
            let (from, to, controls) = match segment {
                Segment::Move(x, y) => {
                    start = (x, y);
                    at = start;
                    continue;
                }
                Segment::Line(x, y) => (at, (x, y), [at, (x, y)]),
                Segment::Cubic([x1, y1, x2, y2, x, y]) => (at, (x, y), [(x1, y1), (x2, y2)]),
                Segment::Close => (at, start, [at, start]),
            };
            for step in 0..40 {
                let t = f64::from(step) / 40.0;
                let u = 1.0 - t;
                let [a, b, c, d] = [u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t];
                let [(x1, y1), (x2, y2)] = controls;
                points.push(to_pixel((
                    a * from.0 + b * x1 + c * x2 + d * to.0,
                    a * from.1 + b * y1 + c * y2 + d * to.1,
                )));
            }
            at = to;
        }
        points
    }

    fn outline_inks(show: &Show) -> (usize, usize) {
        let scale = 4;
        let line = line_of(show);
        let pixmap = paint(&Frame {
            scale,
            ..frame(show, line.as_ref(), &[])
        });
        let inks: Vec<_> = along_the_outline(scale)
            .into_iter()
            .map(|(x, y)| rgba(&pixmap, x, y))
            .collect();
        let count = |ink| inks.iter().filter(|&&found| near(found, ink)).count();
        (count(RUST), count(BONE))
    }

    #[test]
    fn a_broken_capsule_draws_its_dashed_outline() {
        let (rust, bone) = outline_inks(&Show::Broken("Banshee cannot record".into()));
        assert!(rust > 0 && bone > 0, "{rust} rust, {bone} bone");
        let (rust, bone) = outline_inks(&Show::Done(Done::Typed));
        assert!(rust == 0 && bone > 0, "{rust} rust, {bone} bone");
    }

    #[test]
    fn the_width_is_inset_figure_gap_words_and_tail() {
        let show = Show::YourTurn;
        let line = words::line("Your turn", Cut::End, 1000.0);
        let span = INSET + FIGURE + WORDS_GAP + f64::from(line.width) + WORDS_TAIL;
        let height = (TRAVEL + CAPSULE) as u32;
        assert_eq!(
            size(&frame(&show, Some(&line), &[])),
            (span.ceil() as u32, height)
        );
        assert_eq!(
            size(&frame(&Show::Working, None, &[])),
            (CAPSULE as u32, height)
        );
    }

    #[test]
    fn scale_two_doubles_the_pixmap() {
        let show = Show::YourTurn;
        let line = line_of(&show);
        let one = paint(&frame(&show, line.as_ref(), &[]));
        let two = paint(&Frame {
            scale: 2,
            ..frame(&show, line.as_ref(), &[])
        });
        assert_eq!(
            (two.width(), two.height()),
            (2 * one.width(), 2 * one.height())
        );
        assert_eq!(
            size(&frame(&show, line.as_ref(), &[])),
            (one.width(), one.height())
        );
    }

    #[test]
    fn the_rise_starts_below_the_capsules_rest() {
        let rest = paint(&frame(&Show::Working, None, &[]));
        let rising = paint(&frame(&Show::Working, None, &[(Move::Rise, 0.05)]));
        assert!(alpha_centre_y(&rising) > alpha_centre_y(&rest) + 2.0);
    }

    #[test]
    fn the_figure_emerges_from_below_then_strains_and_floats_upward() {
        let recording = Show::Recording(String::new());
        let broken = Show::Broken(String::new());
        let done = Show::Done(Done::Typed);
        for (show, step, elapsed, rises) in [
            (&recording, Move::Emerge, 0.05, false),
            (&broken, Move::Strain, 0.315, true),
            (&done, Move::Float, 0.25, true),
        ] {
            let (_, rest) = bone_centre(&paint(&frame(show, None, &[])));
            let (_, moved) = bone_centre(&paint(&frame(show, None, &[(step, elapsed)])));
            assert_eq!(moved < rest - 1.0, rises, "{step:?}: {rest} then {moved}");
            assert_eq!(moved > rest + 1.0, !rises, "{step:?}: {rest} then {moved}");
        }
    }

    #[test]
    fn the_answer_tilts_the_head_to_the_left() {
        let show = Show::Answering(String::new());
        let (rest, _) = bone_centre(&paint(&frame(&show, None, &[])));
        let (tilted, _) = bone_centre(&paint(&frame(&show, None, &[(Move::Tilt, 1.0)])));
        assert!(tilted < rest - 1.0, "{rest} then {tilted}");
    }

    #[test]
    fn the_words_reveal_from_nothing() {
        let show = Show::Copied;
        let line = line_of(&show);
        let words_bone = |moves: &[(Move, f64)]| {
            let pixmap = paint(&frame(&show, line.as_ref(), moves));
            let left = (INSET + FIGURE + WORDS_GAP) as u32;
            (left..pixmap.width())
                .flat_map(|x| (0..CAPSULE as u32).map(move |y| (x, y)))
                .filter(|&(x, y)| near(rgba(&pixmap, x, y), BONE))
                .count()
        };
        assert_eq!(words_bone(&[(Move::Words, 0.0)]), 0);
        assert!(words_bone(&[]) > 0);
    }

    #[test]
    fn the_size_spans_the_wider_of_the_two_widths() {
        let show = Show::Copied;
        let line = line_of(&show);
        let settled = size(&frame(&show, line.as_ref(), &[])).0;
        let from = |width| Frame {
            width_from: Some((width, 0.1)),
            ..frame(&show, line.as_ref(), &[])
        };
        assert_eq!(size(&from(f64::from(settled) + 40.5)).0, settled + 41);
        assert_eq!(size(&from(CAPSULE)).0, settled);
    }

    #[test]
    fn a_width_transition_starts_at_its_from_width_and_ends_centred_in_its_span() {
        let show = Show::Copied;
        let line = line_of(&show);
        let settled = size(&frame(&show, line.as_ref(), &[])).0;
        let shrinking = |seconds| {
            paint(&Frame {
                width_from: Some((f64::from(settled + 60), seconds)),
                ..frame(&show, line.as_ref(), &[])
            })
        };
        let middle = CAPSULE as u32 / 2;
        let starting = shrinking(0.0);
        assert!(near(rgba(&starting, 2, middle), RUST));
        let halfway = shrinking(0.06);
        assert_eq!(rgba(&halfway, 2, middle)[3], 0);
        assert!(near(rgba(&halfway, 29, middle), RUST));
        let ended = shrinking(1.0);
        assert_eq!(rgba(&ended, 29, middle)[3], 0);
        assert!(near(rgba(&ended, 33, middle), RUST));
        assert_eq!(rgba(&ended, settled + 30, middle)[3], 0);
    }

    #[test]
    fn only_a_voice_show_follows_the_pose() {
        let loud = pose(1.0, 300.0, false);
        let at = |show: &Show, pose| {
            paint(&Frame {
                pose,
                ..frame(show, None, &[])
            })
            .take()
        };
        for show in [Show::Working, Show::YourTurn, Show::Done(Done::Typed)] {
            assert!(
                at(&show, loud) == at(&show, pose(0.0, 0.0, false)),
                "{show:?}"
            );
        }
        let recording = Show::Recording(String::new());
        assert!(at(&recording, loud) != at(&recording, pose(0.0, 0.0, false)));
    }

    #[test]
    fn every_scene_and_its_leaving_paint_at_any_clock() {
        use crate::scenes::{Entrance, leaving, moves};
        let clocks = [0.0, 0.05, 0.15, 0.3, 0.5, 0.9, 1.5, 3.0];
        for show in every_show() {
            let line = line_of(&show);
            for entrance in [Entrance::Fresh, Entrance::SameWords, Entrance::NewWords] {
                for calm in [false, true] {
                    let steps = moves(&show, entrance, calm);
                    for (index, &clock) in clocks.iter().enumerate() {
                        let mut timed: Vec<_> = steps.iter().map(|&step| (step, clock)).collect();
                        if let Some(leave) = leaving(&show, calm) {
                            timed.push((leave, clock / 2.0));
                        }
                        let frame = Frame {
                            pose: pose(0.5, clock * 1000.0, calm),
                            width_from: (index % 2 == 0).then_some((CAPSULE + 50.0, clock)),
                            ..frame(&show, line.as_ref(), &timed)
                        };
                        let pixmap = paint(&frame);
                        assert_eq!((pixmap.width(), pixmap.height()), size(&frame));
                    }
                }
            }
        }
    }
}
