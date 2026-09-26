//! The chip: the app icon come alive. A rust capsule above the Dock holds the
//! figure and its words in bone, in a panel that never takes focus. Every
//! call runs on the main thread.

use std::cell::Cell;
use std::ptr;
use std::rc::Rc;
use std::time::Instant;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSAccessibility, NSAccessibilityAnnouncementKey,
    NSAccessibilityAnnouncementRequestedNotification, NSAccessibilityPostNotificationWithUserInfo,
    NSAccessibilityPriorityKey, NSAccessibilityPriorityLevel, NSApplication, NSBackingStoreType,
    NSColor, NSEvent, NSPanel, NSResponder, NSScreen, NSStatusWindowLevel, NSView,
    NSWindowCollectionBehavior, NSWindowStyleMask, NSWorkspace,
};
use objc2_core_foundation::{
    CFAttributedString, CFData, CFDictionary, CFNumber, CFPreferencesAppSynchronize,
    CFPreferencesCopyAppValue, CFPreferencesGetAppBooleanValue, CFRange, CFRetained, CFString,
    CFType, CGAffineTransform, CGFloat, CGPoint, CGRect, CGSize,
};
use objc2_core_graphics::{CGColor, CGGlyph, CGMutablePath, CGPath};
use objc2_core_text::{
    CTFont, CTFontDescriptor, CTFontManagerCreateFontDescriptorsFromData, CTFontUIFontType, CTLine,
    CTLineTruncationType, CTRun, kCTFontAttributeName, kCTKernAttributeName,
};
use objc2_foundation::{NSArray, NSDictionary, NSNumber, NSString, ns_string};
use objc2_quartz_core::{
    CAAnimation, CALayer, CAShapeLayer, CATransaction, CATransform3D, kCALineCapRound,
    kCALineJoinRound,
};

use super::chip::{Scene, Show};
use super::figure::{self, Pose, Segment, Voice};

mod motion;

const FIGURE: CGFloat = 32.0;
const CAPSULE: CGFloat = 44.0;
const INSET: CGFloat = (CAPSULE - FIGURE) / 2.0;
/// Clear space below the capsule, so the rise and the sinks travel without a cut.
const TRAVEL: CGFloat = 12.0;
const WORDS_GAP: CGFloat = 4.0;
const WORDS_TAIL: CGFloat = 16.0;
const WORDS_SIZE: CGFloat = 16.0;
const WORDS_TRACKING: CGFloat = -0.015;
const WORDS_WEIGHT: CGFloat = 750.0;
const WORDS_WIDTH: CGFloat = 106.0;
/// Unmeasured.
const ABOVE_DOCK: CGFloat = 12.0;
const SCREEN_MARGIN: CGFloat = 12.0;
/// Measured: a Dock with 54pt tiles shows about 79pt tall.
const DOCK_ABOVE_TILE: CGFloat = 26.0;
/// Unmeasured: the tile size the Dock uses before anyone sets one.
const DOCK_TILE_DEFAULT: CGFloat = 48.0;

const RUST: u32 = 0x8A2A0D;
const BONE: u32 = 0xF2EFE9;
const RING_STRENGTH: CGFloat = 0.45;
const OUTLINE: CGFloat = 9.0;
const OUTLINE_CONTRAST: CGFloat = 12.0;
const RING: CGFloat = 6.0;
const LIGHT: CGFloat = 7.0;
const BEHIND: CGFloat = 15.0;
const HALO: (CGFloat, CGFloat, CGFloat, CGFloat) = (50.0, 40.0, 42.0, 16.0);
const CUPS: [(CGFloat, CGFloat); 2] = [(15.0, 46.0), (85.0, 46.0)];
const CUP_RADII: (CGFloat, CGFloat) = (12.0, 17.0);
const CUP_GAP: CGFloat = 10.0;
const HEADBAND: [Segment; 2] = [
    Segment::Move(15.0, 32.0),
    Segment::Cubic([15.0, -4.0, 85.0, -4.0, 85.0, 32.0]),
];
const BAND: CGFloat = 6.0;
const BAND_GAP: CGFloat = 14.0;
const WORKING_DEPTH: f64 = 0.3;
/// The gesture, voice and fill layers turn and scale about the figure's feet.
const PIVOT: (CGFloat, CGFloat) = (50.0, 88.0);
const KINDLE_AFTER: f64 = 0.2;
/// The daemon samples the level every 33ms, so each sample eases into the next.
const LEVEL_STEP: f64 = 0.033;
/// Antialiased glyph edges reach past the outlines' box.
const WORDS_BLEED: CGFloat = 1.0;

fn rect(x: CGFloat, y: CGFloat, width: CGFloat, height: CGFloat) -> CGRect {
    CGRect::new(CGPoint::new(x, y), CGSize::new(width, height))
}

fn color(hex: u32, alpha: CGFloat) -> CFRetained<CGColor> {
    let channel = |shift: u32| CGFloat::from(((hex >> shift) & 0xFF) as u8) / 255.0;
    CGColor::new_srgb(channel(16), channel(8), channel(0), alpha)
}

fn dashes(lengths: [CGFloat; 2]) -> Retained<NSArray<NSNumber>> {
    NSArray::from_retained_slice(&lengths.map(NSNumber::new_cgfloat))
}

fn shape(segments: &[Segment]) -> CFRetained<CGMutablePath> {
    let path = CGMutablePath::new();
    let to = Some(&*path);
    for segment in segments {
        match *segment {
            // SAFETY: a null transform means none.
            Segment::Move(x, y) => unsafe { CGMutablePath::move_to_point(to, ptr::null(), x, y) },
            // SAFETY: a null transform means none.
            Segment::Line(x, y) => unsafe {
                CGMutablePath::add_line_to_point(to, ptr::null(), x, y)
            },
            // SAFETY: a null transform means none.
            Segment::Cubic([x1, y1, x2, y2, x, y]) => unsafe {
                CGMutablePath::add_curve_to_point(to, ptr::null(), x1, y1, x2, y2, x, y)
            },
            Segment::Close => CGMutablePath::close_subpath(to),
        }
    }
    path
}

