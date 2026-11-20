//! The question card's layout: the text, the divider and the answer row as
//! positioned lines, independent of what paints them.

use std::ops::Range;
use std::time::{Duration, Instant};

use crate::chip::{
    Countdown, Scene, Show, Trim, WORKING, YOUR_TURN, YOUR_TURN_NEXT, missed_label, segments_lit,
    split,
};
use crate::figure::{BONE, CAPSULE};
use crate::glyphs::{GLYPH, Glyph, SLOT};

pub const MUTED: u32 = 0xE3B8A4;
pub const RIM: u32 = 0xE2673D;
pub const RIM_WIDTH: f64 = 1.5;
pub const CARD_WIDTH: f64 = 560.0;
pub const NARROW_WIDTH: f64 = 440.0;
pub const CARD_RADIUS: f64 = 20.0;
pub const SIDE: f64 = 20.0;
pub const TOP: f64 = 20.0;
pub const BODY_BOTTOM: f64 = 14.0;
pub const NAME_GAP: f64 = 8.0;
pub const CONTEXT_GAP: f64 = 8.0;
pub const AFTER_GAP: f64 = 6.0;
pub const MORE_GAP: f64 = 4.0;
pub const MAX_LINES: usize = 4;
/// The clamp's mask, in em of the block's size.
pub const FADE_EM: f64 = 2.2;
pub const ANSWER_TOP: f64 = 14.0;
pub const ANSWER_BOTTOM: f64 = 20.0;
pub const ANSWER_LINES: usize = 2;
pub const ANSWER_FADE_EM: f64 = 1.1;
pub const GLYPH_GAP: f64 = 10.0;
pub const TEACH_GAP: f64 = 3.0;
pub const DIVIDER_THIN: f64 = 1.0;
pub const DIVIDER: f64 = 2.0;
/// The 2 px track with a 1 px overhang each side.
pub const DRAIN_HEIGHT: f64 = 4.0;
pub const DRAIN_RADIUS: f64 = 2.0;
pub const TRACK_RADIUS: f64 = 1.0;
pub const SEGMENTS: usize = 5;
pub const SEGMENT_GAP: f64 = 4.0;
pub const SEGMENT_HEIGHT: f64 = 3.0;
pub const SEGMENT_RADIUS: f64 = 1.5;
pub const SEGMENT_OFF: u32 = 0xBF8E7D;
pub const MISSED_PAD: Pad = Pad {
    top: 9.0,
    right: 12.0,
    bottom: 9.0,
    left: 20.0,
};
pub const STACK_GAP: f64 = 1.0;
pub const CHIP_PAD_X: f64 = 9.0;
pub const CHIP_PAD_Y: f64 = 1.0;
pub const CHIP_BORDER: f64 = 1.0;
/// The chip's margin on top of `GLYPH_GAP`.
pub const CHIP_GAP: f64 = 4.0;

pub const ELLIPSIS: char = '\u{2026}';
const TEACH: &str = "Keep talking to add more";
const HINT: &str = "Start of the question is in History";

pub struct Pad {
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
    pub left: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Cut {
    Context,
    Ask,
    User,
    Label,
    Hint,
    Chip,
    Name,
}

impl Cut {
    pub const ALL: [Cut; 7] = [
        Cut::Context,
        Cut::Ask,
        Cut::User,
        Cut::Label,
        Cut::Hint,
        Cut::Chip,
        Cut::Name,
    ];

    pub fn weight(self) -> f64 {
        match self {
            Cut::Context | Cut::Ask | Cut::Hint => 350.0,
            Cut::User => 800.0,
            Cut::Label | Cut::Name => 650.0,
            Cut::Chip => 600.0,
        }
    }

    pub fn width(self) -> f64 {
        match self {
            Cut::Context | Cut::Ask | Cut::Hint => 84.0,
            Cut::User => 112.0,
            Cut::Label | Cut::Chip | Cut::Name => 100.0,
        }
    }

    pub fn size(self) -> f64 {
        match self {
            Cut::Context | Cut::User | Cut::Label => 15.0,
            Cut::Ask => 22.0,
            Cut::Hint | Cut::Chip | Cut::Name => 13.0,
        }
    }

    /// In em.
    pub fn tracking(self) -> f64 {
        match self {
            Cut::Ask | Cut::User => -0.01,
            Cut::Context | Cut::Label | Cut::Hint | Cut::Chip | Cut::Name => 0.0,
        }
    }

