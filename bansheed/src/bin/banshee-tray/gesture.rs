//! Each gesture as keyframe tracks over time. Figure values are in the mark's
//! units, y down. Capsule values are points, y up. Rotations are radians.

use std::borrow::Cow;

use crate::figure::outline_dash;

/// A CSS cubic bezier's control points: x1, y1, x2, y2.
pub type Bezier = [f64; 4];

pub const SETTLE: Bezier = [0.16, 1.0, 0.3, 1.0];
pub const SWAY: Bezier = [0.45, 0.0, 0.55, 1.0];
pub const SHAKE: Bezier = [0.36, 0.07, 0.19, 0.97];
pub const DROP: Bezier = [0.7, 0.0, 0.84, 0.0];
pub const ASCEND: Bezier = [0.33, 0.0, 0.2, 1.0];
pub const LINEAR: Bezier = [0.0, 0.0, 1.0, 1.0];

pub const FADE: f64 = 0.12;

/// What a track shows once its last keyframe has passed.
#[derive(Clone, Copy)]
pub enum End {
    Hold,
    Release,
    Repeat,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Property {
    TranslationX,
    TranslationY,
    Rotation,
    Scale,
    Opacity,
    DashPhase,
    Dash,
    Gap,
    Width,
}

/// One property through `values` at `key_times`, eased per segment by
/// `curves` as a CSS keyframe list eases. Key times are shares of `duration`.
pub struct Track {
    pub property: Property,
    pub key_times: &'static [f64],
    pub values: Cow<'static, [f64]>,
    pub curves: &'static [Bezier],
    pub duration: f64,
    pub delay: f64,
    pub end: End,
}

type Keys = (
    Property,
    &'static [f64],
    Cow<'static, [f64]>,
    &'static [Bezier],
);

fn gesture<const N: usize>(keys: [Keys; N], duration: f64, delay: f64, end: End) -> Vec<Track> {
    keys.into_iter()
        .map(|(property, key_times, values, curves)| Track {
            property,
            key_times,
            values,
            curves,
            duration,
            delay,
            end,
        })
        .collect()
}

const ENDS: &[f64] = &[0.0, 1.0];

pub fn chip_rise() -> Vec<Track> {
    gesture(
        [
            (
                Property::TranslationY,
                ENDS,
                Cow::Borrowed(&[-12.0, 0.0]),
                &[SETTLE],
            ),
            (
                Property::Opacity,
                &[0.0, 0.6, 1.0],
                Cow::Borrowed(&[0.0, 1.0, 1.0]),
                &[SETTLE; 2],
            ),
        ],
        0.22,
        0.0,
        End::Release,
    )
}

pub fn figure_emerge() -> Vec<Track> {
    gesture(
        [
            (
                Property::TranslationY,
                ENDS,
                Cow::Borrowed(&[70.0, 0.0]),
                &[SETTLE],
            ),
            (
                Property::Opacity,
                ENDS,
                Cow::Borrowed(&[0.0, 1.0]),
                &[SETTLE],
            ),
        ],
        0.36,
        0.0,
        End::Release,
    )
}

pub fn fill_kindle(delay: f64) -> Vec<Track> {
    const TIMES: &[f64] = &[0.0, 0.6, 1.0];
    gesture(
        [
            (
                Property::Opacity,
                TIMES,
                Cow::Borrowed(&[0.0, 1.0, 1.0]),
                &[SETTLE; 2],
            ),
            (
                Property::Scale,
                TIMES,
                Cow::Borrowed(&[0.86, 1.06, 1.0]),
                &[SETTLE; 2],
            ),
        ],
        0.26,
        delay,
        End::Release,
    )
}

pub fn working_hover() -> Vec<Track> {
    gesture(
        [(
            Property::TranslationY,
            &[0.0, 0.5, 1.0],
            Cow::Borrowed(&[0.0, -5.0, 0.0]),
            &[SWAY; 2],
        )],
        2.4,
        0.0,
        End::Repeat,
    )
}

/// One lap of the ellipse `length` units round.
pub fn halo_light(length: f64) -> Vec<Track> {
    gesture(
        [(
            Property::DashPhase,
            ENDS,
            Cow::Owned(vec![0.0, -length]),
            &[LINEAR],
        )],
        1.2,
        0.0,
        End::Repeat,
    )
}

fn radians(degrees: &[f64]) -> Cow<'static, [f64]> {
    degrees.iter().map(|value| value.to_radians()).collect()
}