fn ellipses(
    centres: &[(CGFloat, CGFloat)],
    (rx, ry): (CGFloat, CGFloat),
) -> CFRetained<CGMutablePath> {
    let path = CGMutablePath::new();
    for &(cx, cy) in centres {
        let oval = rect(cx - rx, cy - ry, 2.0 * rx, 2.0 * ry);
        // SAFETY: a null transform means none.
        unsafe { CGMutablePath::add_ellipse_in_rect(Some(&path), ptr::null(), oval) };
    }
    path
}

/// Archivo from the window's own font file, at `WORDS_WEIGHT` and `WORDS_WIDTH`. `None` when
/// Core Text cannot read the file.
fn archivo() -> Option<CFRetained<CTFont>> {
    static ARCHIVO: &[u8] =
        include_bytes!("../../../../banshee-app/ui/public/fonts/archivo-latin.woff2");
    let data = CFData::from_static_bytes(ARCHIVO);
    // SAFETY: `data` is a live CFData.
    let found = unsafe { CTFontManagerCreateFontDescriptorsFromData(&data) };
    // SAFETY: the array Core Text returns holds font descriptors only.
    let found = unsafe { found.cast_unchecked::<CTFontDescriptor>() };
    let descriptor = found.get(0)?;
    let axis = |tag: &[u8; 4]| CFNumber::new_i64(i64::from(u32::from_be_bytes(*tag)));
    // SAFETY: each call copies a live descriptor with one axis value set.
    let descriptor = unsafe {
        descriptor
            .copy_with_variation(&axis(b"wght"), WORDS_WEIGHT)
            .copy_with_variation(&axis(b"wdth"), WORDS_WIDTH)
    };
    // SAFETY: a null matrix means none.
    Some(unsafe { CTFont::with_font_descriptor(&descriptor, WORDS_SIZE, ptr::null()) })
}

fn words_font() -> Option<CFRetained<CTFont>> {
    archivo().or_else(|| {
        eprintln!("banshee-tray: Archivo did not load; the chip's words use the system font");
        // SAFETY: no language means the user's own.
        unsafe {
            CTFont::new_ui_font_for_language(CTFontUIFontType::EmphasizedSystem, WORDS_SIZE, None)
        }
    })
}

/// The font and its tracking, as the one dictionary every line of words
/// carries. Built once: neither the font nor the tracking changes after that.
fn text_attributes_of(font: &CTFont) -> CFRetained<CFDictionary<CFString, CFType>> {
    let kern = CFNumber::new_f64(WORDS_TRACKING * WORDS_SIZE);
    // SAFETY: Core Text defines the attribute names as immutable constants.
    let keys = unsafe { [kCTFontAttributeName, kCTKernAttributeName] };
    let values: [&CFType; 2] = [font, &kern];
    CFDictionary::from_slices(&keys, &values)
}

/// One line of `text` under `attributes`. `None` where Core Text refuses to
/// build the attributed string.
fn line_of(text: &str, attributes: &CFDictionary<CFString, CFType>) -> Option<CFRetained<CTLine>> {
    let text = CFString::from_str(text);
    // SAFETY: `attributes` maps CFString keys to CFType values, the types the
    // font and the kern number hold.
    let string = unsafe { CFAttributedString::new(None, Some(&text), Some(attributes.as_ref())) }?;
    // SAFETY: `string` is a live attributed string.
    Some(unsafe { CTLine::with_attributed_string(&string) })
}

/// How far above the screen's bottom edge a hidden Dock's top reaches once it
/// shows. Nothing for a Dock at the side or one that stays: the visible frame
/// already leaves out a Dock that shows.
fn hidden_dock_height() -> CGFloat {
    let dock = CFString::from_static_str("com.apple.dock");
    CFPreferencesAppSynchronize(&dock);
    let key = CFString::from_static_str;
    let at_the_side = CFPreferencesCopyAppValue(&key("orientation"), &dock)
        .and_then(|value| value.downcast::<CFString>().ok())
        .is_some_and(|side| side.to_string() != "bottom");
    // SAFETY: a null pointer means the caller does not ask whether the key exists.
    let hides =
        unsafe { CFPreferencesGetAppBooleanValue(&key("autohide"), &dock, ptr::null_mut()) };
    if at_the_side || !hides {
        return 0.0;
    }
    let tile = CFPreferencesCopyAppValue(&key("tilesize"), &dock)
        .and_then(|value| value.downcast::<CFNumber>().ok())
        .and_then(|tile| tile.as_f64())
        .unwrap_or(DOCK_TILE_DEFAULT);
    tile + DOCK_ABOVE_TILE
}

/// Where the capsule goes on the screen that holds the focused window.
#[derive(Clone, Copy)]
struct Place {
    /// The screen's midline, at the capsule's bottom edge.
    anchor: CGPoint,
    /// The furthest right the capsule may reach. The capsule is centred, so
    /// this also bounds its width.
    right: CGFloat,
    scale: CGFloat,
}

fn placement(mtm: MainThreadMarker) -> Option<Place> {
    let screen = NSScreen::mainScreen(mtm)?;
    let frame = screen.frame();
    let dock_top = screen
        .visibleFrame()
        .origin
        .y
        .max(frame.origin.y + hidden_dock_height());
    Some(Place {
        anchor: CGPoint::new(
            frame.origin.x + frame.size.width / 2.0,
            dock_top + ABOVE_DOCK,
        ),
        right: frame.origin.x + frame.size.width - SCREEN_MARGIN,
        scale: screen.backingScaleFactor(),
    })
}

fn words_of(show: &Show) -> Option<&str> {
    match show {
        Show::YourTurn | Show::Answering => Some("Your turn"),
        Show::NothingHeard(text) | Show::Broken(text) => Some(text),
        Show::Recording | Show::Working | Show::Done(_) => None,
    }
}