    pub fn line_height(self) -> f64 {
        self.size()
            * match self {
                Cut::Context => 1.45,
                Cut::Ask => 1.25,
                Cut::User | Cut::Label | Cut::Hint | Cut::Chip | Cut::Name => 1.35,
            }
    }
}

pub trait Measure {
    fn width(&self, text: &str, cut: Cut) -> f64;
    /// The narrowest advance of one character in `cut`: the floor of every bound below.
    fn narrowest(&self, cut: Cut) -> f64;
}

pub fn narrowest_ascii(width: impl Fn(&str) -> f64) -> f64 {
    (' '..='~')
        .map(|one| width(one.encode_utf8(&mut [0; 4])))
        .fold(f64::INFINITY, f64::min)
}

/// The card text's room in `available` points of screen width.
pub fn room(available: f64) -> f64 {
    available.min(CARD_WIDTH) - 2.0 * SIDE
}

/// 1 at the start of the countdown, 0 at its end.
pub fn share_left(countdown: Countdown, now: Instant) -> f64 {
    let spent = now.saturating_duration_since(countdown.since);
    (1.0 - spent.as_secs_f64() / countdown.lasts.as_secs_f64()).clamp(0.0, 1.0)
}

/// The middle `lit` of the `SEGMENTS`, so they go out from both ends.
pub fn lit_segments(lit: usize) -> Range<usize> {
    let first = (SEGMENTS - lit) / 2;
    first..first + lit
}

/// One segment's width on a `track` wide divider; the next starts
/// `SEGMENT_GAP` after it.
pub fn segment_width(track: f64) -> f64 {
    (track - SEGMENT_GAP * (SEGMENTS - 1) as f64) / SEGMENTS as f64
}

pub fn most_chars(room: f64, narrowest: f64, lines: usize) -> usize {
    (room / narrowest).ceil() as usize * lines
}

/// The last `most` characters, cut at a space; the bool says whether anything was cut.
pub fn tail(text: &str, most: usize) -> (String, bool) {
    let total = text.chars().count();
    if total <= most {
        return (text.to_string(), false);
    }
    let skip = total - most;
    let from = text
        .char_indices()
        .nth(skip)
        .map_or(text.len(), |(at, _)| at);
    let kept = &text[from..];
    let after_space = text[..from].ends_with(' ');
    let kept = match kept.find(' ') {
        Some(space) if !after_space => &kept[space + 1..],
        _ => kept,
    };
    (kept.to_string(), true)
}

pub fn wrap(text: &str, room: f64, width: impl Fn(&str) -> f64) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for word in text.split_whitespace() {
        if let Some(line) = lines.last_mut() {
            let joined = format!("{line} {word}");
            if width(&joined) <= room {
                *line = joined;
                continue;
            }
        }
        lines.push(word.to_string());
    }
    lines
}