pub fn head_tilt() -> Vec<Track> {
    gesture(
        [(Property::Rotation, ENDS, radians(&[0.0, -11.0]), &[SETTLE])],
        0.42,
        0.0,
        End::Hold,
    )
}

/// `side` is 1 for the left cup and -1 for the right, so both press inward.
pub fn cups_press(side: f64) -> Vec<Track> {
    gesture(
        [(
            Property::TranslationX,
            &[0.0, 0.2, 0.4, 0.6, 0.8, 1.0],
            Cow::Owned([0.0, 9.0, 0.0, 6.0, 0.0, 0.0].map(|at| at * side).to_vec()),
            &[SETTLE; 5],
        )],
        0.64,
        0.18,
        End::Release,
    )
}

pub fn head_shake() -> Vec<Track> {
    gesture(
        [(
            Property::Rotation,
            &[0.0, 0.16, 0.38, 0.6, 0.8, 1.0],
            radians(&[0.0, -10.0, 9.0, -6.0, 4.0, 0.0]),
            &[SHAKE; 5],
        )],
        0.46,
        0.0,
        End::Release,
    )
}

pub fn strain() -> Vec<Track> {
    gesture(
        [(
            Property::TranslationY,
            &[0.0, 0.35, 0.48, 0.62, 1.0],
            Cow::Borrowed(&[0.0, -16.0, -13.0, -16.0, 4.0]),
            &[SETTLE; 4],
        )],
        0.9,
        0.0,
        End::Hold,
    )
}

/// From solid to dashes, on an outline `length` units round. The two tracks
/// are the dash pattern's dash and gap.
pub fn outline_breaks(length: f64) -> Vec<Track> {
    let [dash, gap] = outline_dash(length);
    gesture(
        [
            (
                Property::Dash,
                ENDS,
                Cow::Owned(vec![length, dash]),
                &[DROP],
            ),
            (Property::Gap, ENDS, Cow::Owned(vec![0.0, gap]), &[DROP]),
        ],
        0.52,
        0.42,
        End::Release,
    )
}

/// On the words' mask, `width` points wide when whole.
pub fn words_reveal(width: f64) -> Vec<Track> {
    gesture(
        [
            (
                Property::Width,
                ENDS,
                Cow::Owned(vec![0.0, width]),
                &[SETTLE],
            ),
            (
                Property::Opacity,
                ENDS,
                Cow::Borrowed(&[0.2, 1.0]),
                &[SETTLE],
            ),
        ],
        0.22,
        0.08,
        End::Release,
    )
}

pub fn chip_width(from: f64, to: f64) -> Track {
    Track {
        property: Property::Width,
        key_times: ENDS,
        values: Cow::Owned(vec![from, to]),
        curves: &[SETTLE],
        duration: 0.24,
        delay: 0.0,
        end: End::Release,
    }
}

pub fn float_out() -> Vec<Track> {
    const TIMES: &[f64] = &[0.0, 0.25, 1.0];
    gesture(
        [
            (
                Property::TranslationY,
                TIMES,
                Cow::Borrowed(&[0.0, 4.0, -120.0]),
                &[ASCEND; 2],
            ),
            (
                Property::Opacity,
                TIMES,
                Cow::Borrowed(&[1.0, 1.0, 0.0]),
                &[ASCEND; 2],
            ),
        ],
        0.44,
        0.0,
        End::Hold,
    )
}