/// In the stacking order, inside the capsule. Each figure layer nests in the
/// one above it, down to `voice`, which holds the shapes.
struct Layers {
    capsule: Retained<CALayer>,
    figure: Retained<CALayer>,
    exit: Retained<CALayer>,
    entry: Retained<CALayer>,
    gesture: Retained<CALayer>,
    voice: Retained<CALayer>,
    ring_far: Retained<CAShapeLayer>,
    light_far: Retained<CAShapeLayer>,
    gap: Retained<CAShapeLayer>,
    outline: Retained<CAShapeLayer>,
    fill: Retained<CAShapeLayer>,
    ring_near: Retained<CAShapeLayer>,
    light_near: Retained<CAShapeLayer>,
    headphones: Retained<CALayer>,
    cup_left: Retained<CALayer>,
    cup_right: Retained<CALayer>,
    words: Retained<CAShapeLayer>,
    words_mask: Retained<CALayer>,
    light_length: CGFloat,
}

impl Layers {
    fn build(root: &CALayer) -> Self {
        let rust = color(RUST, 1.0);
        let bone = color(BONE, 1.0);
        let ring_bone = color(BONE, RING_STRENGTH);

        let capsule = CALayer::new();
        capsule.setBackgroundColor(Some(&rust));
        capsule.setCornerRadius(CAPSULE / 2.0);
        capsule.setMasksToBounds(true);
        capsule.setAnchorPoint(CGPoint::new(0.5, 0.0));

        let figure = CALayer::new();
        figure.setBounds(rect(0.0, 0.0, 100.0, 100.0));
        figure.setPosition(CGPoint::new(INSET + FIGURE / 2.0, CAPSULE / 2.0));
        figure.setTransform(CATransform3D::new_scale(
            FIGURE / 100.0,
            FIGURE / 100.0,
            1.0,
        ));
        figure.setGeometryFlipped(true);

        let pivot = |layer: &CALayer| {
            layer.setAnchorPoint(CGPoint::new(PIVOT.0 / 100.0, PIVOT.1 / 100.0));
            layer.setPosition(CGPoint::new(PIVOT.0, PIVOT.1));
        };
        let square = || {
            let layer = CALayer::new();
            layer.setFrame(figure.bounds());
            layer
        };
        let (exit, entry, gesture, voice) = (square(), square(), square(), square());
        let headphones = square();
        pivot(&gesture);
        pivot(&voice);

        let unit = |fill: Option<&CGColor>| {
            let layer = CAShapeLayer::new();
            layer.setFrame(figure.bounds());
            layer.setFillColor(fill);
            layer
        };
        let stroke = |width: CGFloat, ink: &CGColor| {
            let layer = unit(None);
            layer.setStrokeColor(Some(ink));
            layer.setLineWidth(width);
            // SAFETY: Core Animation defines the join names as immutable constants.
            layer.setLineJoin(unsafe { kCALineJoinRound });
            layer
        };
        // A wide rust stroke shows as a gap once a narrower bone layer sits on top.
        let gap_front = |width: CGFloat, filled: bool, front: Retained<CAShapeLayer>| {
            let gap = stroke(width, &rust);
            if filled {
                gap.setFillColor(Some(&rust));
            }
            (gap, front)
        };
        let halo = ellipses(&[(HALO.0, HALO.1)], (HALO.2, HALO.3));
        let light_length = figure::ellipse_length(HALO.2, HALO.3);
        let ring = |near: bool, lit: bool| {
            let layer = if lit {
                stroke(LIGHT, &bone)
            } else {
                stroke(RING, &ring_bone)
            };
            layer.setPath(Some(&halo));
            if lit {
                // SAFETY: Core Animation defines the cap names as immutable constants.
                layer.setLineCap(unsafe { kCALineCapRound });
                layer.setLineDashPattern(Some(&dashes([0.22 * light_length, 0.78 * light_length])));
            }
            let half = if near {
                rect(-40.0, HALO.1, 180.0, 100.0)
            } else {
                rect(-40.0, -60.0, 180.0, 60.0 + HALO.1)
            };
            let mask = CALayer::new();
            mask.setFrame(half);
            mask.setBackgroundColor(Some(&bone));
            // SAFETY: the mask is new, sits in no layer tree, and masks this layer alone.
            unsafe { layer.setMask(Some(&mask)) };
            layer
        };
        let (gap, outline) = gap_front(BEHIND, true, stroke(OUTLINE, &bone));
        let fill = stroke(OUTLINE, &bone);
        fill.setFillColor(Some(&bone));
        pivot(&fill);
        let (band_gap, band) = gap_front(BAND_GAP, false, stroke(BAND, &bone));
        let headband = shape(&HEADBAND);
        for layer in [&band_gap, &band] {
            layer.setPath(Some(&headband));
            // SAFETY: Core Animation defines the cap names as immutable constants.
            layer.setLineCap(unsafe { kCALineCapRound });
            headphones.addSublayer(layer);
        }
        let cup = |centre: (CGFloat, CGFloat)| {
            let oval = ellipses(&[centre], CUP_RADII);
            let (cup_gap, cup) = gap_front(CUP_GAP, true, unit(Some(&bone)));
            let pair = square();
            for layer in [&cup_gap, &cup] {
                layer.setPath(Some(&oval));
                pair.addSublayer(layer);
            }
            headphones.addSublayer(&pair);
            pair
        };
        let words = unit(Some(&bone));
        words.setAnchorPoint(CGPoint::new(0.0, 0.0));
        let words_mask = CALayer::new();
        words_mask.setBackgroundColor(Some(&bone));
        words_mask.setAnchorPoint(CGPoint::ZERO);
        // SAFETY: the mask is new, sits in no layer tree, and masks the words alone.
        unsafe { words.setMask(Some(&words_mask)) };

        let layers = Layers {
            ring_far: ring(false, false),
            light_far: ring(false, true),
            gap,
            outline,
            fill,
            ring_near: ring(true, false),
            light_near: ring(true, true),
            cup_left: cup(CUPS[0]),
            cup_right: cup(CUPS[1]),
            headphones,
            words,
            words_mask,
            voice,
            gesture,
            entry,
            exit,
            figure,
            capsule,
            light_length,
        };
        root.addSublayer(&layers.capsule);
        layers.capsule.addSublayer(&layers.figure);
        layers.figure.addSublayer(&layers.exit);
        layers.exit.addSublayer(&layers.entry);
        layers.entry.addSublayer(&layers.gesture);
        layers.gesture.addSublayer(&layers.voice);
        for layer in [
            &layers.ring_far,
            &layers.light_far,
            &layers.gap,
            &layers.outline,
            &layers.fill,
            &layers.ring_near,
            &layers.light_near,
        ] {
            layers.voice.addSublayer(layer);
        }
        layers.voice.addSublayer(&layers.headphones);
        layers.capsule.addSublayer(&layers.words);
        layers
    }