pub fn clamp(lines: Vec<String>, most: usize) -> (Vec<String>, bool) {
    let cut = lines.len() > most;
    let skip = lines.len().saturating_sub(most);
    (lines.into_iter().skip(skip).collect(), cut)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shape {
    pub width: f64,
    pub height: f64,
    pub radius: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ink {
    Bone,
    Muted,
}

impl Ink {
    pub fn hex(self) -> u32 {
        match self {
            Ink::Bone => BONE,
            Ink::Muted => MUTED,
        }
    }
}

/// One line, centred in a line box `height` tall.
pub struct Text {
    pub line: String,
    pub cut: Cut,
    pub ink: Ink,
    pub x: f64,
    pub top: f64,
    pub height: f64,
}

pub struct Block {
    pub lines: Vec<Text>,
    pub top: f64,
    pub height: f64,
    pub fade: Option<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DividerKind {
    Thin,
    Drain {
        share: f64,
        lasts: Duration,
        since: Instant,
    },
    Stepped {
        lit: usize,
    },
}

pub struct Divider {
    pub top: f64,
    pub height: f64,
    pub kind: DividerKind,
}

/// The Open chip.
pub struct ChipBox {
    pub label: Text,
    pub x: f64,
    pub top: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlacedGlyph {
    pub glyph: Glyph,
    pub x: f64,
    pub top: f64,
}

pub struct Layout {
    pub shape: Shape,
    pub blocks: Vec<Block>,
    pub divider: Option<Divider>,
    pub glyph: Option<PlacedGlyph>,
    pub chip: Option<ChipBox>,
}

impl Layout {
    pub fn follows_the_level(&self) -> bool {
        self.glyph.is_some_and(|placed| placed.glyph == Glyph::Bars)
    }
}

fn set(
    text: &str,
    cut: Cut,
    room: f64,
    lines: usize,
    measure: &dyn Measure,
) -> (Vec<String>, bool) {
    let (kept, shortened) = tail(text, most_chars(room, measure.narrowest(cut), lines));
    let (lines, clamped) = clamp(wrap(&kept, room, |line| measure.width(line, cut)), lines);
    (lines, shortened || clamped)
}

fn block(lines: Vec<String>, cut: Cut, ink: Ink, x: f64, top: f64, fade: Option<f64>) -> Block {
    let step = cut.line_height();
    Block {
        height: lines.len() as f64 * step,
        lines: lines
            .into_iter()
            .enumerate()
            .map(|(index, line)| Text {
                line,
                cut,
                ink,
                x,
                top: top + index as f64 * step,
                height: step,
            })
            .collect(),
        top,
        fade,
    }
}

fn answer_of(show: &Show) -> Option<(Glyph, &str, Cut)> {
    match show {
        Show::Reading => Some((Glyph::Arcs, YOUR_TURN_NEXT, Cut::Label)),
        Show::YourTurn => Some((Glyph::Bars, YOUR_TURN, Cut::Label)),
        Show::Answering(words) | Show::Recording(words) if !words.is_empty() => {
            Some((Glyph::Bars, words, Cut::User))
        }
        Show::Answering(_) | Show::Recording(_) => Some((Glyph::Bars, YOUR_TURN, Cut::Label)),
        Show::Working => Some((Glyph::Dots, WORKING, Cut::Label)),
        Show::NothingHeard(_)
        | Show::Broken(_)
        | Show::Done(_)
        | Show::Copied
        | Show::Missed { .. } => None,
    }
}

/// `room` is the width the text may take. `None` for a scene the figure pill
/// draws.
pub fn layout(
    scene: &Scene,
    room: f64,
    calm: bool,
    now: Instant,
    measure: &dyn Measure,
) -> Option<Layout> {
    let Some(question) = scene.question.as_deref() else {
        return match &scene.show {
            Show::Missed { count, ask } => Some(missed_pill(*count, ask, measure)),
            _ => None,
        };
    };
    let (glyph, words, words_cut) = answer_of(&scene.show)?;
    let parts = split(question);
    let width = room + 2.0 * SIDE;
    let mut blocks = Vec::new();
    let mut y = TOP;

    if let Some(client) = scene.client.as_deref() {
        let line = fit(client, room, Trim::End, |line| {
            measure.width(line, Cut::Name)
        });
        let name = block(vec![line], Cut::Name, Ink::Muted, SIDE, SIDE, None);
        y = SIDE + name.height + NAME_GAP;
        blocks.push(name);
    }

    let (context, context_cut) = set(&parts.context, Cut::Context, room, MAX_LINES, measure);
    if !context.is_empty() {
        if context_cut {
            let hint = block(vec![HINT.into()], Cut::Hint, Ink::Muted, SIDE, y, None);
            y += hint.height + MORE_GAP;
            blocks.push(hint);
        }
        let fade = context_cut.then(|| FADE_EM * Cut::Context.size());
        let shown = block(context, Cut::Context, Ink::Bone, SIDE, y, fade);
        y += shown.height + CONTEXT_GAP;
        blocks.push(shown);
    }

    let (ask, ask_cut) = set(&parts.ask, Cut::Ask, room, MAX_LINES, measure);
    let fade = ask_cut.then(|| FADE_EM * Cut::Ask.size());
    let shown = block(ask, Cut::Ask, Ink::Bone, SIDE, y, fade);
    y += shown.height;
    blocks.push(shown);

    let (after, _) = set(&parts.after, Cut::Context, room, MAX_LINES, measure);
    if !after.is_empty() {
        y += AFTER_GAP;
        let shown = block(after, Cut::Context, Ink::Bone, SIDE, y, None);
        y += shown.height;
        blocks.push(shown);
    }

    y = (y + BODY_BOTTOM).round();
    let divider = match scene.countdown {
        Some(countdown) => {
            let share = share_left(countdown, now);
            if calm {
                Divider {
                    top: y,
                    height: SEGMENT_HEIGHT,
                    kind: DividerKind::Stepped {
                        lit: segments_lit(share),
                    },
                }
            } else {
                Divider {
                    top: y,
                    height: DIVIDER,
                    kind: DividerKind::Drain {
                        share,
                        lasts: countdown.lasts,
                        since: countdown.since,
                    },
                }
            }
        }
        None => Divider {
            top: y,
            height: DIVIDER_THIN,
            kind: DividerKind::Thin,
        },
    };
    y += divider.height;
    let glyph = if scene.countdown.is_some() {
        Glyph::BarsQuiet
    } else {
        glyph
    };

    let words_x = SIDE + SLOT + GLYPH_GAP;
    let (lines, words_clamped) = set(
        words,
        words_cut,
        room - SLOT - GLYPH_GAP,
        ANSWER_LINES,
        measure,
    );
    let fade = words_clamped.then(|| ANSWER_FADE_EM * words_cut.size());
    let content_top = y + ANSWER_TOP;
    let mut shown = block(lines, words_cut, Ink::Bone, words_x, content_top, fade);
    if scene.countdown.is_some_and(|countdown| countdown.teach) {
        let top = content_top + shown.height + TEACH_GAP;
        let hint = block(
            vec![TEACH.into()],
            Cut::Hint,
            Ink::Muted,
            words_x,
            top,
            None,
        );
        shown.height += TEACH_GAP + hint.height;
        shown.lines.extend(hint.lines);
    }
    let row = ANSWER_TOP + GLYPH.max(shown.height) + ANSWER_BOTTOM;
    blocks.push(shown);

    Some(Layout {
        shape: Shape {
            width,
            height: y + row,
            radius: CARD_RADIUS,
        },
        blocks,
        divider: Some(divider),
        glyph: Some(PlacedGlyph {
            glyph,
            x: SIDE,
            top: content_top,
        }),
        chip: None,
    })
}

fn missed_pill(count: u32, ask: &str, measure: &dyn Measure) -> Layout {
    let label = missed_label(count);
    let chip_text = "Open";
    let chip_width = measure.width(chip_text, Cut::Chip) + 2.0 * (CHIP_PAD_X + CHIP_BORDER);
    let chip_height = Cut::Chip.line_height() + 2.0 * (CHIP_PAD_Y + CHIP_BORDER);
    let side = GLYPH_GAP + CHIP_GAP + chip_width + MISSED_PAD.right;
    let label_width = measure.width(&label, Cut::Label);
    let room = NARROW_WIDTH - MISSED_PAD.left - side;
    let kept = fit(ask, room, Trim::End, |line| {
        measure.width(line, Cut::Context)
    });
    let cut = kept != ask;
    let lines = if kept.is_empty() { 1.0 } else { 2.0 };
    let line = Cut::Label.line_height();
    let stack = lines * line + (lines - 1.0) * STACK_GAP;
    let height = (MISSED_PAD.top + stack + MISSED_PAD.bottom).max(CAPSULE);
    let first = (height - stack) / 2.0;
    let mut texts = vec![Text {
        line: label,
        cut: Cut::Label,
        ink: Ink::Bone,
        x: MISSED_PAD.left,
        top: first,
        height: line,
    }];
    let mut stack_width = label_width;
    if !kept.is_empty() {
        stack_width = stack_width.max(measure.width(&kept, Cut::Context));
        if cut {
            stack_width = room;
        }
        texts.push(Text {
            line: kept,
            cut: Cut::Context,
            ink: Ink::Bone,
            x: MISSED_PAD.left,
            top: first + line + STACK_GAP,
            height: line,
        });
    }
    let chip_x = MISSED_PAD.left + stack_width + GLYPH_GAP + CHIP_GAP;
    let chip_top = (height - chip_height) / 2.0;
    Layout {
        shape: Shape {
            width: (chip_x + chip_width + MISSED_PAD.right).min(NARROW_WIDTH),
            height,
            radius: if height <= CAPSULE {
                height / 2.0
            } else {
                CARD_RADIUS
            },
        },
        blocks: vec![Block {
            lines: texts,
            top: 0.0,
            height,
            fade: None,
        }],
        divider: None,
        glyph: None,
        chip: Some(ChipBox {
            label: Text {
                line: chip_text.to_string(),
                cut: Cut::Chip,
                ink: Ink::Bone,
                x: chip_x + CHIP_BORDER + CHIP_PAD_X,
                top: chip_top + CHIP_BORDER + CHIP_PAD_Y,
                height: Cut::Chip.line_height(),
            },
            x: chip_x,
            top: chip_top,
            width: chip_width,
            height: chip_height,
        }),
    }
}

/// `text` cut to `room` under `width`, with an ellipsis where `trim` cut it.
pub fn fit(text: &str, room: f64, trim: Trim, width: impl Fn(&str) -> f64) -> String {
    if width(text) <= room {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let keeping = |count: usize| match trim {
        Trim::End => {
            let head: String = chars[..count].iter().collect();
            format!("{}{ELLIPSIS}", head.trim_end())
        }
        Trim::Start => {
            let tail: String = chars[chars.len() - count..].iter().collect();
            format!("{ELLIPSIS}{}", tail.trim_start())
        }
    };
    let (mut fits, mut over) = (0, chars.len());
    while over - fits > 1 {
        let middle = fits + (over - fits) / 2;
        if width(&keeping(middle)) <= room {
            fits = middle;
        } else {
            over = middle;
        }
    }
    keeping(fits)
}

/// A surface holds the card at its top, the clear space below it, and room on
/// both sides during a width transition. Only a point in the rounded shape,
/// centred in `view_width`, counts. `y` runs down from the top.
pub fn in_shape(view_width: f64, shape: Shape, x: f64, y: f64) -> bool {
    let radius = shape.radius;
    let x = x - (view_width - shape.width) / 2.0;
    let centre_x = x.clamp(radius, (shape.width - radius).max(radius));
    let centre_y = y.clamp(radius, (shape.height - radius).max(radius));
    (x - centre_x).hypot(y - centre_y) <= radius
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    use crate::chip::{Countdown, Done, Scene, Show};
    use crate::figure::{CAPSULE, TRAVEL};
    use crate::glyphs::{GLYPH, Glyph};
    use std::cell::Cell;

    struct Mono {
        longest: Cell<usize>,
        marked: Cell<bool>,
    }

    impl Mono {
        fn new() -> Self {
            Mono {
                longest: Cell::new(0),
                marked: Cell::new(false),
            }
        }
    }

    impl Measure for Mono {
        fn width(&self, text: &str, _cut: Cut) -> f64 {
            let count = text.chars().count();
            self.longest.set(self.longest.get().max(count));
            if text.contains("MARKER") {
                self.marked.set(true);
            }
            count as f64 * 8.0
        }

        fn narrowest(&self, _cut: Cut) -> f64 {
            8.0
        }
    }

    const ROOM: f64 = CARD_WIDTH - 2.0 * SIDE;

    fn close(left: f64, right: f64) -> bool {
        (left - right).abs() < 1e-9
    }

    fn scene(show: Show, question: &str) -> Scene {
        Scene {
            question: Some(question.into()),
            ..Scene::of(show, 1)
        }
    }

    fn laid(show: Show, question: &str) -> Layout {
        layout(
            &scene(show, question),
            ROOM,
            false,
            Instant::now(),
            &Mono::new(),
        )
        .unwrap()
    }

    fn asked_by(client: &str) -> Scene {
        Scene {
            client: Some(client.into()),
            ..scene(Show::Reading, "The build is red. Retry it?")
        }
    }

    #[test]
    fn the_name_is_the_first_line_in_muted_ink_and_the_text_follows_eight_below() {
        let card = layout(
            &asked_by("claude"),
            ROOM,
            false,
            Instant::now(),
            &Mono::new(),
        )
        .unwrap();
        let name = &card.blocks[0];
        assert_eq!(name.lines.len(), 1);
        assert_eq!(
            (
                name.lines[0].line.as_str(),
                name.lines[0].cut,
                name.lines[0].ink
            ),
            ("claude", Cut::Name, Ink::Muted)
        );
        assert_eq!((name.lines[0].x, name.top), (SIDE, SIDE));
        assert!(close(
            card.blocks[1].top,
            SIDE + Cut::Name.line_height() + NAME_GAP
        ));
    }

    #[test]
    fn a_named_card_sets_its_name_at_the_side_inset_and_is_no_wider() {
        let named = layout(
            &asked_by("claude"),
            ROOM,
            false,
            Instant::now(),
            &Mono::new(),
        )
        .unwrap();
        let bare = laid(Show::Reading, "The build is red. Retry it?");
        let grown = named.shape.height - bare.shape.height;
        assert!((grown - (SIDE + Cut::Name.line_height() + NAME_GAP - TOP)).abs() <= 0.5);
        assert_eq!(named.shape.width, bare.shape.width);
        assert_eq!(named.blocks.len(), bare.blocks.len() + 1);
    }

    #[test]
    fn a_long_name_stays_one_line_cut_at_the_end() {
        let mono = Mono::new();
        let card = layout(
            &asked_by(&"a".repeat(200)),
            ROOM,
            false,
            Instant::now(),
            &mono,
        )
        .unwrap();
        let name = &card.blocks[0];
        assert_eq!(name.lines.len(), 1);
        let line = &name.lines[0].line;
        assert!(line.starts_with('a') && line.ends_with(ELLIPSIS));
        assert!(mono.width(line, Cut::Name) <= ROOM);
    }

    #[test]
    fn the_name_cut_is_banshees_label_at_hint_size() {
        assert_eq!(
            (
                Cut::Name.weight(),
                Cut::Name.width(),
                Cut::Name.size(),
                Cut::Name.tracking()
            ),
            (650.0, 100.0, 13.0, 0.0)
        );
        assert!(close(Cut::Name.line_height(), 13.0 * 1.35));
        assert_eq!(Cut::ALL[Cut::Name as usize], Cut::Name);
    }

    fn block_of(layout: &Layout, cut: Cut) -> &Block {
        layout
            .blocks
            .iter()
            .find(|block| block.lines.first().is_some_and(|text| text.cut == cut))
            .unwrap()
    }

    #[test]
    fn wrap_breaks_at_spaces_only() {
        let mono = |text: &str| text.chars().count() as f64 * 8.0;
        assert_eq!(
            wrap("one two three four", 80.0, mono),
            ["one two", "three four"]
        );
        assert_eq!(
            wrap("extraordinarily long", 80.0, mono),
            ["extraordinarily", "long"]
        );
        assert!(
            wrap("a  b   c", 8.0, mono)
                .iter()
                .all(|line| !line.is_empty())
        );
    }

    #[test]
    fn clamp_keeps_the_last_lines_and_says_so() {
        let lines = |names: &[&str]| {
            names
                .iter()
                .map(|name| name.to_string())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            clamp(lines(&["a", "b", "c", "d", "e"]), 4),
            (lines(&["b", "c", "d", "e"]), true)
        );
        assert_eq!(
            clamp(lines(&["a", "b", "c", "d"]), 4),
            (lines(&["a", "b", "c", "d"]), false)
        );
    }

    #[test]
    fn a_hundred_kilobyte_question_wraps_only_its_tail() {
        let mut question = format!("MARKER {}", vec!["xxxxxxxxx"; 10_000].join(" "));
        question.push_str(". Which one?");
        let mono = Mono::new();
        let card = layout(
            &scene(Show::Reading, &question),
            ROOM,
            false,
            Instant::now(),
            &mono,
        )
        .unwrap();

        let context = block_of(&card, Cut::Context);
        assert_eq!(context.lines.len(), MAX_LINES);
        assert!(context.fade.is_some());
        assert!(
            card.blocks
                .iter()
                .flat_map(|block| &block.lines)
                .any(|text| text.cut == Cut::Hint
                    && text.line == "Start of the question is in History")
        );
        assert!(mono.longest.get() <= most_chars(ROOM, 8.0, MAX_LINES) + 1);
        assert!(!mono.marked.get());
    }

    #[test]
    fn a_short_question_shows_no_fade_and_no_note() {
        let card = laid(Show::Reading, "The build is red. Retry it?");
        assert!(
            card.blocks
                .iter()
                .flat_map(|block| &block.lines)
                .all(|text| text.cut != Cut::Hint)
        );
        assert_eq!(block_of(&card, Cut::Context).fade, None);
        assert_eq!(block_of(&card, Cut::Ask).fade, None);
    }

    #[test]
    fn the_card_stacks_from_the_top_to_the_answer_row() {
        let card = laid(Show::Reading, "The build is red. Retry it?");
        assert!(close(card.blocks[0].top, TOP));
        let ask = block_of(&card, Cut::Ask);
        assert!(close(ask.top, TOP + 21.75 + CONTEXT_GAP));
        let divider = card.divider.as_ref().unwrap();
        assert!(close(divider.top, (ask.top + 27.5 + BODY_BOTTOM).round()));
        assert!(close(divider.height, DIVIDER_THIN));
        let PlacedGlyph { x, top, .. } = card.glyph.unwrap();
        assert!(close(x, SIDE));
        assert!(close(top, divider.top + 1.0 + ANSWER_TOP));
        assert!(close(card.shape.height, top + 20.25 + 20.0));
        assert!(close(card.shape.width, CARD_WIDTH));
        assert!(close(card.shape.radius, CARD_RADIUS));
    }

    #[test]
    fn the_card_sits_evenly_inside_its_edges() {
        let bare = laid(Show::Reading, "The build is red. Retry it?");
        assert!(close(bare.blocks[0].top, 20.0));
        let named = layout(
            &asked_by("claude"),
            ROOM,
            false,
            Instant::now(),
            &Mono::new(),
        )
        .unwrap();
        assert!(close(named.blocks[0].top, 20.0));
        for card in [&bare, &named] {
            let words = card.blocks.last().unwrap();
            let bottom = words.top + words.height.max(GLYPH);
            assert!(close(card.shape.height, bottom + 20.0));
            let top = card.divider.as_ref().unwrap().top;
            assert!(close(top, top.round()));
        }
    }

    fn row_of(show: Show) -> (Glyph, Vec<String>, Cut) {
        let card = laid(show, "Retry it?");
        let words = card.blocks.last().unwrap();
        (
            card.glyph.unwrap().glyph,
            words.lines.iter().map(|text| text.line.clone()).collect(),
            words.lines[0].cut,
        )
    }

    #[test]
    fn the_answer_row_follows_the_show() {
        assert_eq!(
            row_of(Show::Reading),
            (Glyph::Arcs, vec!["Your turn next".into()], Cut::Label)
        );
        assert_eq!(
            row_of(Show::YourTurn),
            (Glyph::Bars, vec!["Your turn".into()], Cut::Label)
        );
        assert_eq!(
            row_of(Show::Answering("yes".into())),
            (Glyph::Bars, vec!["yes".into()], Cut::User)
        );
        assert_eq!(
            row_of(Show::Working),
            (Glyph::Dots, vec!["Working".into()], Cut::Label)
        );
    }

    #[test]
    fn a_long_answer_keeps_its_last_two_lines() {
        let words = vec!["alpha"; 22].join(" ");
        let card = laid(Show::Answering(words), "Retry it?");
        let block = card.blocks.last().unwrap();
        assert_eq!(block.lines.len(), ANSWER_LINES);
        assert_eq!(block.fade, Some(ANSWER_FADE_EM * 15.0));
    }

    #[test]
    fn a_scene_with_no_question_has_no_layout() {
        assert!(
            layout(
                &Scene::of(Show::YourTurn, 1),
                ROOM,
                false,
                Instant::now(),
                &Mono::new()
            )
            .is_none()
        );
    }

    #[test]
    fn sent_is_a_figure_pill_with_no_layout() {
        let scene = Scene::of(Show::Done(Done::Sent), 1);
        assert!(layout(&scene, ROOM, false, Instant::now(), &Mono::new()).is_none());
    }

    fn missed(count: u32, ask: &str) -> Layout {
        let scene = Scene::of(
            Show::Missed {
                count,
                ask: ask.into(),
            },
            1,
        );
        layout(&scene, ROOM, false, Instant::now(), &Mono::new()).expect("a pill")
    }

    #[test]
    fn the_missed_pill_stacks_its_label_the_ask_and_open() {
        let card = missed(1, &"q".repeat(60));
        let lines: Vec<&Text> = card.blocks.iter().flat_map(|b| &b.lines).collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(
            (lines[0].line.as_str(), lines[0].cut),
            ("You missed a question", Cut::Label)
        );
        assert_eq!(lines[1].cut, Cut::Context);
        assert!(lines[1].line.ends_with('\u{2026}'));
        assert!(lines[1].line.starts_with("qqq"));
        let chip = card.chip.expect("a chip");
        assert_eq!(
            (chip.label.line.as_str(), chip.label.cut),
            ("Open", Cut::Chip)
        );
        let stack = lines
            .iter()
            .map(|text| text.x + Mono::new().width(&text.line, text.cut))
            .fold(0.0, f64::max);
        let gap = chip.x - stack;
        assert!((GLYPH_GAP + CHIP_GAP..GLYPH_GAP + CHIP_GAP + 8.0).contains(&gap));
        assert_eq!(card.shape.radius, CARD_RADIUS);
        assert!(close(
            card.shape.height,
            MISSED_PAD.top + 20.25 + STACK_GAP + 20.25 + MISSED_PAD.bottom
        ));
        assert!(close(lines[1].top - lines[0].top, 20.25 + STACK_GAP));
    }

    #[test]
    fn a_missed_pill_with_no_ask_is_one_line_tall() {
        let card = missed(1, "");
        assert_eq!(card.shape.height, CAPSULE);
        assert_eq!(card.shape.radius, card.shape.height / 2.0);
        assert_eq!(card.blocks.iter().flat_map(|b| &b.lines).count(), 1);
    }

    #[test]
    fn the_missed_pill_never_exceeds_the_narrow_width() {
        let card = missed(3, &"x".repeat(200));
        assert_eq!(card.shape.width, NARROW_WIDTH);
        let lines: Vec<&Text> = card.blocks.iter().flat_map(|b| &b.lines).collect();
        assert_eq!(lines[0].line, "You missed 3 questions");
        assert!(lines[1].line.ends_with('\u{2026}'));
        let chip = card.chip.expect("a chip");
        assert!(close(chip.x + chip.width + MISSED_PAD.right, NARROW_WIDTH));
    }

    #[test]
    fn a_short_ask_keeps_the_pill_snug() {
        let card = missed(1, "Ready?");
        assert!(card.shape.width < NARROW_WIDTH);
        let chip = card.chip.expect("a chip");
        assert!(close(
            chip.x + chip.width + MISSED_PAD.right,
            card.shape.width
        ));
    }

    fn pill(width: f64) -> Shape {
        Shape {
            width,
            height: CAPSULE,
            radius: CAPSULE / 2.0,
        }
    }

    #[test]
    fn a_click_lands_only_inside_the_capsules_rounded_shape() {
        for (x, y) in [(2.0, 22.0), (98.0, 22.0), (50.0, 22.0)] {
            assert!(in_shape(100.0, pill(100.0), x, y), "({x}, {y})");
        }
        for (x, y) in [(0.0, 0.0), (100.0, 0.0), (0.0, 44.0), (100.0, 44.0)] {
            assert!(!in_shape(100.0, pill(100.0), x, y), "({x}, {y})");
        }
    }

    #[test]
    fn a_click_in_the_clear_space_below_the_capsule_misses() {
        for y in [
            CAPSULE + 1.0,
            CAPSULE + TRAVEL / 2.0,
            CAPSULE + TRAVEL - 1.0,
        ] {
            assert!(!in_shape(100.0, pill(100.0), 50.0, y), "y {y}");
        }
    }

    #[test]
    fn a_click_beside_a_narrower_centred_capsule_misses() {
        for x in [2.0, 18.0, 122.0, 138.0] {
            assert!(!in_shape(140.0, pill(100.0), x, 22.0), "x {x}");
        }
        for x in [22.0, 70.0, 118.0] {
            assert!(in_shape(140.0, pill(100.0), x, 22.0), "x {x}");
        }
    }

    fn countdown(millis: u64) -> Countdown {
        Countdown {
            since: Instant::now(),
            lasts: Duration::from_millis(millis),
            teach: false,
        }
    }

    #[test]
    fn the_share_runs_from_one_to_nothing() {
        let c = countdown(2000);
        assert_eq!(share_left(c, c.since), 1.0);
        assert_eq!(share_left(c, c.since + c.lasts / 2), 0.5);
        assert_eq!(share_left(c, c.since + c.lasts), 0.0);
        assert_eq!(share_left(c, c.since + c.lasts * 2), 0.0);
    }

    #[test]
    fn the_lit_segments_are_the_middle_ones() {
        assert_eq!(lit_segments(5), 0..5);
        assert_eq!(lit_segments(3), 1..4);
        assert_eq!(lit_segments(1), 2..3);
        assert!(lit_segments(0).is_empty());
    }

    fn counting_in(show: Show, calm: bool, teach: bool) -> Layout {
        let c = Countdown {
            teach,
            ..countdown(2000)
        };
        let now = c.since + c.lasts / 2;
        let scene = Scene {
            countdown: Some(c),
            ..scene(show, "Retry it?")
        };
        layout(&scene, ROOM, calm, now, &Mono::new()).unwrap()
    }

    fn counting(calm: bool) -> Layout {
        counting_in(Show::Answering("yes".into()), calm, false)
    }

    #[test]
    fn a_countdown_turns_the_divider_into_a_drain() {
        let card = counting(false);
        let divider = card.divider.unwrap();
        assert!(matches!(divider.kind, DividerKind::Drain { share, .. } if close(share, 0.5)));
        assert!(close(divider.height, DIVIDER));
        assert_eq!(card.glyph.unwrap().glyph, Glyph::BarsQuiet);
    }

    #[test]
    fn the_bars_follow_the_level_until_a_countdown_quiets_them() {
        assert!(laid(Show::YourTurn, "Retry it?").follows_the_level());
        assert!(laid(Show::Recording("yes".into()), "Retry it?").follows_the_level());
        assert!(!counting(false).follows_the_level());
        assert!(!laid(Show::Reading, "Retry it?").follows_the_level());
    }

    #[test]
    fn calm_steps_the_divider() {
        let card = counting(true);
        let divider = card.divider.unwrap();
        assert!(matches!(divider.kind, DividerKind::Stepped { lit: 3 }));
        assert!(close(divider.height, SEGMENT_HEIGHT));
    }

    #[test]
    fn no_countdown_is_a_thin_divider() {
        let card = laid(Show::YourTurn, "Retry it?");
        let divider = card.divider.unwrap();
        assert!(matches!(divider.kind, DividerKind::Thin));
        assert!(close(divider.height, DIVIDER_THIN));
        assert_eq!(card.glyph.unwrap().glyph, Glyph::Bars);
    }

    #[test]
    fn the_hint_sits_under_the_words() {
        let words = Show::Answering(vec!["alpha"; 22].join(" "));
        let plain = counting_in(words.clone(), false, false);
        let taught = counting_in(words, false, true);
        let answer = taught.blocks.last().unwrap();
        let hint = answer.lines.last().unwrap();
        assert_eq!(hint.line, "Keep talking to add more");
        assert_eq!(hint.cut, Cut::Hint);
        assert_eq!(hint.ink, Ink::Muted);
        assert_eq!(taught.blocks.len(), plain.blocks.len());
        let words = &plain.blocks.last().unwrap();
        assert!(close(hint.top, words.top + words.height + TEACH_GAP));
        assert!(close(answer.height, words.height + TEACH_GAP + 17.55));
        assert!(close(
            taught.shape.height,
            plain.shape.height + TEACH_GAP + 17.55
        ));
    }
}
