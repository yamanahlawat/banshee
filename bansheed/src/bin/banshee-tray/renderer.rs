//! What the capsule shows from moment to moment, with time passed in and no
//! Wayland types.

use std::time::{Duration, Instant};

use tiny_skia::Pixmap;

use crate::card::{self, Shape};
use crate::chip::{Scene, words_of};
use crate::figure::{self, AROUND_WORDS, LEVEL_STEP, Pose, SCREEN_MARGIN, Voice, seconds};
use crate::gesture;
use crate::paint::{self, Card, Frame};
use crate::scenes::{Entrance, Move, answer_opens, holds_the_voice, leaving, moves, takes_clicks};
use crate::words::{self, Line};

/// `(x, y, width, height)` in surface logical pixels.
pub type Rect = (i32, i32, i32, i32);

/// What the surface does before the next frame.
#[derive(Debug, PartialEq, Eq)]
pub enum Order {
    Create,
    Destroy,
}

/// One commit: the logical size to ask for, when it changes, and the frame to
/// attach. A size committed without its frame shows the last buffer at the
/// new size.
pub struct Draw {
    pub resize: Option<(u32, u32)>,
    pub pixmap: Pixmap,
}

pub struct Renderer {
    /// On the surface, through its leave too.
    scene: Option<Scene>,
    /// Each move with the instant it started and how long it lasts, in the
    /// order they play.
    moves: Vec<(Move, Instant, Option<f64>)>,
    leave_ends: Option<Instant>,
    line: Option<Line>,
    card: Option<Card>,
    /// The width the capsule grows or shrinks from, and when that began.
    width_from: Option<(f64, Instant)>,
    /// The height the card grows or shrinks from, and when that began.
    height_from: Option<(f64, Instant)>,
    fonts: words::Fonts,
    voice: Voice,
    voice_since: Option<Instant>,
    /// The pose the last level sample set, eased into from `pose_from`
    /// over `LEVEL_STEP` after `heard`.
    pose: Pose,
    pose_from: Pose,
    heard: Option<Instant>,
    level: f64,
    /// The surface's logical size, which never shrinks until the surface goes.
    surface_size: (u32, u32),
    /// A frame is owed: the picture changed, or the surface asked for one.
    pub stale: bool,
    /// Logical pixels the words may take.
    pub room: f64,
    pub card_room: f64,
}

impl Renderer {
    pub fn new() -> Self {
        Renderer {
            scene: None,
            moves: Vec::new(),
            leave_ends: None,
            line: None,
            card: None,
            width_from: None,
            height_from: None,
            fonts: words::Fonts,
            voice: Voice::default(),
            voice_since: None,
            pose: figure::pose(0.0, 0.0, false),
            pose_from: figure::pose(0.0, 0.0, false),
            heard: None,
            level: 0.0,
            surface_size: (0, 0),
            stale: false,
            room: f64::INFINITY,
            card_room: card::room(f64::INFINITY),
        }
    }

    pub fn show(&mut self, scene: Option<&Scene>, now: Instant) -> Vec<Order> {
        let Some(scene) = scene else {
            return self.leave(now);
        };
        let mut orders = Vec::new();
        if self.leave_ends.is_some() {
            self.clear();
            orders.push(Order::Destroy);
        }
        if self.scene.as_ref() == Some(scene) {
            return orders;
        }
        let card = card::layout(scene, self.card_room, false, now, &self.fonts).map(Card::new);
        let entrance = match &self.scene {
            None => Entrance::Fresh,
            Some(shown)
                if shown.serial == scene.serial && self.card.is_some() == card.is_some() =>
            {
                self.lay_words(scene, card, now);
                self.scene = Some(scene.clone());
                return orders;
            }
            Some(shown) if answer_opens(Some(&shown.show), &scene.show) => Entrance::SameWords,
            Some(_) => Entrance::NewWords,
        };
        if entrance == Entrance::Fresh {
            orders.push(Order::Create);
        } else {
            // The capsule keeps its entrance; every gesture since stops.
            self.moves
                .retain(|&(step, ..)| matches!(step, Move::Rise | Move::FadeIn | Move::Emerge));
        }
        self.voice = Voice::default();
        self.voice_since = Some(now);
        self.pose = figure::pose(0.0, 0.0, false);
        self.heard = None;
        self.level = 0.0;
        self.lay_words(scene, card, now);
        let started = moves(scene, entrance, false);
        self.moves.extend(
            started
                .into_iter()
                .map(|step| (step, now, paint::lasts(step))),
        );
        self.scene = Some(scene.clone());
        orders
    }