    /// Every gesture a scene adds, so the next scene starts from the model.
    /// The capsule keeps its entrance and its width transition.
    fn quiet(&self) {
        let moving: [&CALayer; 9] = [
            &self.exit,
            &self.gesture,
            &self.fill,
            &self.outline,
            &self.light_far,
            &self.light_near,
            &self.cup_left,
            &self.cup_right,
            &self.words_mask,
        ];
        for layer in moving {
            layer.removeAllAnimations();
        }
        self.capsule.removeAnimationForKey(ns_string!("exit"));
    }
}

fn scale_tree(layer: &CALayer, scale: CGFloat) {
    layer.setContentsScale(scale);
    // SAFETY: the walk only reads the array, on the main thread, and nothing
    // changes the layer tree while it runs.
    if let Some(sublayers) = unsafe { layer.sublayers() } {
        for sublayer in sublayers.iter() {
            scale_tree(&sublayer, scale);
        }
    }
}

/// The panel holds the capsule, the clear space below it, and room on both
/// sides during a width transition. Any of it may take a click, so only a
/// point in the rounded shape of the centred capsule, `capsule` points wide,
/// counts.
fn in_capsule(view: CGSize, capsule: CGFloat, at: CGPoint) -> bool {
    let radius = CAPSULE / 2.0;
    let x = at.x - (view.width - capsule) / 2.0;
    let centre_x = x.clamp(radius, (capsule - radius).max(radius));
    (x - centre_x).hypot(at.y - TRAVEL - radius) <= radius
}

struct Clicks {
    open: Box<dyn Fn()>,
    capsule: Retained<CALayer>,
}

define_class!(
    // SAFETY: NSView has no subclassing rule this class breaks, and the class
    // does not implement Drop.
    #[unsafe(super(NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "BansheeChipView"]
    #[ivars = Clicks]
    struct ChipView;

    impl ChipView {
        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: Option<&NSEvent>) -> bool {
            true
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            let capsule = self.ivars().capsule.bounds().size.width;
            if in_capsule(self.bounds().size, capsule, event.locationInWindow()) {
                (self.ivars().open)();
            }
        }
    }
);

impl ChipView {
    fn new(
        mtm: MainThreadMarker,
        open: Box<dyn Fn()>,
        capsule: Retained<CALayer>,
    ) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Clicks { open, capsule });
        let frame = rect(0.0, 0.0, CAPSULE, TRAVEL + CAPSULE);
        // SAFETY: initWithFrame: is NSView's designated initialiser.
        unsafe { msg_send![super(this), initWithFrame: frame] }
    }
}

/// The words as glyph outlines, from the Core Text line that measures them. The
/// baseline sits at zero and y runs up.
struct Words {
    path: CFRetained<CGMutablePath>,
    ink: CGRect,
    width: CGFloat,
    ascent: CGFloat,
    descent: CGFloat,
}

/// What a scene plays. Each names one function in `motion`, or the fade that
/// stands in for it under Reduce Motion.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Move {
    Rise,
    FadeIn,
    Emerge,
    Kindle,
    KindleFade,
    Hover,
    Orbit,
    Tilt,
    Open,
    OpenFade,
    Cups,
    Shake,
    Strain,
    Breaks,
    Words,
    WordsFade,
    Float,
    FrameSink,
    FailureExit,
    FadeOut,
}

/// How a scene's moves join what showed before it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Entrance {
    /// The panel had nothing on screen.
    Fresh,
    /// A scene already on screen, and its words carry over unchanged.
    SameWords,
    /// A scene already on screen; this one starts its own words from nothing.
    NewWords,
}

/// The moves a scene plays as its serial moves. `calm` is Reduce Motion.
fn moves(show: &Show, entrance: Entrance, calm: bool) -> Vec<Move> {
    let mut moves = Vec::new();
    if entrance == Entrance::Fresh {
        moves.push(if calm { Move::FadeIn } else { Move::Rise });
        if !calm && matches!(show, Show::Recording) {
            moves.push(Move::Emerge);
        }
    }
    if entrance == Entrance::SameWords {
        moves.extend_from_slice(opening(calm));
        return moves;
    }
    moves.extend_from_slice(match (show, calm) {
        (Show::Recording, false) => &[Move::Kindle][..],
        (Show::Recording, true) => &[Move::KindleFade],
        (Show::Working, false) => &[Move::Hover, Move::Orbit],
        (Show::YourTurn, false) => &[Move::Cups],
        (Show::NothingHeard(_), false) => &[Move::Shake, Move::Breaks],
        (Show::Broken(_), false) => &[Move::Strain, Move::Breaks],
        (Show::Done(_), false) => &[Move::Float, Move::FrameSink],
        (Show::Done(_), true) => &[Move::FadeOut],
        (Show::Answering, calm) => opening(calm),
        (Show::Working | Show::YourTurn | Show::NothingHeard(_) | Show::Broken(_), true) => &[],
    });
    if words_of(show).is_some() {
        moves.push(if calm { Move::WordsFade } else { Move::Words });
    }
    moves
}

/// What the answer plays as it opens: the fill, and the head tilting in to
/// listen.
fn opening(calm: bool) -> &'static [Move] {
    if calm {
        &[Move::OpenFade]
    } else {
        &[Move::Open, Move::Tilt]
    }
}

/// Your turn becomes the answer in place: the capsule and the words stay.
fn answer_opens(before: Option<&Show>, after: &Show) -> bool {
    matches!((before, after), (Some(Show::YourTurn), Show::Answering))
}

/// The states whose figure fills and moves with the microphone level.
fn holds_the_voice(show: &Show) -> bool {
    matches!(show, Show::Recording | Show::Answering)
}

fn centred(midline: CGFloat, width: CGFloat) -> CGFloat {
    midline - width / 2.0
}

/// How a scene leaves when none follows it. `None` for Done: its float has
/// already carried it out.
fn leaving(show: &Show, calm: bool) -> Option<Move> {
    match show {
        Show::Done(_) => None,
        _ => Some(if calm {
            Move::FadeOut
        } else {
            Move::FailureExit
        }),
    }
}

