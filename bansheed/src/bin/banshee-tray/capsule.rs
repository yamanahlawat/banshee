//! The capsule on a Wayland desktop: a layer surface on its own thread, each
//! frame painted by tiny-skia into shared memory.

use std::error::Error;
use std::ffi::OsString;
use std::fmt;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use smithay_client_toolkit::compositor::{
    CompositorHandler, CompositorState, FrameCallbackData, Region,
};
use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::reexports::calloop::{self, EventLoop, channel};
use smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource;
use smithay_client_toolkit::reexports::client::globals::registry_queue_init;
use smithay_client_toolkit::reexports::client::protocol::{wl_output, wl_shm, wl_surface};
use smithay_client_toolkit::reexports::client::{Connection, EventQueue, QueueHandle};
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shell::wlr_layer::{
    Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
    LayerSurfaceConfigure,
};
use smithay_client_toolkit::shm::slot::SlotPool;
use smithay_client_toolkit::shm::{Shm, ShmHandler};
use smithay_client_toolkit::{delegate_dispatch2, delegate_registry, registry_handlers};
use tiny_skia::Pixmap;

use crate::chip::{Scene, Show, words_of};
use crate::figure::{self, CAPSULE, FIGURE, INSET, Pose, TRAVEL, Voice, WORDS_GAP, WORDS_TAIL};
use crate::gesture;
use crate::paint::{self, Frame};
use crate::scenes::{Entrance, Move, answer_opens, holds_the_voice, leaving, moves};
use crate::words::{self, Line};

const BOTTOM_MARGIN: i32 = 12;
const SIDE_MARGIN: f64 = 12.0;

enum Command {
    Show(Option<Scene>),
    Level(f32),
}

pub struct Capsule {
    commands: channel::Sender<Command>,
}

impl Capsule {
    /// `None` when the desktop has no layer shell to draw on. `lost` runs on
    /// the capsule's thread once the connection ends.
    pub fn open(lost: impl FnOnce() + Send + 'static) -> Option<Self> {
        Self::open_on(std::env::var_os("WAYLAND_DISPLAY"), lost)
    }

    fn open_on(display: Option<OsString>, lost: impl FnOnce() + Send + 'static) -> Option<Self> {
        let (reply, opened) = mpsc::sync_channel(0);
        let (commands, received) = channel::channel();
        let spawned = std::thread::Builder::new()
            .name("banshee-capsule".into())
            .spawn(move || match connect(display) {
                Err(failure) => {
                    let _ = reply.send(Err(failure));
                }
                Ok((stage, connection, queue)) => {
                    let _ = reply.send(Ok(()));
                    if let Err(error) = stage.run(connection, queue, received) {
                        log::error!("the capsule's Wayland connection ended: {error}");
                        lost();
                    }
                }
            });
        if let Err(error) = spawned {
            log::error!("the capsule's thread did not start: {error}");
            return None;
        }
        match opened.recv() {
            Ok(Ok(())) => Some(Capsule { commands }),
            Ok(Err(failure)) => {
                log::warn!("the capsule draws nothing: {failure}");
                None
            }
            Err(_) => {
                log::error!("the capsule's thread ended before it connected");
                None
            }
        }
    }

    // The thread ends only when the connection does, and `lost` then ends the
    // tray, so a send that fails has nowhere left to go.
    pub fn show(&self, scene: Option<&Scene>) {
        let _ = self.commands.send(Command::Show(scene.cloned()));
    }

    pub fn level(&self, peak: f32) {
        let _ = self.commands.send(Command::Level(peak));
    }
}

#[derive(Debug)]
enum Failure {
    NoDisplay,
    Connect(String),
    NoLayerShell,
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Failure::NoDisplay => f.write_str("WAYLAND_DISPLAY is unset"),
            Failure::Connect(error) => write!(f, "the Wayland connection failed: {error}"),
            Failure::NoLayerShell => f.write_str("the compositor offers no layer shell"),
        }
    }
}