    /// Sets the words or the card and sizes the capsule to them, from the
    /// size it shows now when a scene already shows.
    fn lay_words(&mut self, scene: &Scene, card: Option<Card>, now: Instant) {
        let from = self.picture(now, 1, |frame| {
            (paint::shown_width(frame), paint::shown_height(frame))
        });
        self.card = card;
        self.line = words_of(&scene.show)
            .filter(|_| self.card.is_none())
            .map(|(text, cut)| words::line(text, cut, self.room as f32));
        let layout = self.card.as_ref().map(|card| &card.layout);
        let (width, height) = paint::target(self.line.as_ref(), layout);
        let moved = |from: f64, to: f64| (from != to).then_some((from, now));
        self.width_from = from.and_then(|(from, _)| moved(from, width));
        self.height_from = from.and_then(|(_, from)| moved(from, height));
        self.stale = true;
    }

    fn leave(&mut self, now: Instant) -> Vec<Order> {
        if self.leave_ends.is_some() {
            return Vec::new();
        }
        let Some(shown) = &self.scene else {
            return Vec::new();
        };
        let Some(step) = leaving(&shown.show, false) else {
            self.clear();
            return vec![Order::Destroy];
        };
        let lasts = paint::lasts(step).expect("a leave ends");
        self.moves.push((step, now, Some(lasts)));
        self.leave_ends = Some(now + Duration::from_secs_f64(lasts));
        self.stale = true;
        Vec::new()
    }

    /// Ends a leave whose move has played.
    pub fn tick(&mut self, now: Instant) -> Vec<Order> {
        if self.deadline().is_some_and(|ends| now >= ends) {
            self.clear();
            return vec![Order::Destroy];
        }
        Vec::new()
    }

    pub fn deadline(&self) -> Option<Instant> {
        self.leave_ends
    }

    pub fn level(&mut self, peak: f32, now: Instant) {
        let Some(scene) = self.scene.as_ref().filter(|_| self.leave_ends.is_none()) else {
            return;
        };
        let bars = self
            .card
            .as_ref()
            .is_some_and(|card| card.layout.follows_the_level());
        let figure = self.card.is_none() && holds_the_voice(&scene.show);
        if !bars && !figure {
            return;
        }
        let level = self.voice.hear(peak, now);
        self.stale = true;
        if bars {
            self.level = level;
            return;
        }
        let t_ms = self
            .voice_since
            .map_or(0.0, |since| seconds(since, now) * 1000.0);
        self.pose_from = self.pose_at(now);
        self.pose = figure::pose(level, t_ms, false);
        self.heard = Some(now);
    }

    /// Whether frames still change with nothing new coming in.
    pub fn live(&self, now: Instant) -> bool {
        let moving = self
            .moves
            .iter()
            .any(|&(_, started, lasts)| lasts.is_none_or(|lasts| seconds(started, now) < lasts));
        let easing = self
            .heard
            .is_some_and(|heard| seconds(heard, now) < LEVEL_STEP);
        let counting = self.countdown_share(now).is_some_and(|share| share > 0.0);
        moving
            || easing
            || counting
            || self.transition(now).is_some()
            || self.height_transition(now).is_some()
    }

    fn countdown_share(&self, now: Instant) -> Option<f64> {
        let countdown = self.scene.as_ref()?.countdown?;
        Some(card::share_left(countdown, now))
    }

    /// The pose part way from the one before the last sample to the last, at a
    /// linear pace.
    fn pose_at(&self, now: Instant) -> Pose {
        let Some(heard) = self.heard else {
            return self.pose;
        };
        let share = (seconds(heard, now) / LEVEL_STEP).min(1.0);
        Pose::lerp(self.pose_from, self.pose, share)
    }

    /// The width transition's from-width and seconds so far, while it runs.
    fn transition(&self, now: Instant) -> Option<(f64, f64)> {
        running(self.width_from, now)
    }

    fn height_transition(&self, now: Instant) -> Option<(f64, f64)> {
        running(self.height_from, now)
    }

    /// Hands `use_frame` the frame at `now`. `None` with nothing on the surface.
    pub fn picture<R>(
        &self,
        now: Instant,
        scale: u32,
        use_frame: impl FnOnce(&Frame) -> R,
    ) -> Option<R> {
        let scene = self.scene.as_ref()?;
        let moves: Vec<_> = self
            .moves
            .iter()
            .map(|&(step, started, _)| (step, seconds(started, now)))
            .collect();
        Some(use_frame(&Frame {
            show: &scene.show,
            moves: &moves,
            pose: self.pose_at(now),
            line: self.line.as_ref(),
            width_from: self.transition(now),
            card: self.card.as_ref(),
            height_from: self.height_transition(now),
            level: self.level,
            countdown_share: self.countdown_share(now),
            scale,
            at_least: self.surface_size,
        }))
    }