fn sink(duration: f64, delay: f64) -> Vec<Track> {
    gesture(
        [
            (
                Property::TranslationY,
                ENDS,
                Cow::Borrowed(&[0.0, -8.0]),
                &[DROP],
            ),
            (Property::Opacity, ENDS, Cow::Borrowed(&[1.0, 0.0]), &[DROP]),
        ],
        duration,
        delay,
        End::Hold,
    )
}

pub fn frame_sink() -> Vec<Track> {
    sink(0.14, 0.3)
}

pub fn failure_exit() -> Vec<Track> {
    sink(0.15, 0.0)
}

pub fn fade_in(delay: f64) -> Vec<Track> {
    gesture(
        [(
            Property::Opacity,
            ENDS,
            Cow::Borrowed(&[0.0, 1.0]),
            &[LINEAR],
        )],
        FADE,
        delay,
        End::Release,
    )
}

pub fn fade_out() -> Vec<Track> {
    gesture(
        [(
            Property::Opacity,
            ENDS,
            Cow::Borrowed(&[1.0, 0.0]),
            &[LINEAR],
        )],
        FADE,
        0.0,
        End::Hold,
    )
}

/// How long `tracks` take to end once added. `None` for a gesture that repeats.
#[cfg(any(target_os = "linux", test))]
pub fn lasts(tracks: &[Track]) -> Option<f64> {
    tracks
        .iter()
        .try_fold(0.0, |longest: f64, track| match track.end {
            End::Repeat => None,
            End::Hold | End::Release => Some(longest.max(track.delay + track.duration)),
        })
}

/// The value `track` gives `elapsed` seconds after it was added, over a layer
/// whose own value is `model`. Before the delay a held track shows the model
/// and any other shows its first value, as Core Animation's fill modes do.
#[cfg(any(target_os = "linux", test))]
pub fn at(track: &Track, elapsed: f64, model: f64) -> f64 {
    let values = &track.values;
    let mut t = elapsed - track.delay;
    if t < 0.0 {
        return match track.end {
            End::Hold => model,
            End::Release | End::Repeat => values[0],
        };
    }
    if t >= track.duration {
        match track.end {
            End::Hold => return values[values.len() - 1],
            End::Release => return model,
            End::Repeat => t %= track.duration,
        }
    }
    let progress = t / track.duration;
    let times = track.key_times;
    let segment = times[1..]
        .iter()
        .position(|&end| progress < end)
        .unwrap_or(times.len() - 2);
    let (start, end) = (times[segment], times[segment + 1]);
    let eased = ease(track.curves[segment], (progress - start) / (end - start));
    values[segment] + (values[segment + 1] - values[segment]) * eased
}

