//! The capsule's bars, arcs and dots as shapes in a box `SLOT` wide and
//! `GLYPH` tall, origin top left, y down. Strokes have round caps.

use crate::figure::Segment;

pub const GLYPH: f64 = 20.0;
pub const SLOT: f64 = 23.0;
pub const DOTS: usize = 3;

const MIDDLE: f64 = 10.0;
const BAR_WIDTH: f64 = 3.0;
const BAR_PITCH: f64 = 5.0;
const REST_HEIGHTS: [f64; 5] = [6.0, 12.0, 16.0, 10.0, 7.0];
const CALM_HEIGHT: f64 = 12.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Glyph {
    Bars,
    BarsQuiet,
    Arcs,
    Dots,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Mark {
    Stroke {
        segments: Vec<Segment>,
        width: f64,
        opacity: f64,
    },
    Disc {
        centre: (f64, f64),
        radius: f64,
    },
}

/// `level` is the smoothed microphone level in 0..=1; only Bars read it.
pub fn marks(glyph: Glyph, level: f64, calm: bool) -> Vec<Mark> {
    match glyph {
        Glyph::Bars => REST_HEIGHTS
            .iter()
            .enumerate()
            .map(|(i, rest)| {
                if calm {
                    bar(i, CALM_HEIGHT, 0.70 + 0.30 * level)
                } else {
                    bar(i, rest * (0.45 + 0.55 * level), 1.0)
                }
            })
            .collect(),
        Glyph::BarsQuiet => (0..REST_HEIGHTS.len()).map(|i| bar(i, 3.0, 1.0)).collect(),
        Glyph::Arcs => vec![
            Mark::Disc {
                centre: (4.0, MIDDLE),
                radius: 2.6,
            },
            arc([9.0, 5.5, 11.0, 8.0, 11.0, 12.0, 9.0, 14.5]),
            arc([13.0, 2.5, 17.0, 7.0, 17.0, 13.0, 13.0, 17.5]),
        ],
        Glyph::Dots => (0..DOTS)
            .map(|i| Mark::Disc {
                centre: (2.5 + 9.0 * i as f64, MIDDLE),
                radius: 2.5,
            })
            .collect(),
    }
}

fn bar(index: usize, height: f64, opacity: f64) -> Mark {
    let x = BAR_WIDTH / 2.0 + BAR_PITCH * index as f64;
    Mark::Stroke {
        segments: vec![
            Segment::Move(x, MIDDLE - height / 2.0),
            Segment::Line(x, MIDDLE + height / 2.0),
        ],
        width: BAR_WIDTH,
        opacity,
    }
}

fn arc([x0, y0, x1, y1, x2, y2, x, y]: [f64; 8]) -> Mark {
    Mark::Stroke {
        segments: vec![
            Segment::Move(x0, y0),
            Segment::Cubic([x1, y1, x2, y2, x, y]),
        ],
        width: 1.9,
        opacity: 1.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn heights(marks: &[Mark]) -> Vec<f64> {
        marks
            .iter()
            .map(|mark| match mark {
                Mark::Stroke { segments, .. } => match segments[..] {
                    [Segment::Move(_, top), Segment::Line(_, bottom)] => bottom - top,
                    _ => panic!("not a bar"),
                },
                Mark::Disc { .. } => panic!("not a bar"),
            })
            .collect()
    }

    fn opacities(marks: &[Mark]) -> Vec<f64> {
        marks
            .iter()
            .map(|mark| match mark {
                Mark::Stroke { opacity, .. } => *opacity,
                Mark::Disc { .. } => panic!("not a bar"),
            })
            .collect()
    }

    fn assert_close(got: &[f64], want: &[f64]) {
        assert_eq!(got.len(), want.len());
        for (g, w) in got.iter().zip(want) {
            assert!((g - w).abs() < 0.01, "{got:?} vs {want:?}");
        }
    }

    #[test]
    fn bars_follow_the_level() {
        let silent = marks(Glyph::Bars, 0.0, false);
        assert_close(&heights(&silent), &[2.7, 5.4, 7.2, 4.5, 3.15]);
        assert_close(
            &heights(&marks(Glyph::Bars, 1.0, false)),
            &[6.0, 12.0, 16.0, 10.0, 7.0],
        );
        for (i, mark) in silent.iter().enumerate() {
            let Mark::Stroke { segments, .. } = mark else {
                panic!("not a bar");
            };
            let Segment::Move(x, _) = segments[0] else {
                panic!("not a bar");
            };
            assert_eq!(x, 1.5 + 5.0 * i as f64);
        }
    }

    #[test]
    fn calm_bars_hold_one_height_and_carry_the_level_as_opacity() {
        let silent = marks(Glyph::Bars, 0.0, true);
        assert_close(&heights(&silent), &[12.0; 5]);
        assert_close(&opacities(&silent), &[0.70; 5]);
        assert_close(&opacities(&marks(Glyph::Bars, 1.0, true)), &[1.0; 5]);
    }

    #[test]
    fn quiet_bars_are_three_tall_whatever_the_level() {
        assert_close(&heights(&marks(Glyph::BarsQuiet, 1.0, false)), &[3.0; 5]);
    }

    #[test]
    fn arcs_and_dots_ignore_the_level() {
        assert_eq!(
            marks(Glyph::Arcs, 0.0, false),
            marks(Glyph::Arcs, 1.0, true)
        );
        let centres: Vec<f64> = marks(Glyph::Dots, 0.5, false)
            .iter()
            .map(|mark| match mark {
                Mark::Disc { centre, .. } => centre.0,
                Mark::Stroke { .. } => panic!("not a dot"),
            })
            .collect();
        assert_eq!(centres, [2.5, 11.5, 20.5]);
    }

    #[test]
    fn every_glyph_fits_the_slot() {
        for glyph in [Glyph::Bars, Glyph::BarsQuiet, Glyph::Arcs, Glyph::Dots] {
            for level in [0.0, 1.0] {
                for mark in marks(glyph, level, false) {
                    let points: Vec<(f64, f64)> = match &mark {
                        Mark::Disc { centre, radius, .. } => vec![
                            (centre.0 - radius, centre.1 - radius),
                            (centre.0 + radius, centre.1 + radius),
                        ],
                        Mark::Stroke {
                            segments, width, ..
                        } => segments
                            .iter()
                            .flat_map(|segment| match *segment {
                                Segment::Move(x, y) | Segment::Line(x, y) => vec![(x, y)],
                                Segment::Cubic([a, b, c, d, x, y]) => {
                                    vec![(a, b), (c, d), (x, y)]
                                }
                                Segment::Close => vec![],
                            })
                            .flat_map(|(x, y)| {
                                let r = width / 2.0;
                                [(x - r, y - r), (x + r, y + r)]
                            })
                            .collect(),
                    };
                    for (x, y) in points {
                        assert!((0.0..=SLOT).contains(&x), "{glyph:?} x {x}");
                        assert!((0.0..=GLYPH).contains(&y), "{glyph:?} y {y}");
                    }
                }
            }
        }
    }
}