pub struct Panel {
    mtm: MainThreadMarker,
    window: Retained<NSPanel>,
    root: Retained<CALayer>,
    layers: Layers,
    font: Option<CFRetained<CTFont>>,
    text_attributes: Option<CFRetained<CFDictionary<CFString, CFType>>>,
    ellipsis: Option<CFRetained<CTLine>>,
    outline_dash: Retained<NSArray<NSNumber>>,
    outline_length: CGFloat,
    place: Option<Place>,
    on_screen: Option<Scene>,
    calm: bool,
    voice: Voice,
    voice_since: Instant,
    /// Moves on every change, so a completion that a later change overtook
    /// does nothing.
    ticket: Rc<Cell<u64>>,
}

impl Panel {
    /// `open` runs on a click on the capsule while a broken state shows.
    pub fn new(mtm: MainThreadMarker, open: Box<dyn Fn()>) -> Self {
        let window = NSPanel::initWithContentRect_styleMask_backing_defer(
            NSPanel::alloc(mtm),
            rect(0.0, 0.0, CAPSULE, TRAVEL + CAPSULE),
            NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel,
            NSBackingStoreType::Buffered,
            false,
        );
        window.setHidesOnDeactivate(false);
        window.setOpaque(false);
        window.setBackgroundColor(Some(&NSColor::clearColor()));
        window.setHasShadow(false);
        // SAFETY: false leaves the panel's lifetime to its `Retained`, the only owner.
        unsafe { window.setReleasedWhenClosed(false) };
        window.setCollectionBehavior(
            NSWindowCollectionBehavior::CanJoinAllSpaces
                | NSWindowCollectionBehavior::FullScreenAuxiliary
                | NSWindowCollectionBehavior::Stationary
                | NSWindowCollectionBehavior::IgnoresCycle,
        );
        window.setLevel(NSStatusWindowLevel);
        window.setIgnoresMouseEvents(true);

        let root = CALayer::new();
        let layers = Layers::build(&root);
        let view = ChipView::new(mtm, open, layers.capsule.clone());
        view.setLayer(Some(&root));
        view.setWantsLayer(true);
        window.setContentView(Some(&view));
        // AppKit reads a borderless, status-level panel as a system dialog to
        // VoiceOver. The panel holds no text or controls, so the window and its
        // view stay hidden. VoiceOver must not speak while an agent's question
        // keeps the microphone open.
        window.setAccessibilityElement(false);
        window.setAccessibilityHidden(true);
        view.setAccessibilityElement(false);
        view.setAccessibilityHidden(true);

        let rest = figure::path_length(&figure::shroud(0.0, 0.0));
        let font = words_font();
        let text_attributes = font.as_deref().map(text_attributes_of);
        let ellipsis = text_attributes
            .as_deref()
            .and_then(|attributes| line_of("\u{2026}", attributes));
        Panel {
            mtm,
            window,
            layers,
            root,
            font,
            text_attributes,
            ellipsis,
            outline_dash: dashes([0.088 * rest, 0.056 * rest]),
            outline_length: rest,
            place: None,
            on_screen: None,
            calm: false,
            voice: Voice::default(),
            voice_since: Instant::now(),
            ticket: Rc::new(Cell::new(0)),
        }
    }

    pub fn show(&mut self, scene: Option<&Scene>) {
        if self.on_screen.as_ref() == scene {
            return;
        }
        self.ticket.set(self.ticket.get() + 1);
        let Some(scene) = scene else {
            self.leave();
            return;
        };
        let entering = !self.window.isVisible();
        let opens = answer_opens(
            self.on_screen.as_ref().map(|shown| &shown.show),
            &scene.show,
        );
        if entering && let Some(place) = placement(self.mtm) {
            self.place = Some(place);
            scale_tree(&self.layers.capsule, place.scale);
        }
        let Some(place) = self.place else {
            return;
        };
        let workspace = NSWorkspace::sharedWorkspace();
        let contrast = workspace.accessibilityDisplayShouldIncreaseContrast();
        self.calm = workspace.accessibilityDisplayShouldReduceMotion();
        self.voice = Voice::default();
        self.voice_since = Instant::now();
        CATransaction::begin();
        CATransaction::setDisableActions(true);
        self.layers.quiet();
        self.draw(&scene.show, place, contrast, entering);
        let entrance = if opens {
            Entrance::SameWords
        } else if entering {
            Entrance::Fresh
        } else {
            Entrance::NewWords
        };
        for step in moves(&scene.show, entrance, self.calm) {
            self.play(step);
        }
        CATransaction::commit();
        self.window
            .setIgnoresMouseEvents(!matches!(scene.show, Show::Broken(_)));
        self.window.orderFrontRegardless();
        self.on_screen = Some(scene.clone());
    }

    /// Moves the figure with the microphone while it hears you.
    pub fn level(&mut self, peak: f32) {
        if !self
            .on_screen
            .as_ref()
            .is_some_and(|scene| holds_the_voice(&scene.show))
        {
            return;
        }
        let level = self.voice.hear(peak, Instant::now());
        let t_ms = self.voice_since.elapsed().as_secs_f64() * 1000.0;
        CATransaction::begin();
        CATransaction::setAnimationDuration(LEVEL_STEP);
        let pose = figure::pose(level, t_ms, self.calm);
        // Reduce Motion holds the shroud where `draw` struck it.
        if self.calm {
            self.fill_to(pose.fill);
        } else {
            self.strike(pose);
        }
        CATransaction::commit();
    }

    pub fn announce(&self, words: &str) {
        let words = NSString::from_str(words);
        let priority = NSNumber::new_isize(NSAccessibilityPriorityLevel::High.0);
        // SAFETY: AppKit defines both keys as immutable constants.
        let keys = unsafe { [NSAccessibilityAnnouncementKey, NSAccessibilityPriorityKey] };
        let values: [&AnyObject; 2] = [words.as_ref(), priority.as_ref()];
        let info = NSDictionary::from_slices(&keys, &values);
        let app = NSApplication::sharedApplication(self.mtm);
        // SAFETY: AppKit defines the notification name as an immutable constant. The
        // info holds a string under the announcement key and a number under the
        // priority key, the types each key names.
        unsafe {
            NSAccessibilityPostNotificationWithUserInfo(
                app.as_ref(),
                NSAccessibilityAnnouncementRequestedNotification,
                Some(&info),
            )
        };
    }