fn connect(display: Option<OsString>) -> Result<(Stage, Connection, EventQueue<Stage>), Failure> {
    if display.is_none() {
        return Err(Failure::NoDisplay);
    }
    let failed = |error: &dyn fmt::Display| Failure::Connect(error.to_string());
    let connection = Connection::connect_to_env().map_err(|error| failed(&error))?;
    let (globals, queue) = registry_queue_init(&connection).map_err(|error| failed(&error))?;
    let qh = queue.handle();
    let compositor = CompositorState::bind(&globals, &qh).map_err(|error| failed(&error))?;
    let shm = Shm::bind(&globals, &qh).map_err(|error| failed(&error))?;
    let layer_shell = LayerShell::bind(&globals, &qh).map_err(|_| Failure::NoLayerShell)?;
    let bare = (CAPSULE * (TRAVEL + CAPSULE)) as usize * 4;
    let pool = SlotPool::new(bare, &shm).map_err(|error| failed(&error))?;
    let stage = Stage {
        registry: RegistryState::new(&globals),
        outputs: OutputState::new(&globals, &qh),
        compositor,
        shm,
        layer_shell,
        pool,
        qh,
        renderer: Renderer::new(),
        surface: None,
        entered: None,
        scale: 1,
        owed: false,
        done: false,
    };
    Ok((stage, connection, queue))
}

/// What the surface does before the next frame.
#[derive(Debug, PartialEq, Eq)]
enum Order {
    Create,
    Destroy,
}

/// The scene logic the macOS panel keeps in its layers, with time passed in.
struct Renderer {
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
    pose: Pose,
    /// The picture changed since the last frame.
    stale: bool,
    /// Logical pixels the words may take.
    room: f64,
}

impl Renderer {
    fn new() -> Self {
        Renderer {
            scene: None,
            moves: Vec::new(),
            leave_ends: None,
            line: None,
            width_from: None,
            voice: Voice::default(),
            voice_since: None,
            pose: figure::pose(0.0, 0.0, false),
            stale: false,
            room: f64::INFINITY,
        }
    }

