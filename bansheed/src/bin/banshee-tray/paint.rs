//! One capsule frame in tiny-skia, with each live move read at `elapsed`.

use std::sync::LazyLock;

use tiny_skia::{
    Color, FillRule, GradientStop, LineCap, LineJoin, LinearGradient, Mask, Paint, Path,
    PathBuilder, Pixmap, PixmapPaint, Point, Rect, SpreadMode, Stroke, StrokeDash, Transform,
};

use crate::card::{
    self, Block, CHIP_BORDER, ChipBox, DRAIN_HEIGHT, DRAIN_RADIUS, Divider, DividerKind, Layout,
    MUTED, PlacedGlyph, RIM, RIM_WIDTH, SEGMENT_GAP, SEGMENT_HEIGHT, SEGMENT_OFF, SEGMENTS, SIDE,
    Shape, Text,
};
use crate::chip::{Show, segments_lit};
use crate::figure::{
    self, BAND, BAND_GAP, BEHIND, BONE, CAPSULE, CUP_GAP, CUP_RADII, CUPS, FIGURE, HALO, HEADBAND,
    INSET, KINDLE_AFTER, LIGHT, OUTLINE, PIVOT, Pose, RING, RING_STRENGTH, RUST, Segment, TRAVEL,
    WORDS_BLEED, WORDS_LEFT,
};
use crate::gesture::{self, Property, Track};
use crate::glyphs::{DOTS, Glyph, Mark, marks};
use crate::scenes::{self, Move, failed, holds_the_voice, rest_pose, wears_headphones};
use crate::words::{self, Line};

const CENTRE: (f64, f64) = (50.0, 50.0);

static OUTLINE_LENGTH: LazyLock<f64> =
    LazyLock::new(|| figure::path_length(&figure::shroud(0.0, 0.0)));
static LIGHT_LENGTH: LazyLock<f64> = LazyLock::new(|| figure::ellipse_length(HALO.2, HALO.3));

/// A card's layout with each of its lines shaped once, so a frame shapes
/// nothing.
pub struct Card {
    pub layout: Layout,
    /// Each block's lines, in the layout's order.
    lines: Vec<Vec<Line>>,
    chip: Option<Line>,
}

impl Card {
    pub fn new(layout: Layout) -> Self {
        let lines = layout
            .blocks
            .iter()
            .map(|block| {
                block
                    .lines
                    .iter()
                    .map(|text| words::shaped(&text.line, text.cut))
                    .collect()
            })
            .collect();
        let chip = layout
            .chip
            .as_ref()
            .map(|chip| words::shaped(&chip.label.line, chip.label.cut));
        Card {
            layout,
            lines,
            chip,
        }
    }
}

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
    pub card: Option<&'a Card>,
    /// The height the card grows or shrinks from, with the seconds since the
    /// change. `None` once no height transition runs.
    pub height_from: Option<(f64, f64)>,
    pub level: f64,
    /// The share of the countdown left, from 1 to 0. `None` with no countdown.
    pub countdown_share: Option<f64>,
    pub scale: u32,
    /// The size the surface already has, which a frame never shrinks: a
    /// compositor may animate a resize by stretching the new frame.
    pub at_least: (u32, u32),
}

impl<'a> Frame<'a> {
    fn layout(&self) -> Option<&'a Layout> {
        self.card.map(|card| &card.layout)
    }
}

/// The span the capsule may reach during a width transition, and the height
/// it may reach during a height transition with the clear space below it, in
/// logical pixels, and never less than the surface already has.
pub fn size(frame: &Frame) -> (u32, u32) {
    let (to_width, to_height) = target(frame.line, frame.layout());
    let largest = |to: f64, from: Option<(f64, f64)>| to.max(from.map_or(0.0, |(from, _)| from));
    let (width, height) = frame.at_least;
    (
        (largest(to_width, frame.width_from).ceil() as u32).max(width),
        ((TRAVEL + largest(to_height, frame.height_from)).ceil() as u32).max(height),
    )
}

/// The width and height the capsule or the card takes once any transition
/// ends.
pub fn target(line: Option<&Line>, layout: Option<&Layout>) -> (f64, f64) {
    match layout {
        Some(layout) => (layout.shape.width.ceil(), layout.shape.height.ceil()),
        None => (width(line), CAPSULE),
    }
}