    fn leave(&mut self) {
        let Some(was) = self.on_screen.take() else {
            return;
        };
        self.window.setIgnoresMouseEvents(true);
        let Some(step) = leaving(&was.show, self.calm) else {
            self.window.orderOut(None);
            return;
        };
        CATransaction::begin();
        let window = self.window.clone();
        self.after(move || window.orderOut(None));
        self.play(step);
        CATransaction::commit();
    }

    /// Runs `then` once the current transaction's animations end, unless a
    /// later change came first.
    fn after(&self, then: impl Fn() + 'static) {
        let ticket = Rc::clone(&self.ticket);
        let issued = ticket.get();
        let block = RcBlock::new(move || {
            if ticket.get() == issued {
                then();
            }
        });
        // SAFETY: the block takes and returns nothing, as the completion's type
        // says, and Core Animation runs it on the main thread, where the
        // transaction began.
        unsafe { CATransaction::setCompletionBlock(Some(&block)) };
    }

    fn play(&self, step: Move) {
        let layers = &self.layers;
        let key = match step {
            Move::Rise | Move::FadeIn | Move::Emerge => ns_string!("enter"),
            Move::FrameSink | Move::FailureExit | Move::FadeOut => ns_string!("exit"),
            _ => ns_string!("move"),
        };
        let add = |layer: &CALayer, animation: Retained<CAAnimation>| {
            layer.addAnimation_forKey(&animation, Some(key));
        };
        match step {
            Move::Rise => add(&layers.capsule, motion::chip_rise()),
            Move::FadeIn => add(&layers.capsule, motion::fade_in(0.0)),
            Move::Emerge => add(&layers.entry, motion::figure_emerge()),
            Move::Kindle => add(&layers.fill, motion::fill_kindle(KINDLE_AFTER)),
            Move::KindleFade => add(&layers.fill, motion::fade_in(KINDLE_AFTER)),
            Move::Hover => add(&layers.gesture, motion::working_hover()),
            Move::Orbit => {
                let orbit = motion::halo_light(layers.light_length);
                for light in [&layers.light_far, &layers.light_near] {
                    add(light, orbit.clone());
                }
            }
            Move::Tilt => add(&layers.gesture, motion::head_tilt()),
            Move::Open => add(&layers.fill, motion::fill_kindle(0.0)),
            Move::OpenFade => add(&layers.fill, motion::fade_in(0.0)),
            Move::Cups => {
                add(&layers.cup_left, motion::cups_press(1.0));
                add(&layers.cup_right, motion::cups_press(-1.0));
            }
            Move::Shake => add(&layers.gesture, motion::head_shake()),
            Move::Strain => add(&layers.gesture, motion::strain()),
            Move::Breaks => add(
                &layers.outline,
                motion::outline_breaks(self.outline_length, &self.outline_dash),
            ),
            Move::Words => add(
                &layers.words_mask,
                motion::words_reveal(layers.words_mask.bounds().size.width),
            ),
            Move::WordsFade => add(&layers.words_mask, motion::fade_in(0.0)),
            Move::Float => add(&layers.exit, motion::float_out()),
            Move::FrameSink => add(&layers.capsule, motion::frame_sink()),
            Move::FailureExit => add(&layers.capsule, motion::failure_exit()),
            Move::FadeOut => add(&layers.capsule, motion::fade_out()),
        }
    }

    /// Sets the hem, the lift and scale, and the fill's strength.
    fn strike(&self, pose: Pose) {
        let layers = &self.layers;
        let body = shape(&figure::shroud(pose.depth, pose.sway));
        for layer in [&layers.gap, &layers.outline, &layers.fill] {
            layer.setPath(Some(&body));
        }
        layers.voice.setTransform(
            CATransform3D::new_translation(0.0, pose.lift, 0.0).scale(pose.scale, pose.scale, 1.0),
        );
        self.fill_to(pose.fill);
    }

    fn fill_to(&self, strength: f64) {
        let strength = color(BONE, strength);
        let fill = &self.layers.fill;
        fill.setFillColor(Some(&strength));
        fill.setStrokeColor(Some(&strength));
    }

    /// The capsule's width as it shows now, part way through a transition or not.
    fn shown_width(&self) -> CGFloat {
        let capsule = &self.layers.capsule;
        // SAFETY: the presentation copy is read once, on the main thread, and not kept.
        let shown = unsafe { capsule.presentationLayer() };
        shown
            .map_or_else(|| capsule.bounds(), |shown| shown.bounds())
            .size
            .width
    }