    /// The logical size a new surface asks for.
    pub fn open(&mut self, now: Instant) -> Option<(u32, u32)> {
        self.surface_size = self.picture(now, 1, paint::size)?;
        Some(self.surface_size)
    }

    /// What the surface commits next.
    pub fn draw(&mut self, now: Instant, scale: u32) -> Option<Draw> {
        let (size, pixmap) = self.picture(now, scale, |frame| {
            (paint::size(frame), paint::paint(frame))
        })?;
        self.stale = false;
        let resize = (size != self.surface_size).then_some(size);
        self.surface_size = size;
        Some(Draw { resize, pixmap })
    }

    /// The capsule's or the pill's rectangle while it takes clicks.
    pub fn input_region(&self, now: Instant) -> Option<Rect> {
        let (span, shape, top) = self.clickable(now)?;
        let x = ((span - shape.width) / 2.0).floor();
        Some((
            x as i32,
            top as i32,
            shape.width.ceil() as i32,
            shape.height.ceil() as i32,
        ))
    }

    /// Whether a press at `(x, y)`, surface logical pixels, lands on a
    /// capsule or a pill that takes clicks.
    pub fn opens_at(&self, x: f64, y: f64, now: Instant) -> bool {
        self.clickable(now)
            .is_some_and(|(span, shape, top)| card::in_shape(span, shape, x, y - top))
    }

    /// The surface's span, the shape as it shows and its top edge to the whole
    /// pixel, while a scene that takes clicks shows and is not on its way out.
    fn clickable(&self, now: Instant) -> Option<(f64, Shape, f64)> {
        let clicks = self
            .scene
            .as_ref()
            .is_some_and(|scene| takes_clicks(&scene.show));
        if !clicks || self.leave_ends.is_some() {
            return None;
        }
        self.picture(now, 1, |frame| {
            (
                f64::from(paint::size(frame).0),
                paint::shown_shape(frame),
                paint::capsule_top(frame).round(),
            )
        })
    }

    pub fn clear(&mut self) {
        self.scene = None;
        self.moves.clear();
        self.leave_ends = None;
        self.line = None;
        self.card = None;
        self.width_from = None;
        self.height_from = None;
        self.surface_size = (0, 0);
    }
}

fn running(from: Option<(f64, Instant)>, now: Instant) -> Option<(f64, f64)> {
    let (from, started) = from?;
    let elapsed = seconds(started, now);
    (elapsed < gesture::CHIP_WIDTH_LASTS).then_some((from, elapsed))
}

/// The words' room on an output `width` logical pixels wide, with the capsule
/// centred and clear of both sides.
pub fn room_on(width: i32) -> f64 {
    f64::from(width) - 2.0 * SCREEN_MARGIN - AROUND_WORDS
}

pub fn card_room_on(width: i32) -> f64 {
    card::room(f64::from(width) - 2.0 * SCREEN_MARGIN)
}