/// The width `line` gives the capsule once any transition ends.
pub fn width(line: Option<&Line>) -> f64 {
    line.filter(|line| line.path.is_some())
        .map_or(CAPSULE, |line| figure::capsule_width(f64::from(line.width)))
        .ceil()
}

/// The capsule's or the card's width as it shows now, part way through a
/// transition or not.
pub fn shown_width(frame: &Frame) -> f64 {
    let (to, _) = target(frame.line, frame.layout());
    frame.width_from.map_or(to, |(from, seconds)| {
        gesture::at(&gesture::chip_width(from, to), seconds, to)
    })
}

pub fn shown_height(frame: &Frame) -> f64 {
    let (_, to) = target(frame.line, frame.layout());
    frame.height_from.map_or(to, |(from, seconds)| {
        gesture::at(&gesture::chip_height(from, to), seconds, to)
    })
}

pub fn shown_shape(frame: &Frame) -> Shape {
    Shape {
        width: shown_width(frame),
        height: shown_height(frame),
        radius: frame
            .layout()
            .map_or(CAPSULE / 2.0, |layout| layout.shape.radius),
    }
}

/// The capsule's top edge in the surface, in logical pixels.
pub fn capsule_top(frame: &Frame) -> f64 {
    top(frame, &Live::of(frame))
}

/// The bottom edge rests `TRAVEL` above the surface's, so a card grows up
/// from where the pill sits.
fn top(frame: &Frame, live: &Live) -> f64 {
    live.top() + (f64::from(size(frame).1) - TRAVEL - shown_height(frame))
}

