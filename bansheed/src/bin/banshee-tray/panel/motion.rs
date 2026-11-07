//! Each gesture as the one Core Animation it adds, built from its `gesture`
//! tracks.

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_core_foundation::CGFloat;
use objc2_foundation::{NSArray, NSNumber, NSString};
use objc2_quartz_core::{
    CAAnimation, CAAnimationGroup, CACurrentMediaTime, CAKeyframeAnimation, CAMediaTiming,
    CAMediaTimingFunction, kCAFillModeBackwards, kCAFillModeBoth, kCAFillModeForwards,
    kCAMediaTimingFunctionLinear,
};

use crate::gesture::{self, Bezier, End, LINEAR, Property, Track};

fn numbers(values: &[f64]) -> Retained<NSArray<NSNumber>> {
    NSArray::from_retained_slice(
        &values
            .iter()
            .copied()
            .map(NSNumber::new_f64)
            .collect::<Vec<_>>(),
    )
}

fn key_path(property: Property) -> &'static str {
    match property {
        Property::TranslationX => "transform.translation.x",
        Property::TranslationY => "transform.translation.y",
        Property::Rotation => "transform.rotation.z",
        Property::Scale => "transform.scale",
        Property::Opacity => "opacity",
        Property::DashPhase => "lineDashPhase",
        Property::Dash | Property::Gap => "lineDashPattern",
        Property::Width => "bounds.size.width",
    }
}

/// One key path through `values` at `times`, eased per segment as a CSS
/// keyframe list eases.
fn track(
    key_path: &str,
    times: &[f64],
    values: &NSArray<AnyObject>,
    curves: &[Bezier],
) -> Retained<CAKeyframeAnimation> {
    let animation = CAKeyframeAnimation::animationWithKeyPath(Some(&NSString::from_str(key_path)));
    // SAFETY: every caller passes NSNumber values, or NSArrays of NSNumber for
    // a dash pattern: the types its key path animates.
    unsafe { animation.setValues(Some(values)) };
    animation.setKeyTimes(Some(&numbers(times)));
    let segments: Vec<_> = curves
        .iter()
        .map(|&curve| {
            if curve == LINEAR {
                // SAFETY: Core Animation defines the function names as immutable constants.
                let linear = unsafe { kCAMediaTimingFunctionLinear };
                CAMediaTimingFunction::functionWithName(linear)
            } else {
                let [x1, y1, x2, y2] = curve.map(|point| point as f32);
                CAMediaTimingFunction::functionWithControlPoints(x1, y1, x2, y2)
            }
        })
        .collect();
    animation.setTimingFunctions(Some(&NSArray::from_retained_slice(&segments)));
    animation
}

fn scalar(keys: &Track) -> Retained<CAKeyframeAnimation> {
    let values = numbers(&keys.values);
    // SAFETY: an array of NSNumber is an array of objects.
    let values = unsafe { values.cast_unchecked() };
    track(key_path(keys.property), keys.key_times, values, keys.curves)
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

/// A gesture's tracks as one animation, or as a group when it moves two
/// properties together.
fn animation(tracks: &[Track]) -> Retained<CAAnimation> {
    match tracks {
        [only] => one(scalar(only), only.duration, only.delay, only.end),
        [first, second] => pair(
            [scalar(first), scalar(second)],
            first.duration,
            first.delay,
            first.end,
        ),
        _ => unreachable!("every gesture moves one or two properties"),
    }
}

pub fn chip_rise() -> Retained<CAAnimation> {
    animation(&gesture::chip_rise())
}

pub fn figure_emerge() -> Retained<CAAnimation> {
    animation(&gesture::figure_emerge())
}

pub fn fill_kindle(delay: f64) -> Retained<CAAnimation> {
    animation(&gesture::fill_kindle(delay))
}

pub fn working_hover() -> Retained<CAAnimation> {
    animation(&gesture::working_hover())
}

pub fn halo_light(length: CGFloat) -> Retained<CAAnimation> {
    animation(&gesture::halo_light(length))
}

pub fn head_tilt() -> Retained<CAAnimation> {
    animation(&gesture::head_tilt())
}

pub fn cups_press(side: f64) -> Retained<CAAnimation> {
    animation(&gesture::cups_press(side))
}

pub fn head_shake() -> Retained<CAAnimation> {
    animation(&gesture::head_shake())
}

pub fn strain() -> Retained<CAAnimation> {
    animation(&gesture::strain())
}

/// The dash and gap tracks as one track of dash patterns.
pub fn outline_breaks(length: CGFloat) -> Retained<CAAnimation> {
    let tracks = gesture::outline_breaks(length);
    let [dash, gap] = &tracks[..] else {
        unreachable!("an outline breaks by its dash and its gap");
    };
    let patterns: Vec<_> = dash
        .values
        .iter()
        .zip(gap.values.iter())
        .map(|(&dash, &gap)| numbers(&[dash, gap]))
        .collect();
    let patterns = NSArray::from_retained_slice(&patterns);
    // SAFETY: an array of arrays is an array of objects.
    let values = unsafe { patterns.cast_unchecked() };
    one(
        track(key_path(dash.property), dash.key_times, values, dash.curves),
        dash.duration,
        dash.delay,
        dash.end,
    )
}

pub fn words_reveal(width: CGFloat) -> Retained<CAAnimation> {
    animation(&gesture::words_reveal(width))
}

pub fn chip_width(from: CGFloat, to: CGFloat) -> Retained<CAAnimation> {
    animation(&[gesture::chip_width(from, to)])
}

pub fn float_out() -> Retained<CAAnimation> {
    animation(&gesture::float_out())
}

pub fn frame_sink() -> Retained<CAAnimation> {
    animation(&gesture::frame_sink())
}

pub fn failure_exit() -> Retained<CAAnimation> {
    animation(&gesture::failure_exit())
}

pub fn fade_in(delay: f64) -> Retained<CAAnimation> {
    animation(&gesture::fade_in(delay))
}

pub fn fade_out() -> Retained<CAAnimation> {
    animation(&gesture::fade_out())
}

#[cfg(test)]
mod tests {
    use objc2::Message;

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
