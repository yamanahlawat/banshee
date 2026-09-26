//! The Banshee shroud in the mark's own 100-unit square, y down.

use std::time::Instant;

/// MacBook Pro Microphone, sample 2 (1029 frames) p10: 0.016, the room between words.
const FLOOR: f64 = 0.016;

/// MacBook Pro Microphone, sample 2: 1 / (p95 0.106 - FLOOR), rounded to one decimal.
const GAIN: f64 = 11.1;

/// The microphone's peak level, smoothed toward a target the figure billows with.
#[derive(Default)]
pub struct Voice {
    level: f64,
    at: Option<Instant>,
}

impl Voice {
    /// Folds one microphone peak into the smoothed level and returns it.
    pub fn hear(&mut self, peak: f32, now: Instant) -> f64 {
        let target = ((peak as f64 - FLOOR) * GAIN).clamp(0.0, 1.0);
        // Unmeasured: the 0.55 share kept per 60ms step, and the 100ms cap.
        let dt_ms = self
            .at
            .map_or(60.0, |at| now.duration_since(at).as_secs_f64() * 1000.0)
            .min(100.0);
        let follow = 1.0 - 0.55f64.powf(dt_ms / 60.0);
        self.level += (target - self.level) * follow;
        self.at = Some(now);
        self.level
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    pub depth: f64,
    pub sway: f64,
    pub lift: f64,
    pub scale: f64,
    pub fill: f64,
}

/// Reduce Motion holds the figure still and carries the level as the bone
/// fill's strength alone, from 70% at silence to 100%.
pub fn pose(level: f64, t_ms: f64, calm: bool) -> Pose {
    if calm {
        return Pose {
            depth: 0.0,
            sway: 0.0,
            lift: 0.0,
            scale: 1.0,
            fill: 0.70 + 0.30 * level,
        };
    }
    Pose {
        depth: level,
        sway: (t_ms / 60.0 * 0.9).sin() * 4.0 * level,
        lift: -level * 7.0,
        scale: 1.0 + level * 0.08,
        fill: 1.0,
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Segment {
    Move(f64, f64),
    Line(f64, f64),
    Cubic([f64; 6]),
    Close,
}

/// `depth` deepens the hem's folds and `sway` moves them sideways. Both are 0
/// at rest.
pub fn shroud(depth: f64, sway: f64) -> [Segment; 10] {
    let s = sway;
    let fold = 86.0 + depth * 7.0;
    let notch = 72.0 - depth * 6.0;
    let side = 80.0 + depth * 3.0;
    [
        Segment::Move(21.0, 70.0),
        Segment::Line(24.0, 46.0),
        Segment::Cubic([24.0, 27.0, 34.0, 14.0, 50.0, 14.0]),
        Segment::Cubic([66.0, 14.0, 76.0, 27.0, 76.0, 46.0]),
        Segment::Line(79.0, 70.0),
        Segment::Cubic([79.0, side, 72.0 + s, fold + 2.0, 64.0 + s, fold]),
        Segment::Cubic([57.0 + s, fold - 2.0, 55.0 + s, notch, 50.0 + s, notch]),
        Segment::Cubic([45.0 + s, notch, 43.0 + s, fold - 2.0, 36.0 + s, fold]),
        Segment::Cubic([28.0 + s, fold + 2.0, 21.0, side, 21.0, 70.0]),
        Segment::Close,
    ]
}

/// The length SVG's `pathLength` stands for.
pub fn path_length(segments: &[Segment]) -> f64 {
    const PIECES: usize = 64;
    let distance = |(x0, y0): (f64, f64), (x1, y1): (f64, f64)| (x1 - x0).hypot(y1 - y0);
    let mut start = (0.0, 0.0);
    let mut at = (0.0, 0.0);
    let mut length = 0.0;
    for segment in segments {
        match *segment {
            Segment::Move(x, y) => {
                start = (x, y);
                at = start;
            }
            Segment::Line(x, y) => {
                length += distance(at, (x, y));
                at = (x, y);
            }
            Segment::Cubic([x1, y1, x2, y2, x, y]) => {
                let from = at;
                for piece in 1..=PIECES {
                    let t = piece as f64 / PIECES as f64;
                    let u = 1.0 - t;
                    let [a, b, c, d] = [u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t];
                    let next = (
                        a * from.0 + b * x1 + c * x2 + d * x,
                        a * from.1 + b * y1 + c * y2 + d * y,
                    );
                    length += distance(at, next);
                    at = next;
                }
            }
            Segment::Close => {
                length += distance(at, start);
                at = start;
            }
        }
    }
    length
}

/// Ramanujan's second approximation.
pub fn ellipse_length(rx: f64, ry: f64) -> f64 {
    let h = ((rx - ry) / (rx + ry)).powi(2);
    std::f64::consts::PI * (rx + ry) * (1.0 + 3.0 * h / (10.0 + (4.0 - 3.0 * h).sqrt()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn a_first_sample_moves_the_level_by_45_percent() {
        let mut voice = Voice::default();
        let level = voice.hear(1.0, Instant::now());
        assert!((level - 0.45).abs() < 1e-9, "level was {level}");
    }

    #[test]
    fn a_steady_peak_converges_to_its_target_within_a_second() {
        let peak = (FLOOR + 0.5 / GAIN) as f32;
        let mut voice = Voice::default();
        let mut now = Instant::now();
        let mut level = 0.0;
        for _ in 0..31 {
            now += Duration::from_millis(33);
            level = voice.hear(peak, now);
        }
        assert!((level - 0.5).abs() < 0.01, "level was {level}");
    }

    #[test]
    fn a_peak_at_or_below_the_floor_holds_the_level_at_zero() {
        let mut at_floor = Voice::default();
        assert!(at_floor.hear(FLOOR as f32, Instant::now()) < 1e-6);
        let mut below_floor = Voice::default();
        assert_eq!(below_floor.hear(0.0, Instant::now()), 0.0);
    }

    #[test]
    fn a_long_gap_is_capped_at_100ms_of_follow() {
        let mut voice = Voice::default();
        let now = Instant::now();
        let after_first = voice.hear(1.0, now);
        let level = voice.hear(1.0, now + Duration::from_secs(5));
        let capped_follow = 1.0 - 0.55f64.powf(100.0 / 60.0);
        let expected = after_first + (1.0 - after_first) * capped_follow;
        assert!((level - expected).abs() < 1e-9, "level was {level}");
    }

    #[test]
    fn calm_holds_depth_at_zero_and_scales_fill_with_level() {
        assert_eq!(pose(1.0, 0.0, true).depth, 0.0);
        assert_eq!(pose(1.0, 0.0, true).fill, 1.0);
        assert_eq!(pose(0.0, 0.0, true).depth, 0.0);
        assert_eq!(pose(0.0, 0.0, true).fill, 0.70);
    }

    #[test]
    fn silence_without_calm_is_the_rest_pose() {
        let rest = Pose {
            depth: 0.0,
            sway: 0.0,
            lift: 0.0,
            scale: 1.0,
            fill: 1.0,
        };
        assert_eq!(pose(0.0, 0.0, false), rest);
        assert_eq!(pose(0.0, 123.0, false), rest);
    }

    /// The mark's shroud, as the window's mark draws it.
    const MARK_SHROUD: [Segment; 10] = [
        Segment::Move(21.0, 70.0),
        Segment::Line(24.0, 46.0),
        Segment::Cubic([24.0, 27.0, 34.0, 14.0, 50.0, 14.0]),
        Segment::Cubic([66.0, 14.0, 76.0, 27.0, 76.0, 46.0]),
        Segment::Line(79.0, 70.0),
        Segment::Cubic([79.0, 80.0, 72.0, 88.0, 64.0, 86.0]),
        Segment::Cubic([57.0, 84.0, 55.0, 72.0, 50.0, 72.0]),
        Segment::Cubic([45.0, 72.0, 43.0, 84.0, 36.0, 86.0]),
        Segment::Cubic([28.0, 88.0, 21.0, 80.0, 21.0, 70.0]),
        Segment::Close,
    ];

    #[test]
    fn the_resting_shroud_is_the_marks_path() {
        assert_eq!(shroud(0.0, 0.0), MARK_SHROUD);
    }

    #[test]
    fn depth_moves_the_hem_only() {
        assert_eq!(shroud(1.0, 0.0)[..5], shroud(0.0, 0.0)[..5]);
        assert_ne!(shroud(1.0, 0.0)[5..], shroud(0.0, 0.0)[5..]);
    }

    #[test]
    fn a_closed_square_measures_its_four_sides() {
        let square = [
            Segment::Move(0.0, 0.0),
            Segment::Line(10.0, 0.0),
            Segment::Line(10.0, 10.0),
            Segment::Line(0.0, 10.0),
            Segment::Close,
        ];
        assert!((path_length(&square) - 40.0).abs() < 1e-9);
    }

    #[test]
    fn the_halo_length_matches_a_fine_polygon() {
        let (rx, ry) = (42.0, 16.0);
        let pieces = 10_000;
        let point = |i: usize| {
            let angle = std::f64::consts::TAU * i as f64 / pieces as f64;
            (rx * angle.cos(), ry * angle.sin())
        };
        let polygon: f64 = (0..pieces)
            .map(|i| {
                let (x0, y0) = point(i);
                let (x1, y1) = point(i + 1);
                (x1 - x0).hypot(y1 - y0)
            })
            .sum();
        let error = (ellipse_length(rx, ry) - polygon).abs() / polygon;
        assert!(error < 0.001, "off by {:.4}%", error * 100.0);
    }
}
