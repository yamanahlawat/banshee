//! Each gesture as the one Core Animation it adds. Figure values are in the
//! mark's units, y down. Capsule values are points, y up.

use objc2::Message;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_core_foundation::CGFloat;
use objc2_foundation::{NSArray, NSNumber, NSString};
use objc2_quartz_core::{
    CAAnimation, CAAnimationGroup, CACurrentMediaTime, CAKeyframeAnimation, CAMediaTiming,
    CAMediaTimingFunction, kCAFillModeBackwards, kCAFillModeBoth, kCAFillModeForwards,
    kCAMediaTimingFunctionLinear,
};

type Bezier = [f32; 4];

const SETTLE: Bezier = [0.16, 1.0, 0.3, 1.0];
const SWAY: Bezier = [0.45, 0.0, 0.55, 1.0];
const SHAKE: Bezier = [0.36, 0.07, 0.19, 0.97];
const DROP: Bezier = [0.7, 0.0, 0.84, 0.0];
const ASCEND: Bezier = [0.33, 0.0, 0.2, 1.0];

const FADE: f64 = 0.12;

#[derive(Clone, Copy)]
enum Pace {
    Curve(Bezier),
    Linear,
}

#[derive(Clone, Copy)]
enum End {
    Release,
    Hold,
    Repeat,
}

fn numbers(values: &[f64]) -> Retained<NSArray<NSNumber>> {
    NSArray::from_retained_slice(
        &values
            .iter()
            .copied()
            .map(NSNumber::new_f64)
            .collect::<Vec<_>>(),
    )
}

/// One key path through `values` at `times`, eased per segment as a CSS
/// keyframe list eases.
fn track(
    key_path: &str,
    times: &[f64],
    values: &NSArray<AnyObject>,
    pace: Pace,
) -> Retained<CAKeyframeAnimation> {
    let animation = CAKeyframeAnimation::animationWithKeyPath(Some(&NSString::from_str(key_path)));
    // SAFETY: every caller passes NSNumber values, or NSArrays of NSNumber for
    // a dash pattern: the types its key path animates.
    unsafe { animation.setValues(Some(values)) };
    animation.setKeyTimes(Some(&numbers(times)));
    let function = match pace {
        Pace::Curve([x1, y1, x2, y2]) => {
            CAMediaTimingFunction::functionWithControlPoints(x1, y1, x2, y2)
        }
        Pace::Linear => {
            // SAFETY: Core Animation defines the function names as immutable constants.
            let linear = unsafe { kCAMediaTimingFunctionLinear };
            CAMediaTimingFunction::functionWithName(linear)
        }
    };
    let segments = vec![function; times.len().saturating_sub(1)];
    animation.setTimingFunctions(Some(&NSArray::from_retained_slice(&segments)));
    animation
}

fn scalar(
    key_path: &str,
    times: &[f64],
    values: &[f64],
    pace: Pace,
) -> Retained<CAKeyframeAnimation> {
    let values = numbers(values);
    // SAFETY: an array of NSNumber is an array of objects.
    track(key_path, times, unsafe { values.cast_unchecked() }, pace)
}

/// `delay` seconds from now. The layer holds the first keyframe until then.
fn timed(
    animation: Retained<CAAnimation>,
    duration: f64,
    delay: f64,
    end: End,
) -> Retained<CAAnimation> {
    animation.setDuration(duration);
    if delay > 0.0 {
        animation.setBeginTime(CACurrentMediaTime() + delay);
    }
    // SAFETY: Core Animation defines the fill modes as immutable constants.
    let (backwards, forwards) = unsafe { (kCAFillModeBackwards, kCAFillModeForwards) };
    match end {
        End::Release => animation.setFillMode(backwards),
        End::Hold => {
            animation.setFillMode(forwards);
            animation.setRemovedOnCompletion(false);
        }
        End::Repeat => {
            animation.setFillMode(backwards);
            animation.setRepeatCount(f32::INFINITY);
        }
    }
    animation
}

fn one(
    track: Retained<CAKeyframeAnimation>,
    duration: f64,
    delay: f64,
    end: End,
) -> Retained<CAAnimation> {
    timed(track.into_super().into_super(), duration, delay, end)
}

fn pair(
    tracks: [Retained<CAKeyframeAnimation>; 2],
    duration: f64,
    delay: f64,
    end: End,
) -> Retained<CAAnimation> {
    let tracks = tracks.map(|track| {
        track.setDuration(duration);
        // SAFETY: as in `timed`.
        track.setFillMode(unsafe { kCAFillModeBoth });
        track.into_super().into_super()
    });
    let group = CAAnimationGroup::animation();
    group.setAnimations(Some(&NSArray::from_retained_slice(&tracks)));
    timed(group.into_super(), duration, delay, end)
}