/// The eased progress at `x` along `curve`.
#[cfg(any(target_os = "linux", test))]
pub fn ease([x1, y1, x2, y2]: Bezier, x: f64) -> f64 {
    const CLOSE: f64 = 1e-7;
    let along = |p1: f64, p2: f64, t: f64| {
        let u = 1.0 - t;
        3.0 * u * u * t * p1 + 3.0 * u * t * t * p2 + t * t * t
    };
    let slope = |t: f64| {
        let u = 1.0 - t;
        3.0 * u * u * x1 + 6.0 * u * t * (x2 - x1) + 3.0 * t * t * (1.0 - x2)
    };
    let mut t = x;
    for _ in 0..8 {
        let error = along(x1, x2, t) - x;
        if error.abs() < CLOSE {
            return along(y1, y2, t);
        }
        let steep = slope(t);
        if steep.abs() < CLOSE {
            break;
        }
        t -= error / steep;
    }
    let (mut low, mut high) = (0.0, 1.0);
    t = x.clamp(low, high);
    while high - low > CLOSE {
        if along(x1, x2, t) < x {
            low = t;
        } else {
            high = t;
        }
        t = (low + high) / 2.0;
    }
    along(y1, y2, t)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn every_gesture() -> Vec<Vec<Track>> {
        vec![
            chip_rise(),
            figure_emerge(),
            fill_kindle(0.2),
            working_hover(),
            halo_light(100.0),
            head_tilt(),
            cups_press(1.0),
            head_shake(),
            strain(),
            outline_breaks(100.0),
            words_reveal(40.0),
            vec![chip_width(44.0, 120.0)],
            float_out(),
            frame_sink(),
            failure_exit(),
            fade_in(0.0),
            fade_out(),
        ]
    }

    fn one(mut tracks: Vec<Track>) -> Track {
        tracks.remove(0)
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn every_track_has_one_value_per_key_time_and_one_curve_per_segment() {
        for track in every_gesture().into_iter().flatten() {
            let times = track.key_times.len();
            assert_eq!(track.values.len(), times, "{:?}", track.property);
            assert_eq!(track.curves.len(), times - 1, "{:?}", track.property);
        }
    }

    #[test]
    fn ease_meets_the_ends_and_settle_runs_ahead() {
        assert_eq!(ease(SETTLE, 0.0), 0.0);
        assert_eq!(ease(SETTLE, 1.0), 1.0);
        assert!(ease(SETTLE, 0.5) > 0.5);
    }

    #[test]
    fn ease_follows_settles_curve_between_the_ends() {
        let eased = ease(SETTLE, 0.5);
        assert!((eased - 0.97178).abs() < 0.001, "{eased}");
    }

    #[test]
    fn ease_solves_for_the_curves_parameter_before_reading_its_height() {
        assert!(close(ease(LINEAR, 0.25), 0.25), "{}", ease(LINEAR, 0.25));
    }

    #[test]
    fn at_eases_between_key_times() {
        let width = chip_width(0.0, 1.0);
        let halfway = at(&width, width.delay + width.duration / 2.0, 99.0);
        assert!(close(halfway, ease(SETTLE, 0.5)), "{halfway}");
    }

    #[test]
    fn at_holds_the_model_before_the_delay() {
        let sink = one(frame_sink());
        assert_eq!(at(&sink, 0.0, 0.7), 0.7);
        assert_eq!(at(&sink, sink.delay - 0.01, 0.7), 0.7);
    }

    #[test]
    fn at_holds_the_first_value_before_a_released_tracks_delay() {
        let press = one(cups_press(1.0));
        assert_eq!(at(&press, press.delay - 0.01, 3.0), press.values[0]);
    }

    #[test]
    fn at_lands_on_each_key_value_at_its_key_time() {
        let strain = one(strain());
        for (key_time, value) in strain.key_times.iter().zip(strain.values.iter()) {
            let elapsed = strain.delay + key_time * strain.duration;
            assert!(close(at(&strain, elapsed, 99.0), *value), "at {key_time}");
        }
        let press = one(cups_press(-1.0));
        let last = press.key_times.len() - 1;
        for (key_time, value) in press.key_times[..last].iter().zip(press.values.iter()) {
            let elapsed = press.delay + key_time * press.duration;
            assert!(close(at(&press, elapsed, 99.0), *value), "at {key_time}");
        }
    }

    #[test]
    fn at_keeps_the_last_value_under_hold() {
        let tilt = one(head_tilt());
        let last = *tilt.values.last().unwrap();
        assert_eq!(at(&tilt, tilt.duration + 5.0, 0.0), last);
    }

    #[test]
    fn at_returns_the_model_under_release() {
        let shake = one(head_shake());
        assert_eq!(at(&shake, shake.duration + 0.5, 0.25), 0.25);
    }

    #[test]
    fn at_repeats_under_repeat() {
        let hover = one(working_hover());
        for x in [0.1, 0.7, 1.3, 2.0] {
            let first = at(&hover, hover.delay + x, 0.0);
            let again = at(&hover, hover.delay + hover.duration + x, 0.0);
            assert!(close(first, again), "x {x}: {first} then {again}");
            assert!(first != 0.0, "x {x}");
        }
    }
}