    fn draw(&self, show: &Show, place: Place, contrast: bool, entering: bool) {
        let layers = &self.layers;
        let working = matches!(show, Show::Working);
        let failed = matches!(show, Show::NothingHeard(_) | Show::Broken(_));

        let room = 2.0 * (place.right - place.anchor.x) - (INSET + FIGURE + WORDS_GAP + WORDS_TAIL);
        let words = words_of(show).and_then(|words| self.words(words, room));
        let width = words.as_ref().map_or(CAPSULE, |words| {
            INSET + FIGURE + WORDS_GAP + words.width + WORDS_TAIL
        });
        let from = if entering || self.calm {
            width
        } else {
            self.shown_width()
        };
        let reach = width.max(from);
        let (window, root, capsule) = (&self.window, &self.root, &layers.capsule);
        fit(window, root, capsule, place, reach);
        capsule.setBounds(rect(0.0, 0.0, width, CAPSULE));
        if from != width {
            layers
                .capsule
                .addAnimation_forKey(&motion::chip_width(from, width), Some(ns_string!("width")));
        }

        let rest = figure::pose(0.0, 0.0, self.calm);
        let depth = if working && !self.calm {
            WORKING_DEPTH
        } else {
            rest.depth
        };
        self.strike(Pose { depth, ..rest });
        let outline = if contrast { OUTLINE_CONTRAST } else { OUTLINE };
        for layer in [&layers.outline, &layers.fill] {
            layer.setLineWidth(outline);
        }
        layers
            .outline
            .setLineDashPattern(failed.then_some(&*self.outline_dash));

        for layer in [
            &layers.ring_far,
            &layers.light_far,
            &layers.gap,
            &layers.ring_near,
            &layers.light_near,
        ] {
            layer.setHidden(!working);
        }
        layers.fill.setHidden(!holds_the_voice(show));
        layers
            .headphones
            .setHidden(!matches!(show, Show::YourTurn | Show::Answering));

        if let Some(words) = &words {
            let line = words.ascent + words.descent;
            let baseline = (CAPSULE - line) / 2.0 + words.descent;
            layers.words.setPath(Some(&words.path));
            layers.words.setBounds(words.ink);
            layers.words.setPosition(CGPoint::new(
                INSET + FIGURE + WORDS_GAP + words.ink.origin.x,
                baseline + words.ink.origin.y,
            ));
            layers.words_mask.setBounds(rect(
                0.0,
                0.0,
                words.ink.size.width + 2.0 * WORDS_BLEED,
                words.ink.size.height + 2.0 * WORDS_BLEED,
            ));
            layers.words_mask.setPosition(CGPoint::new(
                words.ink.origin.x - WORDS_BLEED,
                words.ink.origin.y - WORDS_BLEED,
            ));
        }
        layers.words.setHidden(words.is_none());
    }

    /// Lays the words out in one Core Text line, cut with a trailing ellipsis
    /// past `room` points, and takes each glyph's outline. A glyph Archivo
    /// lacks comes from the font Core Text falls back to.
    fn words(&self, words: &str, room: CGFloat) -> Option<Words> {
        let font = self.font.as_deref()?;
        let attributes = self.text_attributes.as_deref()?;
        let whole = line_of(words, attributes)?;
        // SAFETY: `whole` is live, and the ellipsis line, when present, is live too.
        let line = unsafe {
            whole.truncated_line(room, CTLineTruncationType::End, self.ellipsis.as_deref())
        }
        .unwrap_or(whole);
        let (mut ascent, mut descent) = (0.0, 0.0);
        // SAFETY: both pointers are to live locals, and a null leading is allowed.
        let width = unsafe { line.typographic_bounds(&mut ascent, &mut descent, ptr::null_mut()) };

        let path = CGMutablePath::new();
        // SAFETY: a line's runs array holds runs only.
        let runs = unsafe { line.glyph_runs() };
        // SAFETY: as above.
        for run in unsafe { runs.cast_unchecked::<CTRun>() }.iter() {
            // SAFETY: `run` is a live run.
            let count = usize::try_from(unsafe { run.glyph_count() }).unwrap_or(0);
            if count == 0 {
                continue;
            }
            let mut glyphs: Vec<CGGlyph> = vec![0; count];
            let mut positions = vec![CGPoint::ZERO; count];
            let all = CFRange {
                location: 0,
                length: 0,
            };
            // SAFETY: each buffer holds `count` elements, the run's glyph count, and a
            // zero-length range asks for the whole run.
            unsafe {
                run.glyphs(all, ptr::NonNull::from(&mut glyphs[..]).cast());
                run.positions(all, ptr::NonNull::from(&mut positions[..]).cast());
            }
            // SAFETY: a run's attributes map CFString names to CFType values.
            let attributes = unsafe { run.attributes() };
            // SAFETY: as above.
            let attributes = unsafe { attributes.cast_unchecked::<CFString, CFType>() };
            // SAFETY: Core Text defines the attribute name as an immutable constant.
            let run_font = attributes
                .get(unsafe { kCTFontAttributeName })
                .and_then(|value| value.downcast::<CTFont>().ok());
            let run_font = run_font.as_deref().unwrap_or(font);
            for (glyph, at) in glyphs.into_iter().zip(positions) {
                let place = CGAffineTransform {
                    a: 1.0,
                    b: 0.0,
                    c: 0.0,
                    d: 1.0,
                    tx: at.x,
                    ty: at.y,
                };
                // SAFETY: `place` is a live transform, and the glyph came from this font.
                if let Some(outline) = unsafe { run_font.path_for_glyph(glyph, &place) } {
                    // SAFETY: a null transform means none.
                    unsafe { CGMutablePath::add_path(Some(&path), ptr::null(), Some(&outline)) };
                }
            }
        }
        let ink = ink_of(&path)?;
        Some(Words {
            width: width.max(ink.origin.x + ink.size.width).ceil(),
            path,
            ink,
            ascent,
            descent,
        })
    }
}

/// Sizes the panel `span` points wide on the midline, with the clear space
/// below, and centres the capsule in it.
fn fit(window: &NSPanel, root: &CALayer, capsule: &CALayer, place: Place, span: CGFloat) {
    let left = centred(place.anchor.x, span);
    window.setFrame_display(
        rect(left, place.anchor.y - TRAVEL, span, TRAVEL + CAPSULE),
        true,
    );
    root.setFrame(rect(0.0, 0.0, span, TRAVEL + CAPSULE));
    capsule.setPosition(CGPoint::new(span / 2.0, TRAVEL));
}