fn degrees(values: &[f64]) -> Vec<f64> {
    values.iter().map(|value| value.to_radians()).collect()
}

pub fn chip_rise() -> Retained<CAAnimation> {
    pair(
        [
            scalar(
                "transform.translation.y",
                &[0.0, 1.0],
                &[-12.0, 0.0],
                Pace::Curve(SETTLE),
            ),
            scalar(
                "opacity",
                &[0.0, 0.6, 1.0],
                &[0.0, 1.0, 1.0],
                Pace::Curve(SETTLE),
            ),
        ],
        0.22,
        0.0,
        End::Release,
    )
}

pub fn figure_emerge() -> Retained<CAAnimation> {
    pair(
        [
            scalar(
                "transform.translation.y",
                &[0.0, 1.0],
                &[70.0, 0.0],
                Pace::Curve(SETTLE),
            ),
            scalar("opacity", &[0.0, 1.0], &[0.0, 1.0], Pace::Curve(SETTLE)),
        ],
        0.36,
        0.0,
        End::Release,
    )
}

pub fn fill_kindle(delay: f64) -> Retained<CAAnimation> {
    let times = [0.0, 0.6, 1.0];
    pair(
        [
            scalar("opacity", &times, &[0.0, 1.0, 1.0], Pace::Curve(SETTLE)),
            scalar(
                "transform.scale",
                &times,
                &[0.86, 1.06, 1.0],
                Pace::Curve(SETTLE),
            ),
        ],
        0.26,
        delay,
        End::Release,
    )
}

pub fn working_hover() -> Retained<CAAnimation> {
    one(
        scalar(
            "transform.translation.y",
            &[0.0, 0.5, 1.0],
            &[0.0, -5.0, 0.0],
            Pace::Curve(SWAY),
        ),
        2.4,
        0.0,
        End::Repeat,
    )
}

/// One lap of the ellipse `length` units round.
pub fn halo_light(length: CGFloat) -> Retained<CAAnimation> {
    one(
        scalar("lineDashPhase", &[0.0, 1.0], &[0.0, -length], Pace::Linear),
        1.2,
        0.0,
        End::Repeat,
    )
}

pub fn head_tilt() -> Retained<CAAnimation> {
    one(
        scalar(
            "transform.rotation.z",
            &[0.0, 1.0],
            &degrees(&[0.0, -11.0]),
            Pace::Curve(SETTLE),
        ),
        0.42,
        0.0,
        End::Hold,
    )
}

/// `side` is 1 for the left cup and -1 for the right, so both press inward.
pub fn cups_press(side: f64) -> Retained<CAAnimation> {
    let values = [0.0, 9.0, 0.0, 6.0, 0.0, 0.0].map(|at| at * side);
    one(
        scalar(
            "transform.translation.x",
            &[0.0, 0.2, 0.4, 0.6, 0.8, 1.0],
            &values,
            Pace::Curve(SETTLE),
        ),
        0.64,
        0.18,
        End::Release,
    )
}

pub fn head_shake() -> Retained<CAAnimation> {
    one(
        scalar(
            "transform.rotation.z",
            &[0.0, 0.16, 0.38, 0.6, 0.8, 1.0],
            &degrees(&[0.0, -10.0, 9.0, -6.0, 4.0, 0.0]),
            Pace::Curve(SHAKE),
        ),
        0.46,
        0.0,
        End::Release,
    )
}

pub fn strain() -> Retained<CAAnimation> {
    one(
        scalar(
            "transform.translation.y",
            &[0.0, 0.35, 0.48, 0.62, 1.0],
            &[0.0, -16.0, -13.0, -16.0, 4.0],
            Pace::Curve(SETTLE),
        ),
        0.9,
        0.0,
        End::Hold,
    )
}

/// From solid to the dashes `dash` holds, on an outline `length` units round.
pub fn outline_breaks(length: CGFloat, dash: &NSArray<NSNumber>) -> Retained<CAAnimation> {
    let patterns = NSArray::from_retained_slice(&[numbers(&[length, 0.0]), dash.retain()]);
    // SAFETY: an array of arrays is an array of objects.
    let values = unsafe { patterns.cast_unchecked() };
    one(
        track("lineDashPattern", &[0.0, 1.0], values, Pace::Curve(DROP)),
        0.52,
        0.42,
        End::Release,
    )
}

/// On the words' mask, `width` points wide when whole.
pub fn words_reveal(width: CGFloat) -> Retained<CAAnimation> {
    pair(
        [
            scalar(
                "bounds.size.width",
                &[0.0, 1.0],
                &[0.0, width],
                Pace::Curve(SETTLE),
            ),
            scalar("opacity", &[0.0, 1.0], &[0.2, 1.0], Pace::Curve(SETTLE)),
        ],
        0.22,
        0.08,
        End::Release,
    )
}