fn pose_of(frame: &Frame) -> Pose {
    if holds_the_voice(frame.show) {
        frame.pose
    } else {
        rest_pose(frame.show, false)
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
    let shown = shown_shape(frame);
    let left = scenes::centred(f64::from(width) / 2.0, shown.width);
    let capsule = Transform::from_scale(scale as f32, scale as f32)
        .pre_translate(left as f32, top(frame, &live) as f32);
    let shape = rounded(shown.width as f32, shown.height as f32, shown.radius as f32);
    let mut clip = Mask::new(pixmap.width(), pixmap.height()).expect("the size of the frame");
    clip.fill_path(&shape, FillRule::Winding, true, capsule);
    group(&mut pixmap, live.capsule.opacity, |canvas| {
        canvas.fill_path(&shape, &ink(RUST, 1.0), FillRule::Winding, capsule, None);
        if let Some(card) = frame.card {
            return paint_card(canvas, frame, card, shown, &live, capsule, &clip);
        }
        let pill = body(frame, capsule);
        let opacity = live.exit.opacity * live.entry.opacity;
        group(canvas, opacity, |canvas| {
            paint_figure(canvas, frame, &live, pill, &clip)
        });
        if let Some((line, path)) = words(frame) {
            paint_words(canvas, line, path, &live, pill, &clip);
        }
    });
    pixmap
}

/// Where the contents sit in a shape whose top is at `at`: held to its bottom
/// while its height moves.
fn body(frame: &Frame, at: Transform) -> Transform {
    let (_, to) = target(frame.line, frame.layout());
    at.pre_translate(0.0, (shown_height(frame) - to) as f32)
}

fn paint_card(
    canvas: &mut Pixmap,
    frame: &Frame,
    card: &Card,
    shown: Shape,
    live: &Live,
    at: Transform,
    clip: &Mask,
) {
    let layout = &card.layout;
    let inset = RIM_WIDTH / 2.0;
    let rim = rounded(
        (shown.width - RIM_WIDTH) as f32,
        (shown.height - RIM_WIDTH) as f32,
        (layout.shape.radius - inset) as f32,
    );
    let rim_at = at.pre_translate(inset as f32, inset as f32);
    canvas.stroke_path(&rim, &ink(RIM, 1.0), &stroke(RIM_WIDTH), rim_at, None);

    let body = body(frame, at);
    let mut blocks = layout.blocks.iter().zip(&card.lines);
    let Some((answer, answer_lines)) = blocks.next_back() else {
        return;
    };
    for (block, lines) in blocks {
        paint_block(canvas, block, lines, body, clip);
    }
    if let Some(divider) = &layout.divider {
        let track = layout.shape.width - 2.0 * SIDE;
        let at = body.pre_translate(SIDE as f32, divider.top as f32);
        paint_divider(canvas, divider, track, frame.countdown_share, at, clip);
    }
    group(canvas, live.words.opacity, |canvas| {
        paint_block(canvas, answer, answer_lines, body, clip);
        if let Some(PlacedGlyph { glyph, x, top }) = layout.glyph {
            let glyph_at = body.pre_translate(x as f32, top as f32);
            paint_glyph(canvas, glyph, frame.level, live, glyph_at, clip);
        }
        if let (Some(chip), Some(label)) = (&layout.chip, &card.chip) {
            paint_chip(canvas, chip, label, body, clip);
        }
    });
}

fn paint_chip(canvas: &mut Pixmap, chip: &ChipBox, label: &Line, body: Transform, clip: &Mask) {
    let ChipBox {
        x,
        top,
        width,
        height,
        ..
    } = *chip;
    let inset = CHIP_BORDER / 2.0;
    let border = rounded(
        (width - CHIP_BORDER) as f32,
        (height - CHIP_BORDER) as f32,
        (height / 2.0 - inset) as f32,
    );
    let at = body.pre_translate((x + inset) as f32, (top + inset) as f32);
    let muted = ink(MUTED, 1.0);
    canvas.stroke_path(&border, &muted, &stroke(CHIP_BORDER), at, Some(clip));
    paint_line(canvas, &chip.label, label, &ink(BONE, 1.0), body, clip);
}

fn paint_divider(
    canvas: &mut Pixmap,
    divider: &Divider,
    track: f64,
    share: Option<f64>,
    at: Transform,
    clip: &Mask,
) {
    let line = |width: f64, height: f64| {
        Rect::from_xywh(0.0, 0.0, width as f32, height as f32).expect("a finite divider")
    };
    let laid = match divider.kind {
        DividerKind::Thin => None,
        DividerKind::Drain { share: laid, .. } => Some(laid),
        DividerKind::Stepped { lit } => {
            let lit = card::lit_segments(share.map_or(lit, segments_lit));
            let width = card::segment_width(track);
            for index in 0..SEGMENTS {
                let shown = if lit.contains(&index) {
                    ink(BONE, 1.0)
                } else {
                    ink(MUTED, SEGMENT_OFF)
                };
                let x = index as f64 * (width + SEGMENT_GAP);
                let segment_at = at.pre_translate(x as f32, 0.0);
                canvas.fill_rect(line(width, SEGMENT_HEIGHT), &shown, segment_at, Some(clip));
            }
            return;
        }
    };
    let muted = ink(MUTED, 1.0);
    canvas.fill_rect(line(track, divider.height), &muted, at, Some(clip));
    let Some(laid) = laid else {
        return;
    };
    let width = share.unwrap_or(laid) * track;
    if width > 0.0 {
        let radius = DRAIN_RADIUS.min(width / 2.0);
        let drain = rounded(width as f32, DRAIN_HEIGHT as f32, radius as f32);
        let drain_at = at.pre_translate(
            ((track - width) / 2.0) as f32,
            ((divider.height - DRAIN_HEIGHT) / 2.0) as f32,
        );
        canvas.fill_path(
            &drain,
            &ink(BONE, 1.0),
            FillRule::Winding,
            drain_at,
            Some(clip),
        );
    }
}

fn paint_block(canvas: &mut Pixmap, block: &Block, lines: &[Line], body: Transform, clip: &Mask) {
    for (text, line) in block.lines.iter().zip(lines) {
        let top = line_top(text, line);
        let hex = text.ink.hex();
        let paint = match block.fade {
            Some(fade) => faded(hex, block.top - top, block.top + fade - top),
            None => ink(hex, 1.0),
        };
        paint_line(canvas, text, line, &paint, body, clip);
    }
}

/// The top of `line`'s ink box, centred in the text's line box.
fn line_top(text: &Text, line: &Line) -> f64 {
    text.top + (text.height - f64::from(line.ascent + line.descent)) / 2.0
}

fn paint_line(
    canvas: &mut Pixmap,
    text: &Text,
    line: &Line,
    paint: &Paint,
    body: Transform,
    clip: &Mask,
) {
    let Some(path) = &line.path else {
        return;
    };
    let at = body.pre_translate(text.x as f32, line_top(text, line) as f32);
    canvas.fill_path(path, paint, FillRule::Winding, at, Some(clip));
}

fn paint_glyph(
    canvas: &mut Pixmap,
    glyph: Glyph,
    level: f64,
    live: &Live,
    at: Transform,
    clip: &Mask,
) {
    for (index, mark) in marks(glyph, level, false).into_iter().enumerate() {
        let pulse = match glyph {
            Glyph::Dots => live.dots[index].opacity,
            Glyph::Bars | Glyph::BarsQuiet | Glyph::Arcs => 1.0,
        };
        match mark {
            Mark::Stroke {
                segments,
                width,
                opacity,
            } => {
                let bone = ink(BONE, opacity * pulse);
                let line = round_stroke(width);
                canvas.stroke_path(&path(&segments), &bone, &line, at, Some(clip));
            }
            Mark::Disc { centre, radius } => {
                let bone = ink(BONE, pulse);
                let disc = oval(centre, (radius, radius));
                canvas.fill_path(&disc, &bone, FillRule::Winding, at, Some(clip));
            }
        }
    }
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
            dash: StrokeDash::new(
                figure::light_dash(*LIGHT_LENGTH)
                    .map(|at| at as f32)
                    .to_vec(),
                live.light_phase as f32,
            ),
            ..round_stroke(LIGHT)
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
    if wears_headphones(show) {
        paint_headphones(canvas, live, voice, clip);
    }
}

fn paint_headphones(canvas: &mut Pixmap, live: &Live, voice: Transform, clip: &Mask) {
    let band = path(&HEADBAND);
    for (width, colour) in [(BAND_GAP, RUST), (BAND, BONE)] {
        let line = round_stroke(width);
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
    let at = capsule.pre_translate(WORDS_LEFT as f32, top as f32);
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
    Dot(usize),
}

/// Each animated value: the model, overwritten by every move in order.
struct Live {
    capsule: Motion,
    exit: Motion,
    entry: Motion,
    gesture: Motion,
    fill: Motion,
    cups: [Motion; 2],
    words: Motion,
    dots: [Motion; DOTS],
    words_width: f64,
    light_phase: f64,
    dash: [f64; 2],
}

impl Live {
    fn top(&self) -> f64 {
        -self.capsule.y
    }

    fn of(frame: &Frame) -> Self {
        let outline = *OUTLINE_LENGTH;
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
            dots: [STILL; DOTS],
            words_width,
            light_phase: 0.0,
            dash: if failed(frame.show) {
                figure::outline_dash(outline)
            } else {
                [outline, 0.0]
            },
        };
        for &(step, seconds) in frame.moves {
            for (layer, tracks) in tracks(step, outline, *LIGHT_LENGTH, words_width) {
                for track in &tracks {
                    let value = live.slot(layer, track.property);
                    *value = gesture::at(track, seconds, *value);
                }
            }
        }
        live
    }

    fn slot(&mut self, layer: Layer, property: Property) -> &mut f64 {
        match property {
            Property::Dash => &mut self.dash[0],
            Property::Gap => &mut self.dash[1],
            Property::DashPhase => &mut self.light_phase,
            Property::Width => &mut self.words_width,
            Property::Height => unreachable!("no move's track animates the height"),
            Property::TranslationX => &mut self.motion(layer).x,
            Property::TranslationY => &mut self.motion(layer).y,
            Property::Rotation => &mut self.motion(layer).turn,
            Property::Scale => &mut self.motion(layer).scale,
            Property::Opacity => &mut self.motion(layer).opacity,
        }
    }

    fn motion(&mut self, layer: Layer) -> &mut Motion {
        match layer {
            Layer::Capsule => &mut self.capsule,
            Layer::Exit => &mut self.exit,
            Layer::Entry => &mut self.entry,
            Layer::Gesture => &mut self.gesture,
            Layer::Fill => &mut self.fill,
            Layer::Cup(side) => &mut self.cups[side],
            Layer::WordsMask => &mut self.words,
            Layer::Dot(index) => &mut self.dots[index],
            Layer::Outline | Layer::Lights => unreachable!("{layer:?} has no motion"),
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
        Move::Pulse => {
            return (0..DOTS)
                .map(|i| (Layer::Dot(i), gesture::dot_pulse(i)))
                .collect();
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
pub fn lasts(step: Move) -> Option<f64> {
    tracks(step, 0.0, 0.0, 0.0)
        .iter()
        .try_fold(0.0, |longest: f64, (_, tracks)| {
            Some(longest.max(gesture::lasts(tracks)?))
        })
}

fn colour(hex: u32, alpha: f64) -> Color {
    let channel = |shift: u32| ((hex >> shift) & 0xFF) as u8;
    let mut colour = Color::from_rgba8(channel(16), channel(8), channel(0), 0xFF);
    colour.set_alpha(alpha as f32);
    colour
}

fn ink(hex: u32, alpha: f64) -> Paint<'static> {
    let mut paint = Paint::default();
    paint.set_color(colour(hex, alpha));
    paint
}

/// `hex` clear at `from` and whole at `to`, down the y axis.
fn faded(hex: u32, from: f64, to: f64) -> Paint<'static> {
    let shader = LinearGradient::new(
        Point::from_xy(0.0, from as f32),
        Point::from_xy(0.0, to as f32),
        vec![
            GradientStop::new(0.0, colour(hex, 0.0)),
            GradientStop::new(1.0, colour(hex, 1.0)),
        ],
        SpreadMode::Pad,
        Transform::identity(),
    );
    Paint {
        shader: shader.expect("a fade with length"),
        ..Paint::default()
    }
}

fn stroke(width: f64) -> Stroke {
    Stroke {
        width: width as f32,
        line_join: LineJoin::Round,
        ..Stroke::default()
    }
}

fn round_stroke(width: f64) -> Stroke {
    Stroke {
        line_cap: LineCap::Round,
        ..stroke(width)
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

fn rounded(width: f32, height: f32, radius: f32) -> Path {
    let arm = radius * (1.0 - 4.0 / 3.0 * (std::f32::consts::SQRT_2 - 1.0));
    let (right, bottom) = (width, height);
    let mut builder = PathBuilder::new();
    builder.move_to(radius, 0.0);
    builder.line_to(right - radius, 0.0);
    builder.cubic_to(right - arm, 0.0, right, arm, right, radius);
    builder.line_to(right, bottom - radius);
    builder.cubic_to(
        right,
        bottom - arm,
        right - arm,
        bottom,
        right - radius,
        bottom,
    );
    builder.line_to(radius, bottom);
    builder.cubic_to(arm, bottom, 0.0, bottom - arm, 0.0, bottom - radius);
    builder.line_to(0.0, radius);
    builder.cubic_to(0.0, arm, arm, 0.0, radius, 0.0);
    builder.close();
    builder.finish().expect("a rounded shape has ink")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    use crate::card::{self, CARD_WIDTH, DividerKind};
    use crate::chip::{Countdown, Done, Scene, Trim, words_of};
    use crate::figure::{
        CAPSULE, FIGURE, INSET, Segment, TRAVEL, WORDS_GAP, WORDS_TAIL, pose, shroud,
    };
    use crate::words::{self, Fonts};

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
            Show::Reading,
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
            card: None,
            height_from: None,
            level: 0.0,
            countdown_share: None,
            scale: 1,
            at_least: (0, 0),
        }
    }

    fn laid_from(scene: &Scene, calm: bool) -> Option<Card> {
        card::layout(scene, CARD_WIDTH - 2.0 * SIDE, calm, Instant::now(), &Fonts).map(Card::new)
    }

    fn laid(show: &Show, question: &str) -> Option<Card> {
        laid_from(
            &Scene {
                question: Some(question.into()),
                ..Scene::of(show.clone(), 1)
            },
            false,
        )
    }

    fn counting(show: &Show, calm: bool, teach: bool) -> Card {
        let countdown = Countdown {
            since: Instant::now(),
            lasts: Duration::from_millis(2500),
            teach,
        };
        let scene = Scene {
            question: Some("Retry it?".into()),
            countdown: Some(countdown),
            ..Scene::of(show.clone(), 1)
        };
        laid_from(&scene, calm).expect("a card")
    }

    fn card_frame<'a>(show: &'a Show, card: &'a Card, moves: &'a [(Move, f64)]) -> Frame<'a> {
        Frame {
            card: Some(card),
            ..frame(show, None, moves)
        }
    }

    fn block_of(layout: &Layout, cut: card::Cut) -> &card::Block {
        layout
            .blocks
            .iter()
            .find(|block| block.lines.first().is_some_and(|text| text.cut == cut))
            .expect("a block in that cut")
    }

    fn bytes(hex: u32) -> [u8; 4] {
        let [_, red, green, blue] = hex.to_be_bytes();
        [red, green, blue, 0xFF]
    }

    fn count(pixmap: &Pixmap, ink: [u8; 4], top: f64, bottom: f64, scale: f64) -> usize {
        let rows = (top * scale).ceil() as u32..(bottom * scale).floor() as u32;
        rows.flat_map(|y| (0..pixmap.width()).map(move |x| (x, y)))
            .filter(|&(x, y)| near(rgba(pixmap, x, y), ink))
            .count()
    }

    #[test]
    fn a_card_paints_rust_inside_its_rim_and_leaves_its_corners_clear() {
        let show = Show::Reading;
        let card = laid(&show, "Retry it?").expect("a card");
        let pixmap = paint(&card_frame(&show, &card, &[]));
        let (width, height) = (pixmap.width(), card.layout.shape.height.ceil() as u32);
        let middle = height / 2;
        assert!(near(rgba(&pixmap, SIDE as u32, middle), RUST));
        assert!(near(rgba(&pixmap, 0, middle), bytes(RIM)));
        assert!(near(rgba(&pixmap, width - 1, middle), bytes(RIM)));
        let bottom = height - 1;
        for (x, y) in [(0, 0), (width - 1, 0), (0, bottom), (width - 1, bottom)] {
            assert_eq!(rgba(&pixmap, x, y)[3], 0, "({x}, {y})");
        }
        assert_eq!(rgba(&pixmap, width / 2, pixmap.height() - 1)[3], 0);
    }

    #[test]
    fn the_card_paints_bone_where_the_layout_puts_the_ask() {
        let show = Show::Reading;
        let card = laid(&show, "The build is red. Retry it?").expect("a card");
        let pixmap = paint(&Frame {
            scale: 2,
            ..card_frame(&show, &card, &[])
        });
        let ask = &block_of(&card.layout, card::Cut::Ask).lines[0];
        let line = ask.top + card::Cut::Ask.line_height();
        assert!(count(&pixmap, BONE, ask.top, line, 2.0) > 0);
        let divider = card.layout.divider.as_ref().expect("a divider");
        let (top, bottom) = (divider.top, divider.top + divider.height);
        assert_eq!(count(&pixmap, BONE, top, bottom, 2.0), 0);
        assert!(count(&pixmap, bytes(MUTED), top, bottom, 2.0) > 0);
    }

    #[test]
    fn the_sent_pill_paints_its_rim_and_sent() {
        let show = Show::Done(Done::Sent);
        let card = laid_from(&Scene::of(show.clone(), 1), false).expect("a layout");
        let scale = 2;
        let pixmap = paint(&Frame {
            scale,
            ..card_frame(&show, &card, &[])
        });
        let at = |logical: f64| (logical * f64::from(scale)) as u32;
        let width = card.layout.shape.width.ceil();
        assert_eq!(pixmap.width(), at(width));

        let stadium = rounded(width as f32, CAPSULE as f32, (CAPSULE / 2.0) as f32);
        let mut expected = Pixmap::new(pixmap.width(), pixmap.height()).expect("a frame's size");
        let placed = Transform::from_scale(scale as f32, scale as f32);
        expected.fill_path(&stadium, &ink(RIM, 1.0), FillRule::Winding, placed, None);
        for y in 0..pixmap.height() {
            for x in 0..pixmap.width() {
                let (found, wanted) = (rgba(&pixmap, x, y)[3], rgba(&expected, x, y)[3]);
                if wanted == 0 || wanted == 0xFF {
                    assert_eq!(found, wanted, "({x}, {y})");
                }
            }
        }

        assert!(near(rgba(&pixmap, at(0.75), at(CAPSULE / 2.0)), bytes(RIM)));

        let block = &card.layout.blocks[0];
        let sent = &block.lines[0];
        assert_eq!(sent.line, "Sent");
        let label = &card.lines[0][0];
        let (top, bottom) = (sent.top, sent.top + card::Cut::Label.line_height());
        let columns = at(sent.x)..at(sent.x + f64::from(label.width));
        let bone = (at(top)..at(bottom))
            .flat_map(|y| columns.clone().map(move |x| (x, y)))
            .filter(|&(x, y)| near(rgba(&pixmap, x, y), BONE))
            .count();
        assert!(bone > 0);
    }

    #[test]
    fn the_open_chip_is_a_muted_ring_around_bone() {
        let show = Show::Missed {
            count: 1,
            ask: "Retry the build?".into(),
        };
        let card = laid_from(&Scene::of(show.clone(), 1), false).expect("a layout");
        let scale = 2;
        let pixmap = paint(&Frame {
            scale,
            ..card_frame(&show, &card, &[])
        });
        let at = |logical: f64| logical * f64::from(scale);
        let chip = card.layout.chip.as_ref().expect("a chip");
        let ChipBox { x, top, height, .. } = *chip;
        let edge = rgba(&pixmap, at(x).ceil() as u32, at(top + height / 2.0) as u32);
        assert!(near(edge, bytes(MUTED)), "{edge:?}");

        let label = &chip.label;
        let open = words::shaped(&label.line, label.cut);
        let columns = at(label.x) as u32..at(label.x + f64::from(open.width)) as u32;
        let rows = at(label.top) as u32..at(label.top + label.cut.line_height()) as u32;
        let bone = rows
            .flat_map(|y| columns.clone().map(move |x| (x, y)))
            .filter(|&(x, y)| near(rgba(&pixmap, x, y), BONE))
            .count();
        assert!(bone > 0);
    }

    #[test]
    fn a_pill_shrinking_from_a_card_keeps_its_figure_at_its_bottom() {
        let from = 200.0;
        let pixmap = paint(&Frame {
            height_from: Some((from, 0.0)),
            ..frame(&Show::Recording(String::new()), None, &[])
        });
        assert_eq!(pixmap.height(), (TRAVEL + from) as u32);
        assert_eq!(count(&pixmap, BONE, 0.0, from - CAPSULE, 1.0), 0);
        assert!(count(&pixmap, BONE, from - CAPSULE, from, 1.0) > 0);
    }

    #[test]
    fn the_fade_leaves_the_top_of_a_cut_context_fainter_than_its_bottom() {
        let show = Show::Reading;
        let context = vec!["The migration renames the column."; 40].join(" ");
        let card = laid(&show, &format!("{context} Retry it?")).expect("a card");
        let pixmap = paint(&card_frame(&show, &card, &[]));
        let block = block_of(&card.layout, card::Cut::Context);
        assert_eq!(block.lines.len(), card::MAX_LINES);
        assert!(block.fade.is_some());
        let line = card::Cut::Context.line_height();
        let bone_in = |text: &card::Text| count(&pixmap, BONE, text.top, text.top + line, 1.0);
        let (first, last) = (&block.lines[0], &block.lines[card::MAX_LINES - 1]);
        assert!(
            bone_in(first) < bone_in(last),
            "{} {}",
            bone_in(first),
            bone_in(last)
        );
    }

    #[test]
    fn a_frame_paints_the_lines_shaped_with_the_layout() {
        let show = Show::Reading;
        let mut card = laid(&show, "Retry it?").expect("a card");
        let ask = &block_of(&card.layout, card::Cut::Ask).lines[0];
        let (top, bottom) = (ask.top, ask.top + card::Cut::Ask.line_height());
        let bone = |card: &Card| {
            count(
                &paint(&card_frame(&show, card, &[])),
                BONE,
                top,
                bottom,
                1.0,
            )
        };
        assert!(bone(&card) > 0);
        for line in card.lines.iter_mut().flatten() {
            line.path = None;
        }
        assert_eq!(bone(&card), 0, "a frame drew a line it shaped itself");
    }

    #[test]
    fn the_drain_is_bone_at_the_centre_and_muted_at_the_ends() {
        let show = Show::Answering("yes".into());
        let card = counting(&show, false, false);
        let divider = card.layout.divider.as_ref().expect("a divider");
        assert!(matches!(divider.kind, DividerKind::Drain { .. }));
        let row = (divider.top + divider.height / 2.0) as u32;
        let (centre, end) = ((card.layout.shape.width / 2.0) as u32, (SIDE + 2.0) as u32);
        let muted = bytes(MUTED);
        for (share, at_centre, at_end) in
            [(0.5, BONE, muted), (1.0, BONE, BONE), (0.0, muted, muted)]
        {
            let pixmap = paint(&Frame {
                countdown_share: Some(share),
                ..card_frame(&show, &card, &[])
            });
            let (found_centre, found_end) = (rgba(&pixmap, centre, row), rgba(&pixmap, end, row));
            assert!(near(found_centre, at_centre), "{share}: {found_centre:?}");
            assert!(near(found_end, at_end), "{share}: {found_end:?}");
        }
    }

    #[test]
    fn the_stepped_divider_lights_its_middle_segments() {
        let show = Show::Answering("yes".into());
        let card = counting(&show, true, false);
        let divider = card.layout.divider.as_ref().expect("a divider");
        assert!(matches!(divider.kind, DividerKind::Stepped { .. }));
        let row = (divider.top + divider.height / 2.0) as u32;
        let track = card.layout.shape.width - 2.0 * SIDE;
        let width = card::segment_width(track);
        let middle =
            |at: usize| (SIDE + at as f64 * (width + card::SEGMENT_GAP) + width / 2.0) as u32;
        let lit = |share: f64| {
            let pixmap = paint(&Frame {
                countdown_share: Some(share),
                ..card_frame(&show, &card, &[])
            });
            (0..card::SEGMENTS)
                .map(|at| near(rgba(&pixmap, middle(at), row), BONE))
                .collect::<Vec<_>>()
        };
        assert_eq!(lit(1.0), [true; 5]);
        assert_eq!(lit(0.5), [false, true, true, true, false]);
        assert_eq!(lit(0.0), [false; 5]);
    }

    #[test]
    fn the_hint_paints_under_the_words() {
        let show = Show::Answering("yes".into());
        let card = counting(&show, false, true);
        let hint = card
            .layout
            .blocks
            .last()
            .and_then(|block| block.lines.last())
            .expect("the hint");
        assert_eq!(hint.line, "Keep talking to add more");
        let pixmap = paint(&Frame {
            scale: 2,
            countdown_share: Some(0.5),
            ..card_frame(&show, &card, &[])
        });
        let bottom = hint.top + card::Cut::Hint.line_height();
        assert!(count(&pixmap, bytes(MUTED), hint.top, bottom, 2.0) > 0);
    }

    #[test]
    fn the_painted_card_draws_the_name_in_muted_ink() {
        let show = Show::Reading;
        let card = laid_from(
            &Scene {
                question: Some("Retry it?".into()),
                client: Some("claude".into()),
                ..Scene::of(show.clone(), 1)
            },
            false,
        )
        .expect("a card");
        let name = &card.layout.blocks[0];
        assert_eq!(name.lines[0].line, "claude");
        let pixmap = paint(&Frame {
            scale: 2,
            ..card_frame(&show, &card, &[])
        });
        let (top, bottom) = (name.top, name.top + name.height);
        assert!(count(&pixmap, bytes(MUTED), top, bottom, 2.0) > 0);
        assert_eq!(count(&pixmap, BONE, top, bottom, 2.0), 0);
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
        let line = words::line("Your turn", Trim::End, 1000.0);
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
    fn the_pulse_moves_each_dot_in_turn() {
        let dots = |seconds: f64| {
            Live::of(&frame(&Show::Working, None, &[(Move::Pulse, seconds)]))
                .dots
                .map(|dot| dot.opacity)
        };
        assert_eq!(dots(0.0), [0.45, 0.45, 0.45]);
        let later = dots(0.8);
        assert!((later[0] - 1.0).abs() < 0.01, "{later:?}");
        assert!(later[2] < 0.8, "{later:?}");
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
            let left = WORDS_LEFT as u32;
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
            let laid = laid(&show, "The build is red. Retry it?");
            for entrance in [Entrance::Fresh, Entrance::SameWords, Entrance::NewWords] {
                for calm in [false, true] {
                    let steps = moves(&Scene::of(show.clone(), 1), entrance, calm);
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
                        let Some(laid) = &laid else {
                            continue;
                        };
                        let card = |height_from| Frame {
                            card: Some(laid),
                            height_from,
                            level: clock.min(1.0),
                            ..frame
                        };
                        for height_from in [None, Some((CAPSULE, clock)), Some((400.0, clock))] {
                            let frame = card(height_from);
                            let pixmap = paint(&frame);
                            assert_eq!((pixmap.width(), pixmap.height()), size(&frame));
                        }
                    }
                }
            }
        }
    }
}