/// tiny-skia's premultiplied RGBA as `wl_shm` ARGB8888, a premultiplied
/// little-endian 0xAARRGGBB, so B, G, R, A in memory.
pub fn to_argb(rgba: &[u8], argb: &mut [u8]) {
    for (from, to) in rgba
        .as_chunks::<4>()
        .0
        .iter()
        .zip(argb.as_chunks_mut::<4>().0)
    {
        let [red, green, blue, alpha] = *from;
        *to = [blue, green, red, alpha];
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::card::{CARD_WIDTH, SIDE};
    use crate::chip::{Countdown, Done, Show};
    use crate::figure::{CAPSULE, TRAVEL};

    fn at(t0: Instant, ms: u64) -> Instant {
        t0 + Duration::from_millis(ms)
    }

    fn scene(show: Show, serial: u64) -> Scene {
        Scene::of(show, serial)
    }

    #[test]
    fn a_rise_during_a_leave_shows_the_new_scene_on_a_fresh_surface() {
        let t0 = Instant::now();
        let working = scene(Show::Working, 1);
        let your_turn = scene(Show::YourTurn, 2);
        let mut renderer = Renderer::new();
        let mut orders = renderer.show(Some(&working), at(t0, 0));
        orders.extend(renderer.show(None, at(t0, 1000)));
        orders.extend(renderer.show(Some(&your_turn), at(t0, 1050)));
        orders.extend(renderer.tick(at(t0, 5000)));
        assert_eq!(orders, [Order::Create, Order::Destroy, Order::Create]);
        assert_eq!(renderer.scene.as_ref(), Some(&your_turn));
        let steps: Vec<_> = renderer.moves.iter().map(|&(step, ..)| step).collect();
        assert_eq!(steps.first(), Some(&Move::Rise));
        assert!(!steps.contains(&Move::FailureExit), "{steps:?}");
    }

    #[test]
    fn a_leave_plays_its_move_then_drops_the_surface() {
        let t0 = Instant::now();
        let mut renderer = Renderer::new();
        renderer.show(Some(&scene(Show::Working, 1)), t0);
        assert_eq!(renderer.show(None, at(t0, 1000)), []);
        let ends = renderer.deadline().expect("a leave has an end");
        assert!(ends > at(t0, 1000));
        assert_eq!(renderer.tick(ends - Duration::from_millis(1)), []);
        assert_eq!(renderer.tick(ends), [Order::Destroy]);
        assert!(renderer.scene.is_none());
    }

    #[test]
    fn done_has_already_floated_out_so_its_surface_goes_at_once() {
        let t0 = Instant::now();
        let mut renderer = Renderer::new();
        renderer.show(Some(&scene(Show::Done(Done::Typed), 1)), t0);
        assert_eq!(renderer.show(None, at(t0, 440)), [Order::Destroy]);
        assert_eq!(renderer.deadline(), None);
    }

    #[test]
    fn frames_stop_once_the_scene_is_still() {
        let t0 = Instant::now();
        let mut renderer = Renderer::new();
        renderer.show(Some(&scene(Show::Working, 1)), t0);
        assert!(renderer.live(at(t0, 60_000)), "the halo keeps orbiting");
        renderer.show(Some(&scene(Show::YourTurn, 2)), at(t0, 60_000));
        assert!(renderer.live(at(t0, 60_100)));
        assert!(
            !renderer.live(at(t0, 62_000)),
            "the orbit stopped with its scene"
        );
    }

    #[test]
    fn new_words_grow_the_capsule_from_the_width_it_shows() {
        let t0 = Instant::now();
        let mut renderer = Renderer::new();
        renderer.show(Some(&scene(Show::Recording("hi".into()), 1)), t0);
        assert_eq!(
            renderer.transition(t0),
            None,
            "a rise has no width to grow from"
        );
        let before = renderer.picture(at(t0, 500), 1, paint::shown_width);
        let longer = scene(Show::Recording("hi there, how are you".into()), 1);
        renderer.show(Some(&longer), at(t0, 500));
        let (from, _) = renderer.transition(at(t0, 500)).expect("the width moves");
        assert_eq!(Some(from), before);
        assert!(paint::width(renderer.line.as_ref()) > from);
        assert_eq!(renderer.transition(at(t0, 1500)), None);
    }

    #[test]
    fn the_room_cuts_long_words() {
        let t0 = Instant::now();
        let long = "a sentence long enough to run past a narrow output's room";
        let mut renderer = Renderer::new();
        renderer.room = room_on(400);
        renderer.show(Some(&scene(Show::Recording(long.into()), 1)), t0);
        let width = renderer.line.as_ref().map(|line| f64::from(line.width));
        assert!(
            width.is_some_and(|width| width <= room_on(400)),
            "{width:?}"
        );
    }

    #[test]
    fn only_a_scene_that_holds_the_voice_follows_the_level() {
        let t0 = Instant::now();
        let rest = figure::pose(0.0, 0.0, false);
        let mut renderer = Renderer::new();
        renderer.show(Some(&scene(Show::Working, 1)), t0);
        renderer.level(0.5, at(t0, 100));
        assert_eq!(renderer.pose, rest);
        renderer.show(Some(&scene(Show::Recording(String::new()), 2)), at(t0, 200));
        renderer.level(0.5, at(t0, 300));
        assert_ne!(renderer.pose, rest);
    }

    #[test]
    fn a_level_eases_into_its_pose_over_one_sample() {
        let t0 = Instant::now();
        let rest = figure::pose(0.0, 0.0, false);
        let mut renderer = Renderer::new();
        renderer.show(Some(&scene(Show::Recording(String::new()), 1)), t0);
        assert!(!renderer.live(at(t0, 1000)));
        renderer.level(1.0, at(t0, 1000));
        let heard = renderer.pose;
        assert!(renderer.live(at(t0, 1016)));
        assert!(!renderer.live(at(t0, 1040)));
        let lift = |ms| {
            renderer
                .picture(at(t0, ms), 1, |frame| frame.pose.lift)
                .expect("a scene shows")
        };
        assert_eq!(lift(1000), rest.lift);
        let between = lift(1016);
        assert!(heard.lift < between && between < rest.lift, "{between}");
        assert_eq!(lift(1040), heard.lift);
    }

    #[test]
    fn an_early_sample_eases_on_from_the_pose_on_screen() {
        let t0 = Instant::now();
        let mut renderer = Renderer::new();
        renderer.show(Some(&scene(Show::Recording(String::new()), 1)), t0);
        renderer.level(1.0, at(t0, 1000));
        let lift = |renderer: &Renderer| {
            renderer
                .picture(at(t0, 1016), 1, |frame| frame.pose.lift)
                .expect("a scene shows")
        };
        let shown = lift(&renderer);
        renderer.level(0.2, at(t0, 1016));
        assert_eq!(lift(&renderer), shown);
    }

    #[test]
    fn the_click_area_rides_the_rise() {
        let t0 = Instant::now();
        let mut renderer = Renderer::new();
        renderer.show(Some(&scene(Show::Broken("No microphone".into()), 1)), t0);
        let rising = at(t0, 30);
        let top = renderer
            .picture(rising, 1, paint::capsule_top)
            .expect("a scene shows");
        assert!(top > 1.0, "{top}");
        let (_, y, _, _) = renderer.input_region(rising).expect("a failure");
        assert_eq!(y, top.round() as i32);
        let (y, middle) = (f64::from(y), paint::width(renderer.line.as_ref()) / 2.0);
        assert!(renderer.opens_at(middle, y, rising), "the region's top row");
        assert!(
            renderer.opens_at(middle, y + CAPSULE - 1.0, rising),
            "the region's bottom row"
        );
        assert!(
            !renderer.opens_at(middle, y - 1.0, rising),
            "above the region"
        );
        let settled = renderer.input_region(at(t0, 1000)).expect("a failure");
        assert_eq!(settled.1, 0);
    }

    fn missed() -> Show {
        Show::Missed {
            count: 1,
            ask: "ask".into(),
        }
    }

    #[test]
    fn only_a_broken_or_missed_scene_takes_clicks() {
        let t0 = Instant::now();
        let others = [
            Show::Recording("hi".into()),
            Show::Working,
            Show::YourTurn,
            Show::Answering("ok".into()),
            Show::NothingHeard("Nothing heard".into()),
            Show::Done(Done::Typed),
            Show::Done(Done::Sent),
            Show::Copied,
            Show::Reading,
        ];
        for show in others {
            for scene in [scene(show.clone(), 1), card(show.clone(), 1)] {
                let mut renderer = Renderer::new();
                renderer.show(Some(&scene), t0);
                assert_eq!(renderer.input_region(at(t0, 500)), None, "{show:?}");
                assert!(!renderer.opens_at(20.0, 22.0, at(t0, 500)), "{show:?}");
            }
        }
        let mut renderer = Renderer::new();
        assert_eq!(renderer.input_region(t0), None, "nothing shows");
        renderer.show(Some(&scene(Show::Broken("No microphone".into()), 1)), t0);
        let (x, y, width, height) = renderer.input_region(at(t0, 500)).expect("a failure");
        assert_eq!((x, y, height), (0, 0, CAPSULE as i32));
        assert_eq!(f64::from(width), paint::width(renderer.line.as_ref()));

        let mut renderer = Renderer::new();
        renderer.show(Some(&scene(missed(), 1)), t0);
        let shape = renderer
            .card
            .as_ref()
            .map(|card| card.layout.shape)
            .expect("a layout");
        assert_eq!(
            renderer.input_region(at(t0, 500)),
            Some((0, 0, shape.width.ceil() as i32, shape.height.ceil() as i32))
        );
    }

    #[test]
    fn a_press_inside_the_missed_pill_opens_the_window() {
        let t0 = Instant::now();
        let mut renderer = Renderer::new();
        renderer.show(Some(&scene(missed(), 1)), t0);
        let shape = renderer
            .card
            .as_ref()
            .map(|card| card.layout.shape)
            .expect("a layout");
        let (width, height) = (shape.width, shape.height);
        let now = at(t0, 500);
        assert!(renderer.opens_at(width / 2.0, height / 2.0, now));
        assert!(
            renderer.opens_at(width / 2.0, height - 2.0, now),
            "the second line"
        );
        assert!(!renderer.opens_at(1.0, 1.0, now), "the corner");
        assert!(!renderer.opens_at(width / 2.0, height + 2.0, now), "below");
    }

    #[test]
    fn a_press_inside_a_broken_capsule_opens_the_window() {
        let t0 = Instant::now();
        let mut renderer = Renderer::new();
        renderer.show(Some(&scene(Show::Broken("No microphone".into()), 1)), t0);
        let width = paint::width(renderer.line.as_ref());
        let now = at(t0, 500);
        assert!(renderer.opens_at(width / 2.0, 22.0, now));
        assert!(!renderer.opens_at(width / 2.0, CAPSULE + 4.0, now), "below");
        assert!(!renderer.opens_at(width + 4.0, 22.0, now), "beside");
        assert!(!renderer.opens_at(1.0, 1.0, now), "the corner");
    }

    #[test]
    fn a_failure_on_its_way_out_takes_no_clicks() {
        let t0 = Instant::now();
        let mut renderer = Renderer::new();
        renderer.show(Some(&scene(Show::Broken("No microphone".into()), 1)), t0);
        renderer.show(None, at(t0, 1000));
        assert_eq!(renderer.input_region(at(t0, 1010)), None);
        assert!(!renderer.opens_at(20.0, 22.0, at(t0, 1010)));
    }

    #[test]
    fn the_region_follows_a_growing_capsule_centred_in_its_span() {
        let t0 = Instant::now();
        let mut renderer = Renderer::new();
        renderer.show(Some(&scene(Show::Broken("No microphone".into()), 1)), t0);
        let narrow = renderer.input_region(at(t0, 500)).expect("a failure");
        renderer.show(
            Some(&scene(
                Show::Broken("No microphone, check the device".into()),
                1,
            )),
            at(t0, 500),
        );
        let start = renderer.input_region(at(t0, 500)).expect("a failure");
        let end = renderer.input_region(at(t0, 5000)).expect("a failure");
        assert_eq!(start.2, narrow.2, "it grows from the width it shows");
        assert!(end.2 > start.2);
        assert!(start.0 > 0 && end.0 == 0, "{start:?} {end:?}");
    }

    fn card(show: Show, serial: u64) -> Scene {
        Scene {
            question: Some("The build is red. Retry it?".into()),
            ..scene(show, serial)
        }
    }

    #[test]
    fn a_card_grows_from_the_height_it_shows() {
        let t0 = Instant::now();
        let mut renderer = Renderer::new();
        renderer.show(Some(&card(Show::Reading, 1)), t0);
        let reading = renderer
            .picture(at(t0, 500), 1, paint::shown_height)
            .expect("a scene shows");
        let words = vec!["alpha"; 30].join(" ");
        renderer.show(Some(&card(Show::Answering(words), 2)), at(t0, 500));
        let (from, _) = renderer.height_from.expect("the height moves");
        assert_eq!(from, reading);
        let answering = renderer
            .card
            .as_ref()
            .map(|card| card.layout.shape.height.ceil())
            .expect("a card");
        assert!(answering > reading, "{answering} {reading}");
        let size = renderer.picture(at(t0, 500), 1, paint::size);
        assert_eq!(size, Some((CARD_WIDTH as u32, (TRAVEL + answering) as u32)));
        assert!(renderer.live(at(t0, 600)));
        assert!(!renderer.live(at(t0, 1500)));
    }

    #[test]
    fn the_card_shrinks_to_the_sent_pill_from_its_shown_size() {
        let t0 = Instant::now();
        let mut renderer = Renderer::new();
        renderer.show(Some(&card(Show::Working, 1)), t0);
        let height = renderer
            .card
            .as_ref()
            .map(|card| card.layout.shape.height.ceil())
            .expect("a card");
        let t = at(t0, 1000);
        let sent = scene(Show::Done(Done::Sent), 2);
        renderer.show(Some(&sent), t);
        assert_eq!(renderer.height_from, Some((height, t)));
        assert_eq!(renderer.width_from, Some((CARD_WIDTH, t)));
        assert!(renderer.card.is_none());
        let line = renderer.line.as_ref().expect("the pill's word");
        assert!(line.path.is_some());
        let settled = renderer.picture(at(t0, 5000), 1, paint::shown_shape);
        assert_eq!(
            settled,
            Some(Shape {
                width: paint::width(Some(line)),
                height: CAPSULE,
                radius: CAPSULE / 2.0,
            })
        );
    }

    fn pixels(draw: &Draw) -> (u32, u32) {
        (draw.pixmap.width(), draw.pixmap.height())
    }

    #[test]
    fn the_surface_keeps_the_card_size_once_the_card_shrinks_to_the_sent_pill() {
        for scale in [1, 2] {
            let t0 = Instant::now();
            let mut renderer = Renderer::new();
            renderer.show(Some(&card(Show::Working, 1)), t0);
            let card_size = renderer.open(t0).expect("a card");
            let first = renderer.draw(t0, scale).expect("a card");
            assert_eq!(first.resize, None);
            renderer.show(Some(&scene(Show::Done(Done::Sent), 2)), at(t0, 1000));
            for ms in [1100, 2000, 5000] {
                let draw = renderer.draw(at(t0, ms), scale).expect("a pill");
                assert_eq!(draw.resize, None, "at {ms} ms, scale {scale}");
                assert_eq!(
                    pixels(&draw),
                    (card_size.0 * scale, card_size.1 * scale),
                    "at {ms} ms, scale {scale}"
                );
            }
            let top = renderer
                .picture(at(t0, 5000), 1, paint::capsule_top)
                .expect("a pill");
            assert_eq!(top, f64::from(card_size.1) - TRAVEL - CAPSULE);
            assert!(renderer.line.is_some(), "a figure pill with its word");
            assert_eq!(renderer.input_region(at(t0, 5000)), None);
            let middle = f64::from(card_size.0) / 2.0;
            assert!(!renderer.opens_at(middle, top + CAPSULE / 2.0, at(t0, 5000)));
        }
    }

    #[test]
    fn a_pill_that_ends_a_card_takes_clicks_only_on_its_own_shape() {
        let t0 = Instant::now();
        let mut renderer = Renderer::new();
        renderer.show(Some(&card(Show::Working, 1)), t0);
        let card_size = renderer.open(t0).expect("a card");
        renderer.draw(t0, 1).expect("a card");
        let broken = scene(Show::Broken("No microphone".into()), 2);
        renderer.show(Some(&broken), at(t0, 1000));
        let now = at(t0, 5000);
        let draw = renderer.draw(now, 1).expect("a pill");
        assert_eq!(draw.resize, None);
        let width = paint::width(renderer.line.as_ref());
        let top = f64::from(card_size.1) - TRAVEL - CAPSULE;
        let left = ((f64::from(card_size.0) - width) / 2.0).floor();
        assert_eq!(
            renderer.input_region(now),
            Some((left as i32, top as i32, width as i32, CAPSULE as i32))
        );
        let middle = f64::from(card_size.0) / 2.0;
        assert!(renderer.opens_at(middle, top + CAPSULE / 2.0, now));
        assert!(!renderer.opens_at(middle, top - 2.0, now), "above the pill");
        assert!(
            !renderer.opens_at(left - 2.0, top + CAPSULE / 2.0, now),
            "beside"
        );
    }

    #[test]
    fn a_growing_surface_asks_for_its_size_with_the_frame_that_fills_it() {
        let t0 = Instant::now();
        let mut renderer = Renderer::new();
        renderer.show(Some(&scene(Show::Recording("hi".into()), 1)), t0);
        let short = renderer.open(t0).expect("a pill");
        renderer.draw(t0, 2).expect("a pill");
        let longer = Show::Recording("hi there, how are you doing today".into());
        renderer.show(Some(&scene(longer, 1)), at(t0, 500));
        let draw = renderer.draw(at(t0, 1500), 2).expect("a pill");
        let (width, height) = draw.resize.expect("the surface grows");
        assert!(width > short.0, "{width} {}", short.0);
        assert_eq!(pixels(&draw), (width * 2, height * 2));
        renderer.show(Some(&scene(Show::Recording("hi".into()), 1)), at(t0, 1500));
        let draw = renderer.draw(at(t0, 3000), 2).expect("a pill");
        assert_eq!(draw.resize, None, "it never shrinks while it shows");
    }

    #[test]
    fn a_fresh_surface_starts_at_its_own_size() {
        let t0 = Instant::now();
        let mut renderer = Renderer::new();
        renderer.show(Some(&card(Show::Working, 1)), t0);
        renderer.open(t0).expect("a card");
        renderer.draw(t0, 1).expect("a card");
        renderer.show(None, at(t0, 1000));
        let ends = renderer.deadline().expect("a leave has an end");
        assert_eq!(renderer.tick(ends), [Order::Destroy]);
        renderer.show(Some(&scene(Show::Working, 2)), at(t0, 5000));
        let size = renderer.picture(at(t0, 5000), 1, paint::size);
        assert_eq!(size, Some((CAPSULE as u32, (TRAVEL + CAPSULE) as u32)));
    }

    #[test]
    fn a_serial_keeps_its_moves_only_while_its_layout_stays() {
        let t0 = Instant::now();
        let mut renderer = Renderer::new();
        renderer.show(Some(&card(Show::Working, 1)), t0);
        renderer.show(
            Some(&card(Show::NothingHeard("Nothing heard".into()), 1)),
            at(t0, 500),
        );
        assert!(renderer.card.is_none());
        let steps: Vec<_> = renderer.moves.iter().map(|&(step, ..)| step).collect();
        assert!(!steps.contains(&Move::Pulse), "{steps:?}");
    }

    #[test]
    fn a_countdown_keeps_the_frames_coming_until_it_closes() {
        let t0 = Instant::now();
        let answering = card(Show::Answering("yes".into()), 1);
        let mut renderer = Renderer::new();
        renderer.show(Some(&answering), t0);
        let since = at(t0, 5000);
        assert!(!renderer.live(since));
        let countdown = Countdown {
            since,
            lasts: Duration::from_millis(2500),
            teach: false,
        };
        let counting = Scene {
            countdown: Some(countdown),
            ..answering
        };
        renderer.show(Some(&counting), since);
        assert!(renderer.live(since + Duration::from_secs(1)));
        assert!(!renderer.live(since + countdown.lasts + Duration::from_millis(1)));
    }

    #[test]
    fn a_scene_with_a_question_has_a_layout_and_a_pill_has_none() {
        let t0 = Instant::now();
        let mut renderer = Renderer::new();
        renderer.show(Some(&card(Show::YourTurn, 1)), t0);
        assert!(renderer.card.is_some());
        renderer.show(Some(&scene(Show::YourTurn, 2)), at(t0, 500));
        assert!(renderer.card.is_none());
    }

    #[test]
    fn a_question_that_clears_at_one_serial_turns_the_card_into_the_pill() {
        let t0 = Instant::now();
        let mut renderer = Renderer::new();
        renderer.show(Some(&card(Show::Working, 1)), t0);
        let orders = renderer.show(Some(&scene(Show::Working, 1)), at(t0, 500));
        assert_eq!(orders, []);
        assert!(renderer.card.is_none());
        let steps: Vec<_> = renderer.moves.iter().map(|&(step, ..)| step).collect();
        assert!(steps.contains(&Move::Orbit), "{steps:?}");
        assert!(!steps.contains(&Move::Pulse), "{steps:?}");
    }

    #[test]
    fn a_card_follows_the_level_only_while_it_waits_for_or_hears_the_answer() {
        let t0 = Instant::now();
        let level = |renderer: &Renderer, ms| {
            renderer
                .picture(at(t0, ms), 1, |frame| frame.level)
                .expect("a scene shows")
        };
        let mut renderer = Renderer::new();
        renderer.show(Some(&card(Show::Working, 1)), t0);
        renderer.level(1.0, at(t0, 100));
        assert_eq!(level(&renderer, 100), 0.0);
        renderer.show(Some(&card(Show::YourTurn, 2)), at(t0, 200));
        renderer.level(1.0, at(t0, 300));
        assert!(level(&renderer, 300) > 0.0);
        renderer.show(Some(&card(Show::Reading, 3)), at(t0, 400));
        assert_eq!(level(&renderer, 400), 0.0);
        let counting = Scene {
            countdown: Some(Countdown {
                since: at(t0, 500),
                lasts: Duration::from_millis(2500),
                teach: false,
            }),
            ..card(Show::Answering("yes".into()), 4)
        };
        renderer.show(Some(&counting), at(t0, 500));
        renderer.level(1.0, at(t0, 600));
        assert_eq!(level(&renderer, 600), 0.0);
    }

    #[test]
    fn the_card_room_is_the_card_clear_of_both_screen_margins() {
        assert_eq!(card_room_on(1920), CARD_WIDTH - 2.0 * SIDE);
        assert_eq!(card_room_on(400), 400.0 - 2.0 * SCREEN_MARGIN - 2.0 * SIDE);
    }

    #[test]
    fn a_rust_pixel_lands_as_argb8888() {
        let mut argb = [0; 4];
        to_argb(&[0x8A, 0x2A, 0x0D, 0xFF], &mut argb);
        assert_eq!(u32::from_le_bytes(argb), 0xFF8A_2A0D);
    }
}