pub fn chip_width(from: CGFloat, to: CGFloat) -> Retained<CAAnimation> {
    one(
        scalar(
            "bounds.size.width",
            &[0.0, 1.0],
            &[from, to],
            Pace::Curve(SETTLE),
        ),
        0.24,
        0.0,
        End::Release,
    )
}

pub fn float_out() -> Retained<CAAnimation> {
    let times = [0.0, 0.25, 1.0];
    pair(
        [
            scalar(
                "transform.translation.y",
                &times,
                &[0.0, 4.0, -120.0],
                Pace::Curve(ASCEND),
            ),
            scalar("opacity", &times, &[1.0, 1.0, 0.0], Pace::Curve(ASCEND)),
        ],
        0.44,
        0.0,
        End::Hold,
    )
}

fn sink(duration: f64, delay: f64) -> Retained<CAAnimation> {
    pair(
        [
            scalar(
                "transform.translation.y",
                &[0.0, 1.0],
                &[0.0, -8.0],
                Pace::Curve(DROP),
            ),
            scalar("opacity", &[0.0, 1.0], &[1.0, 0.0], Pace::Curve(DROP)),
        ],
        duration,
        delay,
        End::Hold,
    )
}

pub fn frame_sink() -> Retained<CAAnimation> {
    sink(0.14, 0.3)
}

pub fn failure_exit() -> Retained<CAAnimation> {
    sink(0.15, 0.0)
}

pub fn fade_in(delay: f64) -> Retained<CAAnimation> {
    one(
        scalar("opacity", &[0.0, 1.0], &[0.0, 1.0], Pace::Linear),
        FADE,
        delay,
        End::Release,
    )
}

pub fn fade_out() -> Retained<CAAnimation> {
    one(
        scalar("opacity", &[0.0, 1.0], &[1.0, 0.0], Pace::Linear),
        FADE,
        0.0,
        End::Hold,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tracks(animation: &CAAnimation) -> Vec<Retained<CAKeyframeAnimation>> {
        if let Some(group) = animation.downcast_ref::<CAAnimationGroup>() {
            return group
                .animations()
                .unwrap()
                .iter()
                .flat_map(|child| tracks(&child))
                .collect();
        }
        vec![
            animation
                .downcast_ref::<CAKeyframeAnimation>()
                .unwrap()
                .retain(),
        ]
    }

    #[test]
    fn every_track_has_one_value_per_key_time_and_one_curve_per_segment() {
        let dash = numbers(&[8.8, 5.6]);
        let all = [
            chip_rise(),
            figure_emerge(),
            fill_kindle(0.2),
            working_hover(),
            halo_light(100.0),
            head_tilt(),
            cups_press(1.0),
            head_shake(),
            strain(),
            outline_breaks(100.0, &dash),
            words_reveal(40.0),
            chip_width(44.0, 120.0),
            float_out(),
            frame_sink(),
            failure_exit(),
            fade_in(0.0),
            fade_out(),
        ];
        for animation in &all {
            for track in tracks(animation) {
                let times = track.keyTimes().unwrap().count();
                assert_eq!(
                    track.values().unwrap().count(),
                    times,
                    "{:?}",
                    track.keyPath()
                );
                assert_eq!(
                    track.timingFunctions().unwrap().count(),
                    times - 1,
                    "{:?}",
                    track.keyPath()
                );
            }
        }
    }

    #[test]
    fn the_cups_press_toward_each_other() {
        let at_first_press = |side| {
            let track = tracks(&cups_press(side)).remove(0);
            track
                .values()
                .unwrap()
                .objectAtIndex(1)
                .downcast::<NSNumber>()
                .unwrap()
                .as_f64()
        };
        assert_eq!(at_first_press(1.0), 9.0);
        assert_eq!(at_first_press(-1.0), -9.0);
    }

    #[test]
    fn a_delay_holds_the_first_keyframe_and_a_held_gesture_stays() {
        let kindle = fill_kindle(0.2);
        assert!(kindle.beginTime() > CACurrentMediaTime());
        // SAFETY: Core Animation defines the fill modes as immutable constants.
        assert_eq!(&*kindle.fillMode(), unsafe { kCAFillModeBackwards });
        let tilt = head_tilt();
        assert!(!tilt.isRemovedOnCompletion());
        assert_eq!(tilt.beginTime(), 0.0);
    }

    #[test]
    fn a_delayed_sink_shows_the_model_until_it_starts() {
        let sink = frame_sink();
        assert!(sink.beginTime() > CACurrentMediaTime());
        assert!(!sink.isRemovedOnCompletion());
        // SAFETY: Core Animation defines the fill modes as immutable constants.
        assert_eq!(&*sink.fillMode(), unsafe { kCAFillModeForwards });
    }
}
