//! What the capsule shows from moment to moment: the scene logic the macOS
//! panel keeps in its layers, with time passed in and no Wayland types.

use std::time::{Duration, Instant};

use crate::chip::{Scene, Show, words_of};
use crate::figure::{self, FIGURE, INSET, LEVEL_STEP, Pose, Voice, WORDS_GAP, WORDS_TAIL};
use crate::gesture;
use crate::paint::{self, Frame};
use crate::scenes::{Entrance, Move, answer_opens, holds_the_voice, leaving, moves};
use crate::words::{self, Line};

const SIDE_MARGIN: f64 = 12.0;

/// What the surface does before the next frame.
#[derive(Debug, PartialEq, Eq)]
pub enum Order {
    Create,
    Destroy,
}

/// The scene logic the macOS panel keeps in its layers, with time passed in.
pub struct Renderer {
    /// On the surface, through its leave too.
    scene: Option<Scene>,
    /// Each move with the instant it started, in the order they play.
    moves: Vec<(Move, Instant)>,
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
    /// The picture changed since the last frame.
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
                .retain(|&(step, _)| matches!(step, Move::Rise | Move::FadeIn | Move::Emerge));
        }
        self.voice = Voice::default();
        self.voice_since = Some(now);
        self.pose = figure::pose(0.0, 0.0, false);
        self.heard = None;
        self.lay_words(&scene.show, now);
        let started = moves(&scene.show, entrance, false);
        self.moves
            .extend(started.into_iter().map(|step| (step, now)));
        self.scene = Some(scene.clone());
        orders
    }

    /// Sets the words and sizes the capsule to them, from the width it shows
    /// now when a scene already shows.
    fn lay_words(&mut self, show: &Show, now: Instant) {
        let from = self.picture(now, 1, paint::shown_width);
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
        self.moves.push((step, now));
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
        self.pose_from = self.pose;
        self.pose = figure::pose(level, t_ms, false);
        self.heard = Some(now);
        self.stale = true;
    }

    /// Whether frames still change with nothing new coming in.
    pub fn live(&self, now: Instant) -> bool {
        let moving = self.moves.iter().any(|&(step, started)| {
            paint::lasts(step).is_none_or(|lasts| seconds(started, now) < lasts)
        });
        let easing = self
            .heard
            .is_some_and(|heard| seconds(heard, now) < LEVEL_STEP);
        moving || easing || self.transition(now).is_some()
    }

    /// The pose part way from the one before the last sample to the last, at
    /// the linear pace Core Animation gives the macOS panel's level.
    fn pose_at(&self, now: Instant) -> Pose {
        let Some(heard) = self.heard else {
            return self.pose;
        };
        let share = (seconds(heard, now) / LEVEL_STEP).min(1.0);
        let (from, to) = (self.pose_from, self.pose);
        let mix = |from: f64, to: f64| from + (to - from) * share;
        Pose {
            depth: mix(from.depth, to.depth),
            sway: mix(from.sway, to.sway),
            lift: mix(from.lift, to.lift),
            scale: mix(from.scale, to.scale),
            fill: mix(from.fill, to.fill),
        }
    }

    /// The width transition's from-width and seconds so far, while it runs.
    fn transition(&self, now: Instant) -> Option<(f64, f64)> {
        let (from, started) = self.width_from?;
        let elapsed = seconds(started, now);
        let lasts = gesture::lasts(&gesture::chip_width(from, from))?;
        (elapsed < lasts).then_some((from, elapsed))
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
            .map(|&(step, started)| (step, seconds(started, now)))
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

    pub fn clear(&mut self) {
        self.scene = None;
        self.moves.clear();
        self.leave_ends = None;
        self.line = None;
        self.width_from = None;
    }
}

fn seconds(since: Instant, now: Instant) -> f64 {
    now.saturating_duration_since(since).as_secs_f64()
}

/// The words' room on an output `width` logical pixels wide, with the capsule
/// centred and clear of both sides.
pub fn room_on(width: i32) -> f64 {
    f64::from(width) - 2.0 * SIDE_MARGIN - (INSET + FIGURE + WORDS_GAP + WORDS_TAIL)
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
        let steps: Vec<_> = renderer.moves.iter().map(|&(step, _)| step).collect();
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
    fn a_rust_pixel_lands_as_argb8888() {
        let mut argb = [0; 4];
        to_argb(&[0x8A, 0x2A, 0x0D, 0xFF], &mut argb);
        assert_eq!(u32::from_le_bytes(argb), 0xFF8A_2A0D);
    }
}
