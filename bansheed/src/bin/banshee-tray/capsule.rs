//! The capsule on a Wayland desktop: a layer surface on its own thread, each
//! frame painted by tiny-skia into shared memory.

use std::error::Error;
use std::ffi::OsString;
use std::fmt;
use std::sync::mpsc;
use std::time::Instant;

use smithay_client_toolkit::compositor::{
    CompositorHandler, CompositorState, FrameCallbackData, Region,
};
use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::reexports::calloop::{self, EventLoop, channel};
use smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource;
use smithay_client_toolkit::reexports::client::globals::registry_queue_init;
use smithay_client_toolkit::reexports::client::protocol::{
    wl_output, wl_pointer, wl_seat, wl_shm, wl_surface,
};
use smithay_client_toolkit::reexports::client::{Connection, QueueHandle};
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use smithay_client_toolkit::seat::pointer::{
    BTN_LEFT, PointerEvent, PointerEventKind, PointerHandler,
};
use smithay_client_toolkit::seat::{Capability, SeatHandler, SeatState};
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shell::wlr_layer::{
    Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
    LayerSurfaceConfigure,
};
use smithay_client_toolkit::shm::slot::SlotPool;
use smithay_client_toolkit::shm::{Shm, ShmHandler};
use smithay_client_toolkit::{delegate_dispatch2, delegate_registry, registry_handlers};
use tiny_skia::Pixmap;

use crate::chip::Scene;
use crate::figure::{CAPSULE, TRAVEL};
use crate::paint;
use crate::renderer::{Order, Renderer, room_on, to_argb};

/// The clear space below the capsule, `TRAVEL`, already holds it off the edge.
const BOTTOM_MARGIN: i32 = 0;

enum Command {
    Show(Option<Scene>),
    Level(f32),
}

pub struct Capsule {
    commands: channel::Sender<Command>,
}

impl Capsule {
    /// `None` when the desktop has no layer shell to draw on. `open` runs on
    /// the capsule's thread when a click lands on a failure, and `lost` once
    /// the connection ends.
    pub fn open(
        open: impl Fn() + Send + 'static,
        lost: impl FnOnce() + Send + 'static,
    ) -> Option<Self> {
        Self::open_on(std::env::var_os("WAYLAND_DISPLAY"), open, lost)
    }