    fn show(&mut self, scene: Option<&Scene>, now: Instant) -> Vec<Order> {
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
    fn tick(&mut self, now: Instant) -> Vec<Order> {
        if self.deadline().is_some_and(|ends| now >= ends) {
            self.clear();
            return vec![Order::Destroy];
        }
        Vec::new()
    }

    fn deadline(&self) -> Option<Instant> {
        self.leave_ends
    }

    fn level(&mut self, peak: f32, now: Instant) {
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
        self.pose = figure::pose(level, t_ms, false);
        self.stale = true;
    }

    /// Whether frames still change with nothing new coming in.
    fn live(&self, now: Instant) -> bool {
        let moving = self.moves.iter().any(|&(step, started)| {
            paint::lasts(step).is_none_or(|lasts| seconds(started, now) < lasts)
        });
        moving || self.transition(now).is_some()
    }

    /// The width transition's from-width and seconds so far, while it runs.
    fn transition(&self, now: Instant) -> Option<(f64, f64)> {
        let (from, started) = self.width_from?;
        let elapsed = seconds(started, now);
        let lasts = gesture::lasts(&gesture::chip_width(from, from))?;
        (elapsed < lasts).then_some((from, elapsed))
    }

    /// Hands `use_frame` the frame at `now`. `None` with nothing on the surface.
    fn picture<R>(
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
            pose: self.pose,
            line: self.line.as_ref(),
            width_from: self.transition(now),
            scale,
        }))
    }

    fn clear(&mut self) {
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
fn room_on(width: i32) -> f64 {
    f64::from(width) - 2.0 * SIDE_MARGIN - (INSET + FIGURE + WORDS_GAP + WORDS_TAIL)
}

/// tiny-skia's premultiplied RGBA as `wl_shm` ARGB8888, a premultiplied
/// little-endian 0xAARRGGBB, so B, G, R, A in memory.
fn to_argb(rgba: &[u8], argb: &mut [u8]) {
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

/// The layer surface while it shows.
struct Shown {
    layer: LayerSurface,
    /// The logical size last asked for.
    asked: (u32, u32),
    /// The compositor configured the surface since the last ask, so a buffer may follow.
    ready: bool,
    /// A frame callback is pending.
    waiting: bool,
}

/// The capsule thread's state, which every Wayland event reaches.
struct Stage {
    registry: RegistryState,
    compositor: CompositorState,
    outputs: OutputState,
    shm: Shm,
    layer_shell: LayerShell,
    pool: SlotPool,
    qh: QueueHandle<Stage>,
    renderer: Renderer,
    surface: Option<Shown>,
    /// The output the last surface entered.
    entered: Option<wl_output::WlOutput>,
    scale: u32,
    /// A configure, a scale change or a frame callback asks for a frame.
    owed: bool,
    done: bool,
}

impl Stage {
    fn run(
        mut self,
        connection: Connection,
        queue: EventQueue<Stage>,
        commands: channel::Channel<Command>,
    ) -> Result<(), calloop::Error> {
        let mut event_loop = EventLoop::<Stage>::try_new()?;
        let handle = event_loop.handle();
        WaylandSource::new(connection, queue)
            .insert(handle.clone())
            .map_err(|error| error.error)?;
        handle
            .insert_source(commands, |event, (), stage| match event {
                channel::Event::Msg(command) => stage.take(command, Instant::now()),
                channel::Event::Closed => stage.done = true,
            })
            .map_err(|error| error.error)?;
        while !self.done {
            let wait = self
                .renderer
                .deadline()
                .map(|at| at.saturating_duration_since(Instant::now()));
            event_loop.dispatch(wait, &mut self)?;
            self.refresh(Instant::now());
        }
        Ok(())
    }

    fn take(&mut self, command: Command, now: Instant) {
        match command {
            Command::Show(scene) => {
                if let Some(width) = self.output_width() {
                    self.renderer.room = room_on(width);
                }
                let orders = self.renderer.show(scene.as_ref(), now);
                self.carry_out(orders, now);
            }
            Command::Level(peak) => self.renderer.level(peak, now),
        }
    }

    fn carry_out(&mut self, orders: Vec<Order>, now: Instant) {
        for order in orders {
            self.surface = match order {
                Order::Destroy => None,
                Order::Create => self.create(now),
            };
        }
    }

    fn create(&mut self, now: Instant) -> Option<Shown> {
        let asked = self.renderer.picture(now, 1, paint::size)?;
        self.scale = self.largest_scale();
        let surface =
            self.compositor
                .create_surface_with_data(&self.qh, None, self.scale as i32, ());
        let layer = self.layer_shell.create_layer_surface(
            &self.qh,
            surface,
            Layer::Overlay,
            Some("banshee"),
            None,
        );
        layer.set_anchor(Anchor::BOTTOM);
        layer.set_exclusive_zone(0);
        layer.set_margin(0, 0, BOTTOM_MARGIN, 0);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        match Region::new(&self.compositor) {
            Ok(nowhere) => layer.set_input_region(Some(nowhere.wl_region())),
            Err(error) => log::error!("the capsule may catch the pointer: {error}"),
        }
        layer.set_size(asked.0, asked.1);
        layer.commit();
        Some(Shown {
            layer,
            asked,
            ready: false,
            waiting: false,
        })
    }

    /// The largest scale among the outputs, until the surface reports its own.
    fn largest_scale(&self) -> u32 {
        let scales = self
            .outputs
            .outputs()
            .filter_map(|output| self.outputs.info(&output))
            .map(|info| info.scale_factor);
        u32::try_from(scales.max().unwrap_or(1)).map_or(1, |scale| scale.max(1))
    }

    /// The logical width of the output the capsule last entered, or of the
    /// narrowest output before it has entered one.
    fn output_width(&self) -> Option<i32> {
        let width = |output: &wl_output::WlOutput| {
            self.outputs
                .info(output)?
                .logical_size
                .map(|(width, _)| width)
        };
        self.entered.as_ref().and_then(width).or_else(|| {
            self.outputs
                .outputs()
                .filter_map(|output| width(&output))
                .min()
        })
    }

    fn owns(&self, surface: &wl_surface::WlSurface) -> bool {
        self.surface
            .as_ref()
            .is_some_and(|shown| shown.layer.wl_surface() == surface)
    }

    fn refresh(&mut self, now: Instant) {
        let orders = self.renderer.tick(now);
        self.carry_out(orders, now);
        if self.owed || self.renderer.stale {
            self.draw(now);
        }
    }

    fn draw(&mut self, now: Instant) {
        let Some(shown) = self.surface.as_mut() else {
            return;
        };
        if !shown.ready || shown.waiting {
            return;
        }
        let asked = shown.asked;
        let painted = self.renderer.picture(now, self.scale, |frame| {
            let size = paint::size(frame);
            if size == asked {
                Ok(paint::paint(frame))
            } else {
                Err(size)
            }
        });
        match painted {
            None => {}
            Some(Err((width, height))) => {
                shown.layer.set_size(width, height);
                shown.layer.commit();
                shown.asked = (width, height);
                shown.ready = false;
            }
            Some(Ok(pixmap)) => {
                self.owed = false;
                self.renderer.stale = false;
                let live = self.renderer.live(now);
                if let Err(error) =
                    present(&mut self.pool, &self.qh, shown, &pixmap, self.scale, live)
                {
                    log::error!("the capsule's frame did not reach the screen: {error}");
                }
            }
        }
    }
}

fn present(
    pool: &mut SlotPool,
    qh: &QueueHandle<Stage>,
    shown: &mut Shown,
    pixmap: &Pixmap,
    scale: u32,
    live: bool,
) -> Result<(), Box<dyn Error>> {
    let (width, height) = (pixmap.width() as i32, pixmap.height() as i32);
    let (buffer, canvas) =
        pool.create_buffer(width, height, width * 4, wl_shm::Format::Argb8888)?;
    to_argb(pixmap.data(), canvas);
    let surface = shown.layer.wl_surface();
    // Only a wl_compositor older than version 3 refuses, and it has no scale to set.
    let _ = shown.layer.set_buffer_scale(scale);
    surface.damage_buffer(0, 0, width, height);
    if live {
        surface.frame(qh, FrameCallbackData(surface.clone()));
        shown.waiting = true;
    }
    buffer.attach_to(surface)?;
    shown.layer.commit();
    Ok(())
}

impl CompositorHandler for Stage {
    fn scale_factor_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        factor: i32,
    ) {
        if self.owns(surface) {
            self.scale = u32::try_from(factor).map_or(1, |scale| scale.max(1));
            self.owed = true;
        }
    }

    fn transform_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: wl_output::Transform,
    ) {
    }

    fn frame(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        _: u32,
    ) {
        if let Some(shown) = self
            .surface
            .as_mut()
            .filter(|shown| shown.layer.wl_surface() == surface)
        {
            shown.waiting = false;
            self.owed = true;
        }
    }

    fn surface_enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        output: &wl_output::WlOutput,
    ) {
        if self.owns(surface) {
            self.entered = Some(output.clone());
        }
    }

    fn surface_leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
}

