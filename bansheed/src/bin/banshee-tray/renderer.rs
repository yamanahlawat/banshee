//! What the capsule shows from moment to moment, with time passed in and no
//! Wayland types.

use std::time::{Duration, Instant};

use crate::chip::{Scene, Show, words_of};
use crate::figure::{self, AROUND_WORDS, CAPSULE, LEVEL_STEP, Pose, SCREEN_MARGIN, Voice, seconds};
use crate::gesture;
use crate::paint::{self, Frame};
use crate::scenes::{
    Entrance, Move, answer_opens, holds_the_voice, in_capsule, leaving, moves, takes_clicks,
};
use crate::words::{self, Line};

/// `(x, y, width, height)` in surface logical pixels.
pub type Rect = (i32, i32, i32, i32);

/// What the surface does before the next frame.
#[derive(Debug, PartialEq, Eq)]
pub enum Order {
    Create,
    Destroy,
}

pub struct Renderer {
    /// On the surface, through its leave too.
    scene: Option<Scene>,
    /// Each move with the instant it started and how long it lasts, in the
    /// order they play.
    moves: Vec<(Move, Instant, Option<f64>)>,
    leave_ends: Option<Instant>,
    line: Option<Line>,
    /// The width the capsule grows or shrinks from, and when that began.
    width_from: Option<(f64, Instant)>,
    voice: Voice,
    voice_since: Option<Instant>,
    /// The pose the last level sample set, eased into from `pose_from`
    /// over `LEVEL_STEP` after `heard`.
    pose: Pose,
    pose_from: Pose,
    heard: Option<Instant>,
    /// A frame is owed: the picture changed, or the surface asked for one.
    pub stale: bool,
    /// Logical pixels the words may take.
    pub room: f64,
}

impl Renderer {
    pub fn new() -> Self {
        Renderer {
            scene: None,
            moves: Vec::new(),
            leave_ends: None,
            line: None,
            width_from: None,
            voice: Voice::default(),
            voice_since: None,
            pose: figure::pose(0.0, 0.0, false),
            pose_from: figure::pose(0.0, 0.0, false),
            heard: None,
            stale: false,
            room: f64::INFINITY,
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
        let entrance = match &self.scene {
            None => Entrance::Fresh,
            Some(shown) if shown == scene => return orders,
            Some(shown) if shown.serial == scene.serial => {
                self.lay_words(&scene.show, now);
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
        self.lay_words(&scene.show, now);
        let started = moves(&scene.show, entrance, false);
        self.moves.extend(
            started
                .into_iter()
                .map(|step| (step, now, paint::lasts(step))),
        );
        self.scene = Some(scene.clone());
        orders
    }

    /// Sets the words and sizes the capsule to them, from the width it shows
    /// now when a scene already shows.
    fn lay_words(&mut self, show: &Show, now: Instant) {
        let from = self.widths(now).map(|(_, shown)| shown);
        self.line = words_of(show).map(|(text, cut)| words::line(text, cut, self.room as f32));
        let to = paint::width(self.line.as_ref());
        self.width_from = from.filter(|&from| from != to).map(|from| (from, now));
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
        let hears = self.leave_ends.is_none()
            && self
                .scene
                .as_ref()
                .is_some_and(|scene| holds_the_voice(&scene.show));
        if !hears {
            return;
        }
        let level = self.voice.hear(peak, now);
        let t_ms = self
            .voice_since
            .map_or(0.0, |since| seconds(since, now) * 1000.0);
        self.pose_from = self.pose_at(now);
        self.pose = figure::pose(level, t_ms, false);
        self.heard = Some(now);
        self.stale = true;
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
        moving || easing || self.transition(now).is_some()
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
        let (from, started) = self.width_from?;
        let elapsed = seconds(started, now);
        (elapsed < gesture::CHIP_WIDTH_LASTS).then_some((from, elapsed))
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
            scale,
        }))
    }

    /// The surface's span and the capsule's width as it shows at `now`. `None`
    /// with nothing on the surface.
    fn widths(&self, now: Instant) -> Option<(f64, f64)> {
        self.scene.as_ref()?;
        Some(paint::widths(self.line.as_ref(), self.transition(now)))
    }

    /// The capsule's rectangle while a failure shows and takes clicks.
    pub fn input_region(&self, now: Instant) -> Option<Rect> {
        let (span, width, top) = self.clickable(now)?;
        let x = ((span - width) / 2.0).floor();
        Some((x as i32, top as i32, width.ceil() as i32, CAPSULE as i32))
    }

    /// Whether a press at `(x, y)`, surface logical pixels, lands on a
    /// capsule that takes clicks.
    pub fn opens_at(&self, x: f64, y: f64, now: Instant) -> bool {
        self.clickable(now)
            .is_some_and(|(span, width, top)| in_capsule(span, width, x, y - top))
    }

    /// The surface's span, the capsule's width and its top edge to the whole
    /// pixel, while a failure shows and is not on its way out.
    fn clickable(&self, now: Instant) -> Option<(f64, f64, f64)> {
        let clicks = self
            .scene
            .as_ref()
            .is_some_and(|scene| takes_clicks(&scene.show));
        if !clicks || self.leave_ends.is_some() {
            return None;
        }
        self.picture(now, 1, |frame| {
            let (span, width) = paint::widths(frame.line, frame.width_from);
            (span, width, paint::capsule_top(frame).round())
        })
    }

    pub fn clear(&mut self) {
        self.scene = None;
        self.moves.clear();
        self.leave_ends = None;
        self.line = None;
        self.width_from = None;
    }
}

/// The words' room on an output `width` logical pixels wide, with the capsule
/// centred and clear of both sides.
pub fn room_on(width: i32) -> f64 {
    f64::from(width) - 2.0 * SCREEN_MARGIN - AROUND_WORDS
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
    use crate::chip::Done;

    fn at(t0: Instant, ms: u64) -> Instant {
        t0 + Duration::from_millis(ms)
    }

    fn scene(show: Show, serial: u64) -> Scene {
        Scene { show, serial }
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
        let before = renderer.widths(at(t0, 500)).map(|(_, shown)| shown);
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

    #[test]
    fn only_a_broken_scene_takes_clicks() {
        let t0 = Instant::now();
        let others = [
            Show::Recording("hi".into()),
            Show::Working,
            Show::YourTurn,
            Show::Answering("ok".into()),
            Show::NothingHeard("Nothing heard".into()),
            Show::Done(Done::Typed),
            Show::Copied,
        ];
        for show in others {
            let mut renderer = Renderer::new();
            renderer.show(Some(&scene(show.clone(), 1)), t0);
            assert_eq!(renderer.input_region(at(t0, 500)), None, "{show:?}");
            assert!(!renderer.opens_at(20.0, 22.0, at(t0, 500)), "{show:?}");
        }
        let mut renderer = Renderer::new();
        assert_eq!(renderer.input_region(t0), None, "nothing shows");
        renderer.show(Some(&scene(Show::Broken("No microphone".into()), 1)), t0);
        let (x, y, width, height) = renderer.input_region(at(t0, 500)).expect("a failure");
        assert_eq!((x, y, height), (0, 0, CAPSULE as i32));
        assert_eq!(f64::from(width), paint::width(renderer.line.as_ref()));
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

    #[test]
    fn a_rust_pixel_lands_as_argb8888() {
        let mut argb = [0; 4];
        to_argb(&[0x8A, 0x2A, 0x0D, 0xFF], &mut argb);
        assert_eq!(u32::from_le_bytes(argb), 0xFF8A_2A0D);
    }
}