    fn open_on(
        display: Option<OsString>,
        open: impl Fn() + Send + 'static,
        lost: impl FnOnce() + Send + 'static,
    ) -> Option<Self> {
        let (reply, opened) = mpsc::sync_channel(0);
        let (commands, received) = channel::channel();
        let spawned = std::thread::Builder::new()
            .name("banshee-capsule".into())
            .spawn(move || match connect(display, Box::new(open), received) {
                Err(failure) => {
                    let _ = reply.send(Err(failure));
                }
                Ok((event_loop, stage)) => {
                    let _ = reply.send(Ok(()));
                    let ending = Ending(Some(lost));
                    match stage.run(event_loop) {
                        Ok(()) => ending.quietly(),
                        Err(error) => {
                            log::error!("the capsule's Wayland connection ended: {error}")
                        }
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

    // A send fails only once the thread has ended, and `lost` has then ended
    // the tray.
    pub fn show(&self, scene: Option<&Scene>) {
        let _ = self.commands.send(Command::Show(scene.cloned()));
    }

    pub fn level(&self, peak: f32) {
        let _ = self.commands.send(Command::Level(peak));
    }
}

/// Runs `lost` when dropped, so a panic on the capsule thread ends the tray as
/// a lost connection does.
struct Ending<F: FnOnce()>(Option<F>);

impl<F: FnOnce()> Ending<F> {
    /// The thread ends because the tray let go of the capsule.
    fn quietly(mut self) {
        self.0 = None;
    }
}

impl<F: FnOnce()> Drop for Ending<F> {
    fn drop(&mut self) {
        if let Some(lost) = self.0.take() {
            lost();
        }
    }
}

#[derive(Debug)]
enum Failure {
    NoDisplay,
    Connect(String),
    NoLayerShell,
    EventLoop(String),
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Failure::NoDisplay => f.write_str("WAYLAND_DISPLAY is unset"),
            Failure::Connect(error) => write!(f, "the Wayland connection failed: {error}"),
            Failure::NoLayerShell => f.write_str("the compositor offers no layer shell"),
            Failure::EventLoop(error) => write!(f, "the capsule's event loop failed: {error}"),
        }
    }
}

fn connect(
    display: Option<OsString>,
    open: Box<dyn Fn() + Send>,
    commands: channel::Channel<Command>,
) -> Result<(EventLoop<'static, Stage>, Stage), Failure> {
    if display.is_none() {
        return Err(Failure::NoDisplay);
    }
    let failed = |error: &dyn fmt::Display| Failure::Connect(error.to_string());
    let connection = Connection::connect_to_env().map_err(|error| failed(&error))?;
    let (globals, queue) = registry_queue_init(&connection).map_err(|error| failed(&error))?;
    let qh = queue.handle();
    let compositor = CompositorState::bind(&globals, &qh).map_err(|error| failed(&error))?;
    let shm = Shm::bind(&globals, &qh).map_err(|error| failed(&error))?;
    let seats = SeatState::new(&globals, &qh);
    let layer_shell = LayerShell::bind(&globals, &qh).map_err(|_| Failure::NoLayerShell)?;
    let bare = (CAPSULE * (TRAVEL + CAPSULE)) as usize * 4;
    let pool = SlotPool::new(bare, &shm).map_err(|error| failed(&error))?;
    let stage = Stage {
        registry: RegistryState::new(&globals),
        outputs: OutputState::new(&globals, &qh),
        seats,
        pointer: None,
        open,
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
    let looped = |error: calloop::Error| Failure::EventLoop(error.to_string());
    let event_loop = EventLoop::try_new().map_err(looped)?;
    let handle = event_loop.handle();
    WaylandSource::new(connection, queue)
        .insert(handle.clone())
        .map_err(|error| looped(error.error))?;
    handle
        .insert_source(commands, |event, (), stage: &mut Stage| match event {
            channel::Event::Msg(command) => stage.take(command, Instant::now()),
            channel::Event::Closed => stage.done = true,
        })
        .map_err(|error| looped(error.error))?;
    Ok((event_loop, stage))
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
    /// The input region last committed, where `None` is empty.
    clickable: Option<Rect>,
}

type Rect = (i32, i32, i32, i32);

/// The capsule thread's state, which every Wayland event reaches.
struct Stage {
    registry: RegistryState,
    compositor: CompositorState,
    outputs: OutputState,
    seats: SeatState,
    /// The seat with the pointer, and the pointer.
    pointer: Option<(wl_seat::WlSeat, wl_pointer::WlPointer)>,
    open: Box<dyn Fn() + Send>,
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
    fn run(mut self, mut event_loop: EventLoop<'static, Stage>) -> Result<(), calloop::Error> {
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
        set_clickable(&self.compositor, &layer, None);
        layer.set_size(asked.0, asked.1);
        layer.commit();
        Some(Shown {
            layer,
            asked,
            ready: false,
            waiting: false,
            clickable: None,
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
        self.sync_clickable(now);
    }

    /// Commits the input region when the capsule's rectangle moved.
    fn sync_clickable(&mut self, now: Instant) {
        let Some(shown) = self.surface.as_mut() else {
            return;
        };
        let wanted = self.renderer.input_region(now);
        if wanted != shown.clickable {
            set_clickable(&self.compositor, &shown.layer, wanted);
            shown.layer.commit();
            shown.clickable = wanted;
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

fn set_clickable(compositor: &CompositorState, layer: &LayerSurface, rect: Option<Rect>) {
    match Region::new(compositor) {
        Ok(region) => {
            if let Some((x, y, width, height)) = rect {
                region.add(x, y, width, height);
            }
            layer.set_input_region(Some(region.wl_region()));
        }
        Err(error) => log::error!("the capsule's clicks are not as drawn: {error}"),
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

impl SeatHandler for Stage {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seats
    }

    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}

    fn new_capability(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability != Capability::Pointer || self.pointer.is_some() {
            return;
        }
        match self.seats.get_pointer(qh, &seat) {
            Ok(pointer) => self.pointer = Some((seat, pointer)),
            Err(error) => log::error!("the capsule cannot read the pointer: {error}"),
        }
    }

    fn remove_capability(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Pointer
            && self.pointer.as_ref().is_some_and(|(s, _)| *s == seat)
        {
            self.release_pointer();
        }
    }

    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, seat: wl_seat::WlSeat) {
        if self.pointer.as_ref().is_some_and(|(s, _)| *s == seat) {
            self.release_pointer();
        }
    }
}

impl Stage {
    fn release_pointer(&mut self) {
        if let Some((_, pointer)) = self.pointer.take() {
            if pointer.version() >= 3 {
                pointer.release();
            }
        }
    }
}

impl PointerHandler for Stage {
    fn pointer_frame(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        let now = Instant::now();
        for event in events {
            let PointerEventKind::Press { button, .. } = event.kind else {
                continue;
            };
            let (x, y) = event.position;
            if button == BTN_LEFT && self.owns(&event.surface) && self.renderer.opens_at(x, y, now)
            {
                (self.open)();
            }
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

    registry_handlers![OutputState, SeatState];
}

delegate_registry!(Stage);
delegate_dispatch2!(Stage);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn without_a_wayland_display_the_capsule_does_not_open() {
        assert!(Capsule::open_on(None, || {}, || {}).is_none());
    }

    fn ending_on_a_thread(
        end: impl FnOnce(Ending<Box<dyn FnOnce() + Send>>) + Send + 'static,
    ) -> bool {
        let (told, heard) = mpsc::channel();
        let thread = std::thread::spawn(move || {
            let lost: Box<dyn FnOnce() + Send> = Box::new(move || {
                let _ = told.send(());
            });
            end(Ending(Some(lost)));
        });
        let _ = thread.join();
        heard.try_recv().is_ok()
    }

    #[test]
    fn a_panic_on_the_capsule_thread_reaches_the_tray() {
        assert!(ending_on_a_thread(|_ending| panic!(
            "a bug on the capsule thread"
        )));
    }

    #[test]
    fn a_capsule_the_tray_let_go_of_ends_quietly() {
        assert!(!ending_on_a_thread(Ending::quietly));
    }
}