impl OutputHandler for Stage {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.outputs
    }

    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}

    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}

    fn output_destroyed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        if self.entered.as_ref() == Some(&output) {
            self.entered = None;
        }
    }
}

impl LayerShellHandler for Stage {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, layer: &LayerSurface) {
        if self
            .surface
            .as_ref()
            .is_some_and(|shown| &shown.layer == layer)
        {
            self.surface = None;
            self.renderer.clear();
        }
    }

    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        layer: &LayerSurface,
        _: LayerSurfaceConfigure,
        _: u32,
    ) {
        if let Some(shown) = self.surface.as_mut().filter(|shown| &shown.layer == layer) {
            shown.ready = true;
            self.owed = true;
        }
    }
}

impl ShmHandler for Stage {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl ProvidesRegistryState for Stage {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }

    registry_handlers![OutputState];
}

delegate_registry!(Stage);
delegate_dispatch2!(Stage);

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
    fn without_a_wayland_display_the_capsule_does_not_open() {
        assert!(Capsule::open_on(None, || {}).is_none());
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
    fn a_rust_pixel_lands_as_argb8888() {
        let mut argb = [0; 4];
        to_argb(&[0x8A, 0x2A, 0x0D, 0xFF], &mut argb);
        assert_eq!(u32::from_le_bytes(argb), 0xFF8A_2A0D);
    }
}