/// `None` for a path with no ink. Core Graphics boxes one as the null rect,
/// whose origin is infinite.
fn ink_of(path: &CGPath) -> Option<CGRect> {
    Some(CGPath::bounding_box(Some(path))).filter(|ink| !ink.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chip::Done;

    #[test]
    fn a_path_with_no_ink_is_no_words() {
        assert_eq!(ink_of(&CGMutablePath::new()), None);
    }

    #[test]
    fn a_path_with_ink_is_boxed() {
        let path = CGMutablePath::new();
        let oval = rect(2.0, 3.0, 10.0, 4.0);
        // SAFETY: a null transform means none.
        unsafe { CGMutablePath::add_ellipse_in_rect(Some(&path), ptr::null(), oval) };
        assert_eq!(ink_of(&path), Some(oval));
    }

    #[test]
    fn a_click_lands_only_inside_the_capsules_rounded_shape() {
        let view = CGSize::new(100.0, 44.0 + TRAVEL);
        let at = |x, y| CGPoint::new(x, TRAVEL + y);
        for (x, y) in [(2.0, 22.0), (98.0, 22.0), (50.0, 22.0)] {
            assert!(in_capsule(view, 100.0, at(x, y)), "({x}, {y})");
        }
        for (x, y) in [(0.0, 0.0), (100.0, 0.0), (0.0, 44.0), (100.0, 44.0)] {
            assert!(!in_capsule(view, 100.0, at(x, y)), "({x}, {y})");
        }
    }

    #[test]
    fn a_click_in_the_clear_space_below_the_capsule_misses() {
        let view = CGSize::new(100.0, 44.0 + TRAVEL);
        for y in [0.0, TRAVEL / 2.0, TRAVEL - 1.0] {
            assert!(!in_capsule(view, 100.0, CGPoint::new(50.0, y)), "y {y}");
        }
    }

    #[test]
    fn a_click_beside_a_narrower_centred_capsule_misses() {
        let view = CGSize::new(140.0, 44.0 + TRAVEL);
        let at = |x| CGPoint::new(x, TRAVEL + 22.0);
        for x in [2.0, 18.0, 122.0, 138.0] {
            assert!(!in_capsule(view, 100.0, at(x)), "x {x}");
        }
        for x in [22.0, 70.0, 118.0] {
            assert!(in_capsule(view, 100.0, at(x)), "x {x}");
        }
    }

    fn failed() -> [Show; 2] {
        [
            Show::NothingHeard("Nothing heard".into()),
            Show::Broken("Microphone blocked".into()),
        ]
    }

    fn every_show() -> Vec<Show> {
        let [nothing_heard, broken] = failed();
        vec![
            Show::Recording,
            Show::Working,
            Show::YourTurn,
            Show::Answering,
            nothing_heard,
            broken,
            Show::Done(Done::Typed),
        ]
    }

    #[test]
    fn only_a_scene_from_none_rises_and_only_recording_emerges() {
        for show in every_show() {
            let entering = moves(&show, Entrance::Fresh, false);
            let staying = moves(&show, Entrance::NewWords, false);
            assert_eq!(entering.first(), Some(&Move::Rise), "{show:?}");
            assert!(!staying.contains(&Move::Rise), "{show:?}");
            assert_eq!(
                entering.contains(&Move::Emerge),
                show == Show::Recording,
                "{show:?}"
            );
            assert!(!staying.contains(&Move::Emerge), "{show:?}");
        }
    }

    #[test]
    fn each_state_plays_its_own_gesture() {
        use Move::*;
        assert_eq!(
            moves(&Show::YourTurn, Entrance::NewWords, false),
            [Cups, Words]
        );
        assert_eq!(
            moves(&Show::Answering, Entrance::NewWords, false),
            [Open, Tilt, Words]
        );
        let [nothing_heard, broken] = failed();
        assert_eq!(
            moves(&nothing_heard, Entrance::NewWords, false),
            [Shake, Breaks, Words]
        );
        assert_eq!(
            moves(&broken, Entrance::NewWords, false),
            [Strain, Breaks, Words]
        );
        let done = Show::Done(Done::Typed);
        assert_eq!(moves(&done, Entrance::NewWords, false), [Float, FrameSink]);
    }

    #[test]
    fn the_answer_tilts_the_head_in_as_it_fills_and_calm_only_fills() {
        use Move::*;
        assert_eq!(opening(false), [Open, Tilt]);
        assert_eq!(opening(true), [OpenFade]);
    }

    #[test]
    fn answering_in_place_only_opens_and_drops_the_words_reveal() {
        use Move::*;
        assert_eq!(
            moves(&Show::Answering, Entrance::SameWords, false),
            [Open, Tilt]
        );
        assert_eq!(
            moves(&Show::Answering, Entrance::SameWords, true),
            [OpenFade]
        );
    }

    #[test]
    fn only_your_turn_opens_into_the_answer() {
        assert!(answer_opens(Some(&Show::YourTurn), &Show::Answering));
        for (before, after) in [
            (None, Show::Answering),
            (Some(Show::Working), Show::Answering),
            (Some(Show::Recording), Show::Answering),
            (Some(Show::Answering), Show::Answering),
            (Some(Show::YourTurn), Show::YourTurn),
            (Some(Show::YourTurn), Show::Recording),
        ] {
            assert!(
                !answer_opens(before.as_ref(), &after),
                "{before:?} to {after:?}"
            );
        }
    }

    #[test]
    fn only_a_recording_and_an_answer_hold_the_voice() {
        for show in every_show() {
            assert_eq!(
                holds_the_voice(&show),
                matches!(show, Show::Recording | Show::Answering),
                "{show:?}"
            );
        }
    }

    #[test]
    fn the_capsule_is_centred_on_the_midline() {
        assert_eq!(centred(500.0, 44.0), 478.0);
        assert_eq!(centred(500.0, 120.0), 440.0);
    }

    #[test]
    fn reduce_motion_keeps_only_the_fades() {
        use Move::*;
        assert_eq!(
            moves(&Show::Recording, Entrance::Fresh, true),
            [FadeIn, KindleFade]
        );
        assert_eq!(moves(&Show::Working, Entrance::NewWords, true), []);
        assert_eq!(
            moves(&Show::YourTurn, Entrance::Fresh, true),
            [FadeIn, WordsFade]
        );
        assert_eq!(
            moves(&Show::Answering, Entrance::NewWords, true),
            [OpenFade, WordsFade]
        );
        for fault in failed() {
            assert_eq!(moves(&fault, Entrance::NewWords, true), [WordsFade]);
        }
        assert_eq!(
            moves(&Show::Done(Done::Sent), Entrance::NewWords, true),
            [FadeOut]
        );
    }

    #[test]
    fn done_leaves_by_its_float_and_everything_else_sinks() {
        assert_eq!(leaving(&Show::Done(Done::Finished), false), None);
        for show in [
            Show::Recording,
            Show::Working,
            Show::YourTurn,
            Show::Answering,
        ]
        .into_iter()
        .chain(failed())
        {
            assert_eq!(leaving(&show, false), Some(Move::FailureExit), "{show:?}");
            assert_eq!(leaving(&show, true), Some(Move::FadeOut), "{show:?}");
        }
    }
}
