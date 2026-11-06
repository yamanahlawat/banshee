//! The menu bar indicator.
//!
//! A separate process from the daemon. AppKit owns the main thread on macOS,
//! while the daemon's thread belongs to tokio. Reading the socket is all this
//! does, so it needs no TCC grants of its own.

fn main() {
    banshee_common::logging::install("tray");
    if let Err(error) = tray::run() {
        banshee_common::logging::failure(&format!("banshee-tray: {error}"));
        std::process::exit(1);
    }
}

mod chip;
#[cfg(any(target_os = "macos", test))]
mod figure;
#[cfg(target_os = "macos")]
mod panel;

mod tray {
    use std::collections::HashMap;
    use std::time::{Duration, Instant};

    use banshee_common::cue::Signal;
    use banshee_common::feedback::FeedbackMode;
    use banshee_common::{
        Activity, BANSHEE_CUE, BANSHEE_HISTORY, BANSHEE_LEVEL, BANSHEE_STATE_CHANGED,
        BANSHEE_WORDS, EVENT_CUES, EVENT_LEVEL, EVENT_STATE, EVENT_WORDS, utils,
    };
    use serde::Deserialize;
    use serde_json::Value;
    use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
    use tray_icon::{Icon, TrayIcon, TrayIconBuilder};
    #[cfg(not(target_os = "macos"))]
    use tray_icon::{MouseButton, TrayIconEvent};
    #[cfg(target_os = "macos")]
    use winit::application::ApplicationHandler;
    #[cfg(target_os = "macos")]
    use winit::event::{StartCause, WindowEvent};
    #[cfg(target_os = "macos")]
    use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
    #[cfg(target_os = "macos")]
    use winit::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS};
    #[cfg(target_os = "macos")]
    use winit::window::WindowId;

    use super::chip;

    const QUIT_ID: &str = "quit";
    // A daemon that accepts the connection and never answers would hold this
    // thread forever. Nothing measured this number. It trades how long a slow
    // reply can still land against how long a click can hold a thread.
    const COPY_WAIT: Duration = Duration::from_secs(5);

    const COPY_LAST_ID: &str = "copy-last";
    const OPEN_ID: &str = "open";
    const PROBLEM_ID: &str = "problem";
    const START_ID: &str = "start";
    const START_FAILED_ID: &str = "start-failed";
    const STATE_ID: &str = "state";
    const DEVICE_ID: &str = "device";
    const REMOTE_ID: &str = "remote";

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Start {
        Ready,
        Running,
        /// Exited 0 at this instant, and the watch has not reported since.
        /// `banshee start` restarts a daemon that runs, so a second click here
        /// would kill the one just started.
        Started(Instant),
        Failed,
    }

    // A dead socket answers nothing, so the only way back is to ask again.
    // Nothing measured this number. It trades how long the icon can sit on a
    // stale not-running mark after the daemon returns against how often an idle
    // machine wakes to a failing connect.
    const RETRY: Duration = Duration::from_secs(2);

    /// What the menu bar shows. `Activity` ranks the booleans the daemon
    /// pushes; the last state is the daemon failing to answer at all.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Indicator {
        Idle,
        Recording,
        Speaking,
        Listening,
        Busy,
        NotRunning,
    }

    impl Indicator {
        fn of(state: Option<&Value>) -> Self {
            match state.map(Activity::of) {
                None => Indicator::NotRunning,
                Some(Activity::Idle) => Indicator::Idle,
                Some(Activity::Recording) => Indicator::Recording,
                Some(Activity::Speaking) => Indicator::Speaking,
                Some(Activity::Listening) => Indicator::Listening,
                Some(Activity::Busy) => Indicator::Busy,
            }
        }

        fn label(self) -> &'static str {
            match self {
                Indicator::Idle => "Idle",
                Indicator::Recording => "Recording",
                Indicator::Speaking => "Speaking",
                // What it means to a person, not the name on the wire.
                Indicator::Listening => "Waiting for you",
                Indicator::Busy => "Busy",
                Indicator::NotRunning => "Banshee is not running",
            }
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct Problem {
        name: String,
    }

    fn problem_of(state: &Value) -> Option<Problem> {
        let first = state.get("blockers")?.as_array()?.first()?;
        Some(Problem {
            name: first.get("name")?.as_str()?.to_string(),
        })
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Glyph {
        Mark(Indicator),
        IdleWithBadge,
    }

    /// An activity keeps its own mark while it lasts, and a daemon that is
    /// down is the problem itself, so the badge rides on Idle alone.
    fn shown(indicator: Indicator, problem: Option<&Problem>) -> Glyph {
        match (indicator, problem) {
            (Indicator::Idle, Some(_)) => Glyph::IdleWithBadge,
            _ => Glyph::Mark(indicator),
        }
    }

    fn tooltip(indicator: Indicator, problem: Option<&Problem>) -> String {
        match (indicator, problem) {
            (Indicator::NotRunning, _) => indicator.label().to_string(),
            (_, Some(problem)) => format!("Banshee: {}", problem.name),
            (_, None) => format!("Banshee: {}", indicator.label()),
        }
    }

    // In the 36 px asset's pixel coordinates. The disc ends one pixel short of
    // the right edge and on the figure's lowest row.
    const BADGE_CENTRE: (f32, f32) = (28.0, 27.0);
    const BADGE_RADIUS: f32 = 7.0;
    const BADGE_CLEARANCE: f32 = 2.0;
    /// The bar and the dot, as half-open pixel rectangles `(x0, y0, x1, y1)`.
    const EXCLAMATION: [(u32, u32, u32, u32); 2] = [(27, 22, 29, 28), (27, 30, 29, 32)];

    /// Writes alpha only: the badge is part of the template image.
    fn badge(pixels: &mut [u8], width: u32, height: u32) {
        let clear_radius = BADGE_RADIUS + BADGE_CLEARANCE;
        for y in 0..height {
            for x in 0..width {
                let clear = coverage(x, y, clear_radius);
                if clear == 0.0 {
                    continue;
                }
                let cut = EXCLAMATION
                    .iter()
                    .any(|&(x0, y0, x1, y1)| (x0..x1).contains(&x) && (y0..y1).contains(&y));
                let ink = if cut {
                    0.0
                } else {
                    coverage(x, y, BADGE_RADIUS)
                };
                let alpha = &mut pixels[((y * width + x) * 4 + 3) as usize];
                // The disc lies inside the cleared circle, so this stays at or below 255.
                *alpha = (f32::from(*alpha) * (1.0 - clear) + 255.0 * ink).round() as u8;
            }
        }
    }

    /// Sampled on a 4 by 4 grid so the edge is smoothed the way the drawn
    /// assets are.
    fn coverage(x: u32, y: u32, radius: f32) -> f32 {
        const GRID: u32 = 4;
        let (cx, cy) = BADGE_CENTRE;
        let inside = (0..GRID * GRID)
            .filter(|sample| {
                let sx = x as f32 + ((sample % GRID) as f32 + 0.5) / GRID as f32;
                let sy = y as f32 + ((sample / GRID) as f32 + 0.5) / GRID as f32;
                (sx - cx).hypot(sy - cy) <= radius
            })
            .count();
        inside as f32 / (GRID * GRID) as f32
    }

    // The mark ships as six rendered states, drawn from the same geometry the
    // window uses. macOS paints a template image from its alpha alone, so each
    // asset is black with the drawing in the alpha channel. tray-icon renders
    // any icon 18pt tall, which makes 36px its 2x asset.
    fn glyph(shown: Glyph) -> Result<(Vec<u8>, u32, u32), Box<dyn std::error::Error>> {
        let indicator = match shown {
            Glyph::Mark(indicator) => indicator,
            Glyph::IdleWithBadge => Indicator::Idle,
        };
        let asset: &[u8] = match indicator {
            Indicator::Idle => include_bytes!("../../../assets/tray/mark-idle.png"),
            Indicator::Recording => include_bytes!("../../../assets/tray/mark-recording.png"),
            Indicator::Speaking => include_bytes!("../../../assets/tray/mark-speaking.png"),
            Indicator::Listening => include_bytes!("../../../assets/tray/mark-listening.png"),
            Indicator::Busy => include_bytes!("../../../assets/tray/mark-busy.png"),
            Indicator::NotRunning => include_bytes!("../../../assets/tray/mark-notrunning.png"),
        };
        let mut reader = png::Decoder::new(std::io::Cursor::new(asset)).read_info()?;
        let mut pixels = vec![0; reader.output_buffer_size().ok_or("icon too large")?];
        let info = reader.next_frame(&mut pixels)?;
        pixels.truncate(info.buffer_size());
        if shown == Glyph::IdleWithBadge {
            badge(&mut pixels, info.width, info.height);
        }
        tint(&mut pixels);
        Ok((pixels, info.width, info.height))
    }

    // macOS paints a template image from the alpha and picks the colour itself.
    // Every other platform draws the RGB it is given, and the assets are black,
    // so the mark carries white: what macOS produces, and what the other icons
    // in a panel use. A light panel makes white hard to see, and
    // StatusNotifierItem exposes no panel foreground colour to follow instead.
    #[cfg(target_os = "macos")]
    fn tint(_pixels: &mut [u8]) {}

    #[cfg(not(target_os = "macos"))]
    fn tint(pixels: &mut [u8]) {
        for pixel in pixels.as_chunks_mut::<4>().0 {
            // The alpha is the drawing. Only the colour under it changes.
            pixel[0] = 0xff;
            pixel[1] = 0xff;
            pixel[2] = 0xff;
        }
    }

    fn icon(shown: Glyph) -> Result<Icon, Box<dyn std::error::Error>> {
        let (pixels, width, height) = glyph(shown)?;
        Ok(Icon::from_rgba(pixels, width, height)?)
    }

    enum Message {
        State(Indicator),
        Problem(Option<Problem>),
        // One message for the pair, so user_event drops an unchanged picture
        // before it costs a redraw
        Device(Device),
        Remote(Remote),
        Chip(chip::Input),
        Level(f32),
        Draws(bool),
        Quit,
        Open,
        Start,
        StartFinished { succeeded: bool },
        CopyLast,
        Copied(String),
    }

    /// Draws a scene. The panel is macOS only; elsewhere there is nothing to draw.
    trait Surface {
        fn show(&mut self, scene: Option<&chip::Scene>);
        fn level(&mut self, level: f32);
        fn announce(&self, words: &str);
    }

    #[cfg(target_os = "macos")]
    impl Surface for super::panel::Panel {
        fn show(&mut self, scene: Option<&chip::Scene>) {
            self.show(scene);
        }

        fn level(&mut self, level: f32) {
            self.level(level);
        }

        fn announce(&self, words: &str) {
            self.announce(words);
        }
    }

    /// `None` leaves the tray to the menu bar alone, and the daemon to its earcons.
    #[cfg(target_os = "macos")]
    fn build_panel(postbox: impl Postbox) -> Option<Box<dyn Surface>> {
        let Some(mtm) = objc2::MainThreadMarker::new() else {
            log::error!("the chip must be built on the main thread");
            return None;
        };
        let open = Box::new(move || {
            postbox.post(Message::Open);
        });
        Some(Box::new(super::panel::Panel::new(mtm, open)))
    }

    /// Hands a `Message` to whoever owns the tray. winit carries one as a user
    /// event, and the loop off macOS reads one from a channel.
    trait Postbox: Clone + Send + 'static {
        /// False once the far end is gone, which means the process is on its way
        /// out and this thread with it.
        fn post(&self, message: Message) -> bool;
    }

    #[cfg(target_os = "macos")]
    impl Postbox for EventLoopProxy<Message> {
        fn post(&self, message: Message) -> bool {
            self.send_event(message).is_ok()
        }
    }

    #[cfg(any(not(target_os = "macos"), test))]
    impl Postbox for std::sync::mpsc::Sender<Message> {
        fn post(&self, message: Message) -> bool {
            self.send(message).is_ok()
        }
    }

    /// What `App::handle` asks the loop to do next.
    enum Flow {
        Stay,
        Exit,
    }

    #[derive(Debug, PartialEq, Eq)]
    enum Step {
        Show,
        Stay,
        Exit,
    }

    #[derive(Debug, Default, PartialEq, Eq)]
    struct Device {
        name: Option<String>,
        missing: Option<String>,
        open: bool,
        refused: bool,
    }

    impl Device {
        fn of(state: &Value) -> Self {
            Self {
                name: banshee_common::audio_device(state).map(str::to_string),
                missing: banshee_common::missing_device(state).map(str::to_string),
                open: banshee_common::microphone_open(state),
                refused: banshee_common::microphone_refused(state),
            }
        }
    }

    /// A daemon older than the `feedback` field, or one that sends a mode this
    /// tray does not know, draws nothing.
    fn draws(state: &Value) -> bool {
        state
            .get("feedback")
            .and_then(|mode| FeedbackMode::deserialize(mode).ok())
            .is_some_and(FeedbackMode::draws)
    }

    /// `None` for a cue this tray does not know: a newer daemon may send one.
    fn signal_of(params: &Value) -> Option<Signal> {
        Signal::deserialize(params).ok()
    }

    /// Where the audio goes and where the text goes. Both `None` on a machine
    /// that keeps each on itself.
    #[derive(Debug, Default, PartialEq, Eq)]
    struct Remote {
        audio: Option<String>,
        text: Option<String>,
    }

    impl Remote {
        fn of(status: &Value) -> Self {
            Self {
                audio: banshee_common::remote_stt_host(status).map(str::to_string),
                text: speech_host(status),
            }
        }
    }

    /// The host the text goes to, or nothing while it stays on this machine.
    fn speech_host(status: &Value) -> Option<String> {
        // The `remote` object the daemon builds from what it started, not the
        // live table: that table can name another voice, or none, while the
        // speaker on that host still takes every reply
        let host = banshee_common::remote_tts_host(status)?;
        banshee_common::speaker_started(status).then(|| host.to_string())
    }

    // A daemon that is down sends nothing anywhere.
    fn remote_line(indicator: Indicator, remote: &Remote) -> String {
        if indicator == Indicator::NotRunning {
            return "Audio and text stay on this machine".to_string();
        }
        match (remote.audio.as_deref(), remote.text.as_deref()) {
            (None, None) => "Audio and text stay on this machine".to_string(),
            (Some(audio), None) => format!("Audio goes to {audio}"),
            (None, Some(text)) => format!("Text goes to {text}"),
            (Some(audio), Some(text)) if audio == text => {
                format!("Audio and text go to {audio}")
            }
            (Some(audio), Some(text)) => format!("Audio goes to {audio}, text to {text}"),
        }
    }

    // Reads as the state, then what it is listening with. A dead daemon has
    // no device to name.
    fn device_line(indicator: Indicator, device: &Device) -> Option<String> {
        if indicator == Indicator::NotRunning {
            return None;
        }
        // This menu carries no caption, so the line names what it is about.
        Some(format!(
            "Microphone: {}",
            banshee_common::microphone_label(
                device.name.as_deref(),
                device.missing.as_deref(),
                device.open,
                device.refused
            )
        ))
    }

    enum Row {
        Info(&'static str, String),
        Separator,
        Action(&'static str, String, bool),
    }

    impl Row {
        fn id(&self) -> Option<&'static str> {
            match self {
                Row::Info(id, _) | Row::Action(id, _, _) => Some(id),
                Row::Separator => None,
            }
        }
    }

    /// The row above the rest, when there is something to do first. A daemon
    /// that is down has no blockers to rank, so starting it comes before any
    /// problem.
    fn lead_row(indicator: Indicator, problem: Option<&Problem>, start: Start) -> Option<Row> {
        if indicator == Indicator::NotRunning {
            return Some(match start {
                Start::Ready => Row::Action(START_ID, "Start Banshee".to_string(), true),
                Start::Running | Start::Started(_) => {
                    Row::Action(START_ID, "Starting Banshee".to_string(), false)
                }
                Start::Failed => Row::Action(
                    START_FAILED_ID,
                    "Start failed. Open Banshee".to_string(),
                    true,
                ),
            });
        }
        problem.map(|problem| Row::Action(PROBLEM_ID, problem.name.clone(), true))
    }

    fn menu_rows(
        indicator: Indicator,
        problem: Option<&Problem>,
        start: Start,
        device: &Device,
        remote: &Remote,
    ) -> Vec<Row> {
        let mut rows = Vec::new();
        if let Some(row) = lead_row(indicator, problem, start) {
            rows.push(row);
            rows.push(Row::Separator);
        }
        rows.push(Row::Info(STATE_ID, indicator.label().to_string()));
        if let Some(line) = device_line(indicator, device) {
            rows.push(Row::Info(DEVICE_ID, line));
        }
        rows.push(Row::Info(REMOTE_ID, remote_line(indicator, remote)));
        rows.push(Row::Separator);
        rows.push(Row::Action(
            COPY_LAST_ID,
            "Copy last dictation".to_string(),
            indicator != Indicator::NotRunning,
        ));
        rows.push(Row::Action(OPEN_ID, "Open Banshee".to_string(), true));
        rows.push(Row::Separator);
        rows.push(Row::Action(QUIT_ID, "Quit Banshee".to_string(), true));
        rows
    }

    #[cfg(test)]
    fn menu_labels(
        indicator: Indicator,
        problem: Option<&Problem>,
        start: Start,
        device: &Device,
        remote: &Remote,
    ) -> Vec<String> {
        menu_rows(indicator, problem, start, device, remote)
            .into_iter()
            .map(|row| match row {
                Row::Info(_, text) | Row::Action(_, text, _) => text,
                Row::Separator => "---".to_string(),
            })
            .collect()
    }

    struct Ui {
        tray: TrayIcon,
        // The menu owns the native objects; dropping it empties the tray
        menu: Menu,
        items: HashMap<&'static str, Shown>,
        layout: Vec<Option<&'static str>>,
    }

    /// A native item and what was last written to it. Under muda's snapshot
    /// feature on Linux every write rebuilds the whole StatusNotifierItem menu.
    struct Shown {
        item: MenuItem,
        text: String,
        enabled: bool,
    }

    impl Ui {
        /// Makes the menu read `rows`. Each id keeps one native item for the
        /// life of the menu, and the menu is laid out again only when the
        /// sequence of rows changes.
        fn present(&mut self, rows: &[Row]) -> Result<(), Box<dyn std::error::Error>> {
            for row in rows {
                let (id, text, enabled) = match row {
                    Row::Info(id, text) => (*id, text, false),
                    Row::Action(id, text, enabled) => (*id, text, *enabled),
                    Row::Separator => continue,
                };
                let shown = self.items.entry(id).or_insert_with(|| Shown {
                    item: MenuItem::with_id(id, text, enabled, None),
                    text: text.clone(),
                    enabled,
                });
                if shown.text != *text {
                    shown.item.set_text(text);
                    shown.text.clone_from(text);
                }
                if shown.enabled != enabled {
                    shown.item.set_enabled(enabled);
                    shown.enabled = enabled;
                }
            }
            let layout: Vec<_> = rows.iter().map(Row::id).collect();
            if layout == self.layout {
                return Ok(());
            }
            while self.menu.remove_at(0).is_some() {}
            self.layout.clear();
            for id in &layout {
                match id {
                    Some(id) => self.menu.append(&self.items[id].item)?,
                    None => self.menu.append(&PredefinedMenuItem::separator())?,
                }
            }
            self.layout = layout;
            Ok(())
        }
    }

    struct App<P: Postbox> {
        ui: Option<Ui>,
        indicator: Indicator,
        problem: Option<Problem>,
        device: Device,
        remote: Remote,
        chip: chip::Chip,
        drawing: bool,
        scene: Option<chip::Scene>,
        surface: Option<Box<dyn Surface>>,
        postbox: P,
        begun: bool,
        starting: Start,
        run_start: Box<dyn Fn(P)>,
        clock: Box<dyn Fn() -> Instant>,
    }

    impl<P: Postbox> App<P> {
        fn new(postbox: P) -> Self {
            Self {
                ui: None,
                indicator: Indicator::NotRunning,
                problem: None,
                device: Device::default(),
                remote: Remote::default(),
                chip: chip::Chip::default(),
                drawing: false,
                scene: None,
                surface: None,
                postbox,
                begun: false,
                starting: Start::Ready,
                run_start: Box::new(|postbox| spawn_start(postbox, start_the_daemon)),
                clock: Box::new(Instant::now),
            }
        }

        fn start(&mut self) {
            self.start_with(build_ui, Self::begin);
        }

        /// Builds the menu until one build works. `begin` runs on the first
        /// call only, whether or not the menu built.
        fn start_with(
            &mut self,
            build: impl FnOnce() -> Result<Ui, Box<dyn std::error::Error>>,
            begin: impl FnOnce(&mut Self),
        ) {
            if self.ui.is_none() {
                match build() {
                    Ok(ui) => {
                        self.ui = Some(ui);
                        self.show();
                    }
                    Err(error) => log::error!("{error}"),
                }
            }
            if !std::mem::replace(&mut self.begun, true) {
                begin(self);
            }
        }

        /// The panel and the watch, which must each exist once.
        fn begin(&mut self) {
            #[cfg(target_os = "macos")]
            {
                self.surface = build_panel(self.postbox.clone());
                spawn_watch(self.postbox.clone(), self.surface.is_some());
            }
        }

        /// After every chip input and every tick: hides the surface while
        /// `drawing` is false, and speaks the announcement only while it is true.
        fn redraw(&mut self) {
            let before = self.scene.take();
            self.scene = self.chip.scene();
            let Some(surface) = self.surface.as_deref_mut() else {
                return;
            };
            if !self.drawing {
                surface.show(None);
                return;
            }
            surface.show(self.scene.as_ref());
            if let Some(words) = chip::announcement(before.as_ref(), self.scene.as_ref()) {
                surface.announce(&words);
            }
        }

        fn handle(&mut self, message: Message) -> Flow {
            match self.update(message) {
                Step::Show => {
                    self.show();
                    Flow::Stay
                }
                Step::Stay => Flow::Stay,
                Step::Exit => Flow::Exit,
            }
        }

        fn update(&mut self, message: Message) -> Step {
            // Redrawing costs a decode, a re-encode and a menu bar repaint, and
            // the reconnect loop repeats itself once a cycle while the daemon is
            // down, so an unchanged value must not reach show()
            let changed = match message {
                // The row says Quit Banshee, so it stops Banshee. This menu is
                // the only way to let go of the microphone and the hotkey
                // without a terminal.
                Message::Quit => {
                    close_the_window().unwrap_or_else(|e| log::error!("{e}"));
                    stop_the_daemon().unwrap_or_else(|e| log::error!("{e}"));
                    return Step::Exit;
                }
                Message::State(indicator) => {
                    let stale = match self.starting {
                        // The watch reports again only after RETRY, so a
                        // sooner not-running is its connect from before the
                        // start ended.
                        Start::Started(finished) => {
                            indicator != Indicator::NotRunning
                                || (self.clock)().duration_since(finished) >= RETRY
                        }
                        Start::Failed => indicator != Indicator::NotRunning,
                        Start::Ready | Start::Running => false,
                    };
                    if stale {
                        self.starting = Start::Ready;
                    }
                    let moved = std::mem::replace(&mut self.indicator, indicator) != self.indicator;
                    moved || stale
                }
                Message::Problem(problem) => {
                    std::mem::replace(&mut self.problem, problem) != self.problem
                }
                Message::Device(device) => {
                    std::mem::replace(&mut self.device, device) != self.device
                }
                Message::Remote(remote) => {
                    std::mem::replace(&mut self.remote, remote) != self.remote
                }
                Message::Chip(input) => {
                    self.chip.feed(input, Instant::now());
                    self.redraw();
                    return Step::Stay;
                }
                Message::Level(level) => {
                    if self.drawing
                        && let Some(surface) = self.surface.as_deref_mut()
                    {
                        surface.level(level);
                    }
                    return Step::Stay;
                }
                Message::Draws(drawing) => {
                    self.drawing = drawing;
                    self.redraw();
                    return Step::Stay;
                }
                Message::Start => {
                    if self.indicator != Indicator::NotRunning || self.starting != Start::Ready {
                        return Step::Stay;
                    }
                    self.starting = Start::Running;
                    (self.run_start)(self.postbox.clone());
                    true
                }
                Message::StartFinished { succeeded } => {
                    let finished = match (self.indicator, succeeded) {
                        (Indicator::NotRunning, true) => Start::Started((self.clock)()),
                        (Indicator::NotRunning, false) => Start::Failed,
                        _ => Start::Ready,
                    };
                    std::mem::replace(&mut self.starting, finished) != self.starting
                }
                Message::Open => {
                    open_the_window().unwrap_or_else(|error| log::error!("{error}"));
                    return Step::Stay;
                }
                Message::CopyLast => {
                    spawn_copy_last(self.postbox.clone());
                    return Step::Stay;
                }
                Message::Copied(text) => {
                    copy_to_clipboard(&text).unwrap_or_else(|error| log::error!("{error}"));
                    return Step::Stay;
                }
            };
            if changed { Step::Show } else { Step::Stay }
        }

        fn show(&mut self) {
            let problem = self.problem.as_ref();
            let rows = menu_rows(
                self.indicator,
                problem,
                self.starting,
                &self.device,
                &self.remote,
            );
            let Some(ui) = &mut self.ui else { return };
            if let Err(error) = ui.present(&rows) {
                log::error!("could not lay out the menu: {error}");
            }
            if let Err(error) = ui.tray.set_tooltip(Some(tooltip(self.indicator, problem))) {
                log::error!("could not set the tooltip: {error}");
            }
            if let Err(error) = draw(&ui.tray, shown(self.indicator, problem)) {
                log::error!("could not draw the icon: {error}");
            }
        }
    }

    fn draw(tray: &TrayIcon, shown: Glyph) -> Result<(), Box<dyn std::error::Error>> {
        #[cfg(target_os = "macos")]
        tray.set_icon_templated(Some(icon(shown)?))?;
        #[cfg(not(target_os = "macos"))]
        tray.set_icon(Some(icon(shown)?))?;
        Ok(())
    }

    #[cfg(target_os = "macos")]
    impl<P: Postbox> ApplicationHandler<Message> for App<P> {
        fn resumed(&mut self, _event_loop: &ActiveEventLoop) {
            self.start();
        }

        fn new_events(&mut self, _event_loop: &ActiveEventLoop, cause: StartCause) {
            if matches!(cause, StartCause::ResumeTimeReached { .. }) {
                self.chip.tick(Instant::now());
                self.redraw();
            }
        }

        fn user_event(&mut self, event_loop: &ActiveEventLoop, message: Message) {
            match self.handle(message) {
                Flow::Stay => {}
                Flow::Exit => event_loop.exit(),
            }
        }

        fn window_event(&mut self, _: &ActiveEventLoop, _: WindowId, _: WindowEvent) {}

        fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
            event_loop.set_control_flow(match self.chip.deadline() {
                Some(deadline) => ControlFlow::WaitUntil(deadline),
                None => ControlFlow::Wait,
            });
        }
    }

    fn build_ui() -> Result<Ui, Box<dyn std::error::Error>> {
        let menu = Menu::new();
        let builder = TrayIconBuilder::new()
            // A bar pins an item by the id it reports. The crate's default id
            // carries the process id, so a pin would not survive a restart.
            .with_id("banshee")
            .with_menu(Box::new(menu.clone()))
            .with_tooltip(tooltip(Indicator::NotRunning, None));
        let not_running = Glyph::Mark(Indicator::NotRunning);
        #[cfg(target_os = "macos")]
        let builder = builder.with_icon_templated(icon(not_running)?);
        #[cfg(not(target_os = "macos"))]
        let builder = builder.with_icon(icon(not_running)?);
        let tray = builder.build()?;

        Ok(Ui {
            tray,
            menu,
            items: HashMap::new(),
            layout: Vec::new(),
        })
    }

    /// Any failure to read the socket is the daemon being unreachable: a state to show, not an
    /// error to report. `to_draw` asks for the cue, level and words events too, and silences
    /// the earcons `visual` gives the screen instead.
    async fn watch(postbox: impl Postbox, to_draw: bool) {
        let send = |message| postbox.post(message);
        let events: &[&str] = if to_draw {
            &[EVENT_STATE, EVENT_CUES, EVENT_LEVEL, EVENT_WORDS]
        } else {
            &[EVENT_STATE]
        };
        loop {
            let opened = if to_draw {
                utils::Subscription::open_to_draw(events).await
            } else {
                utils::Subscription::open(events).await
            };
            if let Ok((status, mut changes)) = opened {
                if !send(Message::Device(Device::of(&status)))
                    || !send(Message::State(Indicator::of(Some(&status))))
                    || !send(Message::Problem(problem_of(&status)))
                    || !send(Message::Remote(Remote::of(&status)))
                    || !send(Message::Draws(draws(&status)))
                    || !send(Message::Chip(chip::Input::Seed(chip::Live::of(&status))))
                {
                    return;
                }
                // Every push carries the device too: the watchdog rebinds while
                // the daemon idles, so no other field has to move with it
                loop {
                    let Ok(Some(notification)) = changes.next().await else {
                        break;
                    };
                    let delivered = match notification.method.as_str() {
                        BANSHEE_STATE_CHANGED => {
                            send(Message::Device(Device::of(&notification.params)))
                                && send(Message::State(Indicator::of(Some(&notification.params))))
                                && send(Message::Problem(problem_of(&notification.params)))
                                && send(Message::Draws(draws(&notification.params)))
                                && send(Message::Chip(chip::Input::Live(chip::Live::of(
                                    &notification.params,
                                ))))
                        }
                        BANSHEE_CUE => match signal_of(&notification.params) {
                            Some(signal) => send(Message::Chip(chip::Input::Signal(signal))),
                            None => true,
                        },
                        BANSHEE_LEVEL => {
                            match notification.params.get("level").and_then(Value::as_f64) {
                                Some(level) => send(Message::Level(level as f32)),
                                None => true,
                            }
                        }
                        BANSHEE_WORDS => {
                            match notification.params.get("text").and_then(Value::as_str) {
                                Some(text) => {
                                    send(Message::Chip(chip::Input::Words(text.to_string())))
                                }
                                None => true,
                            }
                        }
                        _ => true,
                    };
                    if !delivered {
                        return;
                    }
                }
            }
            if !send(Message::State(Indicator::NotRunning))
                || !send(Message::Problem(None))
                || !send(Message::Chip(chip::Input::Gone))
            {
                return;
            }
            tokio::time::sleep(RETRY).await;
        }
    }

    fn spawn_copy_last(postbox: impl Postbox) {
        std::thread::spawn(move || {
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(error) => return log::error!("{error}"),
            };
            let reply = runtime.block_on(async {
                tokio::time::timeout(
                    COPY_WAIT,
                    utils::call_daemon(
                        BANSHEE_HISTORY,
                        serde_json::json!({ "limit": 1, "kind": "dictation" }),
                    ),
                )
                .await
            });
            let Ok(reply) = reply else {
                return log::error!("the daemon did not answer in time");
            };
            match reply.as_ref().ok().and_then(last_history_entry) {
                Some(text) => {
                    postbox.post(Message::Copied(text.to_string()));
                }
                None => log::warn!("no dictation to copy"),
            }
        });
    }

    type StartRunner = fn() -> Result<bool, Box<dyn std::error::Error>>;

    fn spawn_start(postbox: impl Postbox, start: StartRunner) {
        std::thread::spawn(move || {
            // A thread that dies without posting leaves the row off for good.
            let outcome = std::panic::catch_unwind(start)
                .unwrap_or_else(|_| Err("the start thread panicked".into()));
            let succeeded = outcome.unwrap_or_else(|error| {
                log::error!("{error}");
                false
            });
            postbox.post(Message::StartFinished { succeeded });
        });
    }

    // The menu has no room for the refusal, so the log keeps it. The window's
    // own Start shows it to a person.
    fn start_the_daemon() -> Result<bool, Box<dyn std::error::Error>> {
        let output = utils::sibling_command("banshee")?.arg("start").output()?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let reason = stderr.lines().rev().find(|line| !line.trim().is_empty());
            log::error!(
                "banshee start failed: {}",
                reason.unwrap_or("no reason given")
            );
        }
        Ok(output.status.success())
    }

    fn last_history_entry(reply: &Value) -> Option<&str> {
        reply
            .get("history")?
            .as_array()?
            .last()?
            .get("text")?
            .as_str()
    }

    #[cfg(target_os = "macos")]
    fn copy_to_clipboard(text: &str) -> Result<(), Box<dyn std::error::Error>> {
        arboard::Clipboard::new()?.set_text(text)?;
        Ok(())
    }

    // X11 and Wayland host the clipboard inside the process that last set it,
    // so the text goes when the handle drops. macOS hands it to the system and
    // needs none of this. The thread below owns the text until something else
    // takes the clipboard, or until the hold runs out.
    #[cfg(not(target_os = "macos"))]
    fn copy_to_clipboard(text: &str) -> Result<(), Box<dyn std::error::Error>> {
        use arboard::SetExtLinux;

        // A copy a person asked for survives a detour to another window. The
        // number is a judgement, not a measurement.
        const HOLD: Duration = Duration::from_secs(600);

        let text = text.to_string();
        // The caller cannot see a failure past this thread, so it reports here.
        std::thread::spawn(move || {
            let mut clipboard = match arboard::Clipboard::new() {
                Ok(clipboard) => clipboard,
                Err(error) => return log::error!("no clipboard: {error}"),
            };
            // A dictation paste is excluded from clipboard history. This copy
            // is deliberate, so it belongs there.
            if let Err(error) = clipboard.set().wait_until(Instant::now() + HOLD).text(text) {
                log::error!("the copy did not reach the clipboard: {error}");
            }
        });
        Ok(())
    }

    // A stop over the socket leaves the login agent installed, so the daemon
    // is down now and back at the next login.
    fn stop_the_daemon() -> Result<(), Box<dyn std::error::Error>> {
        utils::sibling_command("banshee")?.arg("stop").status()?;
        Ok(())
    }

    // The window runs as its own process and can come from Spotlight rather
    // than from Open Banshee, so the tray holds no handle for it.
    fn close_the_window() -> Result<(), Box<dyn std::error::Error>> {
        std::process::Command::new("/usr/bin/pkill")
            .args(["-x", "banshee-app"])
            .status()?;
        Ok(())
    }

    // Not `open`, which resolves the bundle id and starts nothing when a
    // second Banshee.app is registered under it. The window is this binary's
    // sibling, so run it directly.
    fn open_the_window() -> Result<(), Box<dyn std::error::Error>> {
        use std::os::unix::process::CommandExt;

        // Its own process group, or it dies with this one. launchd signals
        // the whole group when it boots the job out, and a reinstall does.
        utils::sibling_command("banshee-app")?
            .process_group(0)
            .spawn()?;
        Ok(())
    }

    // launchd runs one job, but nothing stops the binary being started by hand,
    // and a second process means a second icon. The lock lives as long as the
    // process does and the kernel drops it however the process dies.
    fn claim_the_menu_bar() -> Result<std::fs::File, Box<dyn std::error::Error>> {
        let lock = utils::tray_lock_path().ok_or("home dir not found")?;
        if let Some(dir) = lock.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let file = std::fs::File::create(lock)?;
        match file.try_lock() {
            Ok(()) => Ok(file),
            Err(std::fs::TryLockError::WouldBlock) => {
                Err("the menu bar icon is already running".into())
            }
            Err(std::fs::TryLockError::Error(error)) => Err(error.into()),
        }
    }

    fn menu_message(id: &str) -> Option<Message> {
        match id {
            QUIT_ID => Some(Message::Quit),
            COPY_LAST_ID => Some(Message::CopyLast),
            OPEN_ID | PROBLEM_ID | START_FAILED_ID => Some(Message::Open),
            START_ID => Some(Message::Start),
            _ => None,
        }
    }

    // A StatusNotifierItem host sends a left click as `Activate`. Linux trays
    // open their window on it and keep the menu for a right click, which the
    // host shows without asking the tray.
    #[cfg(not(target_os = "macos"))]
    fn click_message(event: &TrayIconEvent) -> Option<Message> {
        match event {
            TrayIconEvent::Click {
                button: MouseButton::Left,
                ..
            } => Some(Message::Open),
            _ => None,
        }
    }

    // The subscription needs a runtime, and the loop owns this thread
    fn spawn_watch(postbox: impl Postbox, to_draw: bool) {
        std::thread::spawn(move || {
            match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime.block_on(watch(postbox, to_draw)),
                Err(error) => log::error!("{error}"),
            }
        });
    }

    #[cfg(target_os = "macos")]
    pub fn run() -> Result<(), Box<dyn std::error::Error>> {
        // Held for the whole run: dropping it would free the lock
        let _lock = claim_the_menu_bar()?;

        let mut builder = EventLoop::<Message>::with_user_event();
        // No Dock icon and no menu bar of its own: this is furniture
        let builder = builder.with_activation_policy(ActivationPolicy::Accessory);
        let event_loop = builder.build()?;
        event_loop.set_control_flow(ControlFlow::Wait);

        let menu_proxy = event_loop.create_proxy();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            if let Some(message) = menu_message(event.id.0.as_str()) {
                menu_proxy.post(message);
            }
        }));

        // The watch starts from `App::start`, once the panel has had its chance
        // to build.
        let mut app = App::new(event_loop.create_proxy());
        event_loop.run_app(&mut app)?;
        Ok(())
    }

    // KSNI serves the icon from its own thread, so this thread only has to
    // take the messages.
    #[cfg(not(target_os = "macos"))]
    pub fn run() -> Result<(), Box<dyn std::error::Error>> {
        // Held for the whole run: dropping it would free the lock
        let _lock = claim_the_menu_bar()?;

        let (sender, receiver) = std::sync::mpsc::channel::<Message>();

        let click_sender = sender.clone();
        TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
            if let Some(message) = click_message(&event) {
                click_sender.post(message);
            }
        }));

        let menu_sender = sender.clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            if let Some(message) = menu_message(event.id.0.as_str()) {
                menu_sender.post(message);
            }
        }));

        spawn_watch(sender.clone(), false);

        let mut app = App::new(sender);
        app.start();

        for message in receiver {
            if let Flow::Exit = app.handle(message) {
                break;
            }
        }
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn live(recording: bool, speaking: bool) -> Value {
            serde_json::json!({"recording": recording, "speaking": speaking})
        }

        #[test]
        fn the_chip_draws_only_in_visual_and_both() {
            for (mode, drawn) in [
                ("visual", true),
                ("both", true),
                ("sound", false),
                ("none", false),
            ] {
                assert_eq!(
                    draws(&serde_json::json!({"feedback": mode})),
                    drawn,
                    "{mode}"
                );
            }
            assert!(
                !draws(&serde_json::json!({})),
                "an older daemon sends no cues to draw"
            );
        }

        #[test]
        fn a_menu_that_fails_to_build_still_begins_the_panel_and_watch_once() {
            let mut app = App::new(std::sync::mpsc::channel::<Message>().0);
            let mut begun = 0;
            for _ in 0..2 {
                app.start_with(|| Err("no menu bar".into()), |_| begun += 1);
            }
            assert_eq!(begun, 1);
        }

        #[test]
        fn a_cue_line_this_tray_does_not_know_is_skipped() {
            assert!(signal_of(&serde_json::json!({"cue": "levitate"})).is_none());
            assert_eq!(
                signal_of(&serde_json::json!({"cue": "arm"})),
                Some(Signal::Arm)
            );
        }

        const GLYPHS: [Glyph; 7] = [
            Glyph::Mark(Indicator::Idle),
            Glyph::Mark(Indicator::Recording),
            Glyph::Mark(Indicator::Speaking),
            Glyph::Mark(Indicator::Listening),
            Glyph::Mark(Indicator::Busy),
            Glyph::Mark(Indicator::NotRunning),
            Glyph::IdleWithBadge,
        ];

        fn mask(shown: Glyph) -> (Vec<u8>, u32, u32) {
            glyph(shown).expect("a shipped asset must decode")
        }

        // The assets are black with the drawing in the alpha, which macOS tints
        // itself. Any other platform draws what it is given, so a black glyph on a
        // dark bar is invisible.
        #[cfg(not(target_os = "macos"))]
        #[test]
        fn the_linux_glyph_is_not_black() {
            let (pixels, _, _) =
                glyph(Glyph::Mark(Indicator::Idle)).expect("the idle asset decodes");
            let lit = pixels
                .as_chunks::<4>()
                .0
                .iter()
                .filter(|p| p[3] > 40)
                .any(|p| p[0] > 32 || p[1] > 32 || p[2] > 32);
            assert!(
                lit,
                "every visible pixel is still black, so the bar shows nothing"
            );
        }

        // The shape lives in the alpha channel, so a tint that touches it redraws
        // the mark.
        #[cfg(not(target_os = "macos"))]
        #[test]
        fn the_tint_leaves_the_shape_alone() {
            let (tinted, w, h) =
                glyph(Glyph::Mark(Indicator::Recording)).expect("the asset decodes");
            let raw = png::Decoder::new(std::io::Cursor::new(
                &include_bytes!("../../../assets/tray/mark-recording.png")[..],
            ))
            .read_info()
            .and_then(|mut r| {
                let mut buf = vec![0; r.output_buffer_size().unwrap()];
                let info = r.next_frame(&mut buf)?;
                buf.truncate(info.buffer_size());
                Ok(buf)
            })
            .expect("the asset decodes twice");
            assert_eq!(tinted.len(), raw.len(), "{w}x{h} must not change size");
            let alpha_of = |v: &[u8]| {
                v.as_chunks::<4>()
                    .0
                    .iter()
                    .map(|p| p[3])
                    .collect::<Vec<_>>()
            };
            assert_eq!(
                alpha_of(&tinted),
                alpha_of(&raw),
                "the alpha carries the mark"
            );
            assert!(
                tinted
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .all(|p| p[..3] == [0xff, 0xff, 0xff]),
                "tint must paint white"
            );
        }

        fn device(name: Option<&str>, missing: Option<&str>) -> Device {
            Device {
                name: name.map(str::to_string),
                missing: missing.map(str::to_string),
                open: false,
                refused: false,
            }
        }

        // An idle daemon has the microphone closed and nothing wrong, so the
        // line names the device the next use opens.
        #[test]
        fn the_microphone_line_names_the_device_and_whether_it_is_open() {
            assert_eq!(
                device_line(Indicator::Idle, &device(Some("Yeti Nano"), None)).as_deref(),
                Some("Microphone: Yeti Nano, opens when you speak")
            );
            assert_eq!(
                device_line(
                    Indicator::Recording,
                    &Device {
                        open: true,
                        ..device(Some("Yeti Nano"), None)
                    }
                )
                .as_deref(),
                Some("Microphone: Yeti Nano, open")
            );
            assert_eq!(
                device_line(Indicator::Idle, &device(None, None)).as_deref(),
                Some("Microphone: not available")
            );
            assert_eq!(
                device_line(
                    Indicator::Idle,
                    &Device {
                        refused: true,
                        ..device(Some("Yeti Nano"), None)
                    }
                )
                .as_deref(),
                Some("Microphone: Yeti Nano, not allowed by macOS"),
                "a refused microphone never reads as one that opens"
            );
        }

        fn labels(indicator: Indicator, problem: Option<&Problem>, start: Start) -> Vec<String> {
            menu_labels(
                indicator,
                problem,
                start,
                &device(Some("MacBook Pro Microphone"), None),
                &Remote::default(),
            )
        }

        fn action(rows: Vec<Row>, id: &str) -> Option<(String, bool)> {
            rows.into_iter().find_map(|row| match row {
                Row::Action(found, text, enabled) if found == id => Some((text, enabled)),
                _ => None,
            })
        }

        fn stopped(start: Start) -> Vec<Row> {
            menu_rows(
                Indicator::NotRunning,
                None,
                start,
                &Device::default(),
                &Remote::default(),
            )
        }

        const TODAYS_ROWS: [&str; 8] = [
            "Idle",
            "Microphone: MacBook Pro Microphone, opens when you speak",
            "Audio and text stay on this machine",
            "---",
            "Copy last dictation",
            "Open Banshee",
            "---",
            "Quit Banshee",
        ];

        #[test]
        fn the_menu_lists_copy_and_open_between_the_device_and_quit() {
            assert_eq!(labels(Indicator::Idle, None, Start::Ready), TODAYS_ROWS);
        }

        #[test]
        fn a_problem_heads_the_menu_and_opens_the_window() {
            let wrong = problem("Accessibility is off");
            assert_eq!(
                labels(Indicator::Idle, Some(&wrong), Start::Ready),
                [&["Accessibility is off", "---"][..], &TODAYS_ROWS[..]].concat()
            );
            let recording = labels(Indicator::Recording, Some(&wrong), Start::Ready);
            assert_eq!(recording[..3], ["Accessibility is off", "---", "Recording"]);
            assert!(matches!(menu_message(PROBLEM_ID), Some(Message::Open)));
        }

        #[test]
        fn a_stopped_daemon_offers_start_banshee_first() {
            assert_eq!(
                labels(Indicator::NotRunning, None, Start::Ready),
                [
                    "Start Banshee",
                    "---",
                    "Banshee is not running",
                    "Audio and text stay on this machine",
                    "---",
                    "Copy last dictation",
                    "Open Banshee",
                    "---",
                    "Quit Banshee",
                ]
            );
            assert_eq!(
                action(stopped(Start::Ready), START_ID),
                Some(("Start Banshee".to_string(), true))
            );
            assert!(matches!(menu_message(START_ID), Some(Message::Start)));
        }

        #[test]
        fn a_stopped_daemon_wins_over_a_problem() {
            let wrong = problem("Accessibility is off");
            let rows = labels(Indicator::NotRunning, Some(&wrong), Start::Ready);
            assert_eq!(rows[0], "Start Banshee");
            assert!(!rows.contains(&"Accessibility is off".to_string()));
        }

        type Clock = std::rc::Rc<std::cell::Cell<Instant>>;

        fn counting_app() -> (
            App<std::sync::mpsc::Sender<Message>>,
            std::rc::Rc<std::cell::Cell<usize>>,
            Clock,
        ) {
            let runs = std::rc::Rc::new(std::cell::Cell::new(0));
            let clock: Clock = std::rc::Rc::new(std::cell::Cell::new(Instant::now()));
            let mut app = App::new(std::sync::mpsc::channel::<Message>().0);
            let counter = runs.clone();
            app.run_start = Box::new(move |_| counter.set(counter.get() + 1));
            let time = clock.clone();
            app.clock = Box::new(move || time.get());
            (app, runs, clock)
        }

        fn started_banshee() -> (
            App<std::sync::mpsc::Sender<Message>>,
            std::rc::Rc<std::cell::Cell<usize>>,
            Clock,
        ) {
            let (mut app, runs, clock) = counting_app();
            app.handle(Message::Start);
            app.handle(Message::StartFinished { succeeded: true });
            (app, runs, clock)
        }

        // The watch's last failed connect can land just after the start exits.
        #[test]
        fn a_not_running_report_just_after_a_start_keeps_the_row_off() {
            let (mut app, runs, clock) = started_banshee();
            clock.set(clock.get() + RETRY / 2);
            app.handle(Message::State(Indicator::NotRunning));
            assert_eq!(
                action(stopped(app.starting), START_ID),
                Some(("Starting Banshee".to_string(), false))
            );
            app.handle(Message::Start);
            assert_eq!(runs.get(), 1);
        }

        #[test]
        fn a_not_running_report_one_retry_later_offers_start_again() {
            let (mut app, runs, clock) = started_banshee();
            clock.set(clock.get() + RETRY);
            app.handle(Message::State(Indicator::NotRunning));
            assert_eq!(
                action(stopped(app.starting), START_ID),
                Some(("Start Banshee".to_string(), true))
            );
            app.handle(Message::Start);
            assert_eq!(runs.get(), 2);
        }

        #[test]
        fn clearing_a_finished_start_redraws_the_menu() {
            let (mut app, _, clock) = started_banshee();
            clock.set(clock.get() + RETRY);
            assert_eq!(
                app.update(Message::State(Indicator::NotRunning)),
                Step::Show
            );
        }

        #[test]
        fn a_start_thread_that_panics_still_reports_a_failure() {
            let (sender, receiver) = std::sync::mpsc::channel::<Message>();
            spawn_start(sender, || panic!("a start that blew up"));
            let posted = receiver.recv_timeout(Duration::from_secs(5));
            assert!(
                matches!(posted, Ok(Message::StartFinished { succeeded: false })),
                "the row would stay on Starting Banshee"
            );
        }

        #[test]
        fn a_running_state_clears_a_success_at_once() {
            let (mut app, _, _) = started_banshee();
            app.handle(Message::State(Indicator::Idle));
            assert_eq!(app.starting, Start::Ready);
        }

        #[test]
        fn the_start_row_is_disabled_while_it_runs() {
            assert_eq!(
                action(stopped(Start::Running), START_ID),
                Some(("Starting Banshee".to_string(), false))
            );

            let (mut app, runs, _) = counting_app();
            app.handle(Message::Start);
            assert_eq!(app.starting, Start::Running);
            app.handle(Message::Start);
            assert_eq!(runs.get(), 1, "a second start ran beside the first");

            app.handle(Message::StartFinished { succeeded: true });
            assert_eq!(
                action(stopped(app.starting), START_ID),
                Some(("Starting Banshee".to_string(), false)),
                "the row stays off until the watch hears from the daemon"
            );
            app.handle(Message::Start);
            assert_eq!(
                runs.get(),
                1,
                "a start after a success restarts the new daemon"
            );
        }

        #[test]
        fn a_running_daemon_takes_no_start() {
            let (mut app, runs, _) = counting_app();
            app.handle(Message::State(Indicator::Idle));
            app.handle(Message::Start);
            assert_eq!(runs.get(), 0);
        }

        #[test]
        fn a_start_that_ends_after_the_daemon_answered_leaves_no_failure() {
            for succeeded in [false, true] {
                let (mut app, _, _) = counting_app();
                app.handle(Message::Start);
                app.handle(Message::State(Indicator::Idle));
                app.handle(Message::StartFinished { succeeded });
                app.handle(Message::State(Indicator::NotRunning));
                assert_eq!(app.starting, Start::Ready, "succeeded: {succeeded}");
            }
        }

        #[test]
        fn a_failed_start_offers_the_window() {
            let (mut app, _, _) = counting_app();
            app.handle(Message::Start);
            app.handle(Message::StartFinished { succeeded: false });
            assert_eq!(app.starting, Start::Failed);
            assert_eq!(action(stopped(Start::Failed), START_ID), None);
            assert_eq!(
                action(stopped(Start::Failed), START_FAILED_ID),
                Some(("Start failed. Open Banshee".to_string(), true))
            );
            assert!(matches!(menu_message(START_FAILED_ID), Some(Message::Open)));

            app.handle(Message::State(Indicator::Idle));
            app.handle(Message::State(Indicator::NotRunning));
            assert_eq!(
                app.starting,
                Start::Ready,
                "a daemon that came back makes the old failure stale"
            );
        }

        // The menu keeps one native item per id, so two rows with one id would
        // share it.
        #[test]
        fn no_menu_carries_one_id_twice() {
            let wrong = problem("Accessibility is off");
            for indicator in [Indicator::Idle, Indicator::Recording, Indicator::NotRunning] {
                for start in [
                    Start::Ready,
                    Start::Running,
                    Start::Started(Instant::now()),
                    Start::Failed,
                ] {
                    let ids: Vec<_> = menu_rows(
                        indicator,
                        Some(&wrong),
                        start,
                        &Device::default(),
                        &Remote::default(),
                    )
                    .iter()
                    .filter_map(Row::id)
                    .collect();
                    let unique: std::collections::HashSet<_> = ids.iter().collect();
                    assert_eq!(unique.len(), ids.len(), "{indicator:?} {start:?}: {ids:?}");
                }
            }
        }

        #[test]
        fn copy_last_is_offered_whenever_the_daemon_runs() {
            fn copy_enabled(indicator: Indicator) -> bool {
                let rows = menu_rows(
                    indicator,
                    None,
                    Start::Ready,
                    &Device::default(),
                    &Remote::default(),
                );
                action(rows, COPY_LAST_ID)
                    .expect("menu_rows must include the copy action")
                    .1
            }

            assert!(copy_enabled(Indicator::Idle));
            assert!(!copy_enabled(Indicator::NotRunning));
        }

        #[test]
        fn a_failed_dictation_has_nothing_to_copy() {
            let failed = serde_json::json!({ "history": [
                { "kind": "dictation", "text": "kept", "outcome": "transcribed" },
                { "kind": "dictation", "text": null, "outcome": "failed" },
            ]});
            let kept = serde_json::json!({ "history": [
                { "kind": "dictation", "text": "kept", "outcome": "transcribed" },
            ]});
            assert_eq!(last_history_entry(&failed), None);
            assert_eq!(last_history_entry(&kept), Some("kept"));
        }

        // A stopped daemon is the state the window has most to say about: it
        // names the fix and prints the command that applies it.
        #[test]
        fn the_open_row_stays_live_even_when_the_daemon_is_not() {
            for indicator in [Indicator::Idle, Indicator::NotRunning] {
                let rows = menu_rows(
                    indicator,
                    None,
                    Start::Ready,
                    &Device::default(),
                    &Remote::default(),
                );
                let open = action(rows, OPEN_ID).expect("menu_rows must include the open action");
                assert!(open.1, "{indicator:?} must still offer the window");
            }
        }

        fn hosts(audio: Option<&str>, text: Option<&str>) -> Remote {
            Remote {
                audio: audio.map(str::to_string),
                text: text.map(str::to_string),
            }
        }

        // One row for both sides, because a menu with two near-identical lines
        // reads as noise rather than as one fact about this machine.
        #[test]
        fn the_row_says_which_sides_leave_the_machine() {
            let line = |audio, text| remote_line(Indicator::Idle, &hosts(audio, text));
            assert_eq!(line(None, None), "Audio and text stay on this machine");
            assert_eq!(
                line(Some("api.groq.com"), None),
                "Audio goes to api.groq.com"
            );
            assert_eq!(
                line(None, Some("api.openai.com")),
                "Text goes to api.openai.com"
            );
            assert_eq!(
                line(Some("api.groq.com"), Some("api.openai.com")),
                "Audio goes to api.groq.com, text to api.openai.com"
            );
            assert_eq!(
                line(Some("api.openai.com"), Some("api.openai.com")),
                "Audio and text go to api.openai.com"
            );
        }

        // A daemon that is down sends nothing anywhere.
        #[test]
        fn the_hosts_drop_with_the_daemon() {
            assert_eq!(
                remote_line(
                    Indicator::NotRunning,
                    &hosts(Some("api.groq.com"), Some("api.openai.com"))
                ),
                "Audio and text stay on this machine"
            );
        }

        #[test]
        fn both_hosts_are_read_off_the_status_reply() {
            let reply = serde_json::json!({
                "remote": {
                    "stt": {"remote": true, "host": "api.groq.com"},
                    "tts": {"remote": true, "host": "api.openai.com", "speaker_started": true},
                }
            });
            assert_eq!(
                Remote::of(&reply),
                hosts(Some("api.groq.com"), Some("api.openai.com"))
            );
            let older = serde_json::json!({"remote": {"stt": false, "tts": false}});
            assert_eq!(Remote::of(&older), hosts(None, None));
        }

        // A speaker that did not start speaks nothing, so the reply is spoken
        // here and the row may not say the text leaves. The host stands in the
        // reply either way: it is where the text would have gone.
        #[test]
        fn a_speaker_that_did_not_start_sends_no_text_anywhere() {
            let reply = serde_json::json!({
                "remote": {
                    "stt": {"remote": false, "host": null},
                    "tts": {"remote": true, "host": "api.openai.com", "speaker_started": false},
                }
            });
            assert_eq!(Remote::of(&reply), hosts(None, None));
            assert_eq!(
                remote_line(Indicator::Idle, &Remote::of(&reply)),
                "Audio and text stay on this machine"
            );
        }

        // The daemon reports the speaker in force, which the file can stop
        // agreeing with. A row that read the voice in the file beside the host
        // the daemon reports would claim a privacy this machine does not have.
        #[test]
        fn a_cleared_voice_names_the_host_the_daemon_reports() {
            let reply = serde_json::json!({
                "config": {"tts": {"remote": {"voice": ""}}},
                "remote": {
                    "stt": {"remote": false, "host": null},
                    "tts": {"remote": true, "host": "api.openai.com", "speaker_started": true},
                }
            });
            assert_eq!(Remote::of(&reply), hosts(None, Some("api.openai.com")));
            assert_eq!(
                remote_line(Indicator::Idle, &Remote::of(&reply)),
                "Text goes to api.openai.com"
            );
        }

        #[test]
        fn a_dead_daemon_names_no_microphone() {
            assert_eq!(
                device_line(Indicator::NotRunning, &device(Some("Yeti"), Some("yeti"))),
                None
            );
        }

        #[test]
        fn a_pushed_update_carries_every_microphone_field() {
            let pushed = serde_json::json!({
                "recording": true,
                "speaking": false,
                "audio_device": "MacBook Pro Microphone",
                "missing_device": "yeti",
                "microphone_open": true,
                "microphone_refused": true,
            });
            assert_eq!(
                Device::of(&pushed),
                Device {
                    open: true,
                    refused: true,
                    ..device(Some("MacBook Pro Microphone"), Some("yeti"))
                }
            );
        }

        fn problem(name: &str) -> Problem {
            Problem {
                name: name.to_string(),
            }
        }

        #[test]
        fn the_first_blocker_is_the_problem() {
            let pushed = serde_json::json!({
                "blockers": [
                    {"id": "accessibility", "name": "Accessibility is off", "fix": "Open Settings"},
                    {"id": "kokoro", "name": "The voice model is missing", "fix": "banshee setup"},
                ]
            });
            assert_eq!(problem_of(&pushed), Some(problem("Accessibility is off")));
            assert_eq!(problem_of(&serde_json::json!({"blockers": []})), None);
            assert_eq!(
                problem_of(&serde_json::json!({})),
                None,
                "an older daemon sends no blockers"
            );
        }

        #[test]
        fn a_problem_shows_its_badge_only_at_rest() {
            let wrong = problem("Accessibility is off");
            assert_eq!(shown(Indicator::Idle, Some(&wrong)), Glyph::IdleWithBadge);
            assert_eq!(shown(Indicator::Idle, None), Glyph::Mark(Indicator::Idle));
            for indicator in [
                Indicator::Recording,
                Indicator::Speaking,
                Indicator::Listening,
                Indicator::Busy,
                Indicator::NotRunning,
            ] {
                assert_eq!(
                    shown(indicator, Some(&wrong)),
                    Glyph::Mark(indicator),
                    "{indicator:?} keeps its own mark"
                );
            }
        }

        #[test]
        fn the_badge_is_alpha_only_and_stays_in_its_corner() {
            const UNTOUCHED: u8 = 0x80;
            let (width, height) = (36, 36);
            let mut pixels = [0, 0, 0, UNTOUCHED].repeat(width * height);
            badge(&mut pixels, width as u32, height as u32);

            let (cx, cy) = BADGE_CENTRE;
            let clear = BADGE_RADIUS + BADGE_CLEARANCE;
            // Half a pixel's diagonal: a pixel whose centre is this far inside
            // a boundary lies wholly on that side of it.
            let margin = std::f32::consts::FRAC_1_SQRT_2;
            let cut = |x: usize, y: usize| {
                EXCLAMATION.iter().any(|&(x0, y0, x1, y1)| {
                    (x0 as usize..x1 as usize).contains(&x)
                        && (y0 as usize..y1 as usize).contains(&y)
                })
            };
            let (mut ring, mut disc, mut mark) = (0, 0, 0);
            for y in 0..height {
                for x in 0..width {
                    let pixel = &pixels[(y * width + x) * 4..][..4];
                    assert_eq!(pixel[..3], [0, 0, 0], "({x}, {y}) took colour");
                    let distance = (x as f32 + 0.5 - cx).hypot(y as f32 + 0.5 - cy);
                    let corner = x as f32 + 1.0 > cx - clear
                        && y as f32 + 1.0 > cy - clear
                        && (x as f32) < cx + clear
                        && (y as f32) < cy + clear;
                    if !corner {
                        assert_eq!(pixel[3], UNTOUCHED, "({x}, {y}) is outside the corner");
                    } else if cut(x, y) {
                        mark += 1;
                        assert_eq!(pixel[3], 0, "({x}, {y}) is the exclamation mark");
                    } else if distance <= BADGE_RADIUS - margin {
                        disc += 1;
                        assert_eq!(pixel[3], 0xff, "({x}, {y}) is inside the disc");
                    } else if distance >= BADGE_RADIUS + margin && distance <= clear - margin {
                        ring += 1;
                        assert_eq!(pixel[3], 0, "({x}, {y}) is in the clear ring");
                    }
                }
            }
            assert!(ring > 0 && disc > 0 && mark > 0, "{ring} {disc} {mark}");

            let ink_at = |x: usize, y: usize| pixels[(y * width + x) * 4 + 3];
            for y in 0..height {
                for x in (0..width).filter(|&x| cut(x, y)) {
                    let inked = [(x - 1, y), (x + 1, y), (x, y - 1), (x, y + 1)]
                        .iter()
                        .any(|&(nx, ny)| !cut(nx, ny) && ink_at(nx, ny) == 0xff);
                    assert!(inked, "({x}, {y}) of the mark has no ink beside it");
                }
            }

            let mut blank = [0, 0, 0, 0].repeat(width * height);
            badge(&mut blank, width as u32, height as u32);
            let (mut weight, mut sum_x, mut sum_y) = (0.0, 0.0, 0.0);
            for y in 0..height {
                for x in 0..width {
                    let ink = f32::from(blank[(y * width + x) * 4 + 3]);
                    weight += ink;
                    sum_x += ink * (x as f32 + 0.5);
                    sum_y += ink * (y as f32 + 0.5);
                }
            }
            let centroid = (sum_x / weight, sum_y / weight);
            assert!(
                centroid.0 > width as f32 / 2.0 && centroid.1 > height as f32 / 2.0,
                "the badge sits at {centroid:?}, outside the lower right quadrant"
            );
        }

        #[test]
        fn the_tooltip_names_the_problem() {
            let wrong = problem("Accessibility is off");
            assert_eq!(
                tooltip(Indicator::Idle, Some(&wrong)),
                "Banshee: Accessibility is off"
            );
            assert_eq!(
                tooltip(Indicator::Speaking, Some(&wrong)),
                "Banshee: Accessibility is off"
            );
            assert_eq!(tooltip(Indicator::Idle, None), "Banshee: Idle");
            assert_eq!(
                tooltip(Indicator::NotRunning, Some(&wrong)),
                "Banshee is not running",
                "a daemon that is down wins over a problem"
            );
        }

        #[test]
        fn a_silent_daemon_is_not_running_rather_than_idle() {
            assert_eq!(Indicator::of(None), Indicator::NotRunning);
            assert_eq!(Indicator::of(Some(&live(false, false))), Indicator::Idle);
        }

        fn alpha(shown: Glyph) -> Vec<u8> {
            mask(shown)
                .0
                .as_chunks::<4>()
                .0
                .iter()
                .map(|pixel| pixel[3])
                .collect()
        }

        #[test]
        fn every_state_has_its_own_silhouette() {
            // Compared on alpha alone: macOS paints a template image from that
            // channel, so two assets differing only in colour render the same
            for (index, one) in GLYPHS.iter().enumerate() {
                for other in &GLYPHS[index + 1..] {
                    assert_ne!(
                        alpha(*one),
                        alpha(*other),
                        "{one:?} and {other:?} must differ by shape, not by colour"
                    );
                }
            }
        }

        #[test]
        fn a_glyph_is_one_rgba_pixel_per_cell() {
            let (pixels, width, height) = mask(Glyph::Mark(Indicator::Idle));
            assert_eq!(pixels.len(), (width * height * 4) as usize);
        }

        // Off macOS, tint() paints real colour into these same pixels on purpose,
        // so this invariant is macOS's alone.
        #[cfg(target_os = "macos")]
        #[test]
        fn every_glyph_draws_in_alpha_only() {
            // A template image is painted by macOS, so a coloured asset renders
            // blank. Every glyph ships, so every glyph is checked.
            for shown in GLYPHS {
                let (pixels, _, _) = mask(shown);
                assert!(
                    pixels
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .all(|pixel| pixel[..3] == [0, 0, 0]),
                    "{shown:?} carries colour, which the template renderer drops"
                );
                assert!(
                    pixels.as_chunks::<4>().0.iter().any(|pixel| pixel[3] > 0),
                    "{shown:?} is an empty mask"
                );
            }
        }

        #[test]
        fn filling_the_shroud_only_adds_ink() {
            // Recording differs from Idle by the fill, so the outline it shares
            // must stay exactly where it was
            let (idle, _, _) = mask(Glyph::Mark(Indicator::Idle));
            let (recording, _, _) = mask(Glyph::Mark(Indicator::Recording));
            for (before, after) in idle
                .as_chunks::<4>()
                .0
                .iter()
                .zip(recording.as_chunks::<4>().0)
            {
                assert!(after[3] >= before[3], "the shroud moved instead of filling");
            }
        }

        #[test]
        fn the_glyph_keeps_clear_of_the_edges() {
            for shown in GLYPHS {
                let (pixels, width, height) = mask(shown);
                let (w, h) = (width as usize, height as usize);
                let alpha = |row: usize, column: usize| pixels[(row * w + column) * 4 + 3];
                for column in 0..w {
                    assert_eq!(alpha(0, column), 0, "{shown:?} touches the top");
                    assert_eq!(alpha(h - 1, column), 0, "{shown:?} touches the base");
                }
                for row in 0..h {
                    assert_eq!(alpha(row, 0), 0, "{shown:?} touches the left");
                    assert_eq!(alpha(row, w - 1), 0, "{shown:?} touches the right");
                }
            }
        }

        #[cfg(not(target_os = "macos"))]
        fn click(button: MouseButton) -> TrayIconEvent {
            TrayIconEvent::Click {
                id: tray_icon::TrayIconId::new("banshee"),
                position: tray_icon::dpi::PhysicalPosition::new(0.0, 0.0),
                rect: tray_icon::Rect::default(),
                button,
                button_state: tray_icon::MouseButtonState::Up,
            }
        }

        #[cfg(not(target_os = "macos"))]
        #[test]
        fn a_left_click_opens_the_window_and_no_other_click_does() {
            assert!(matches!(
                click_message(&click(MouseButton::Left)),
                Some(Message::Open)
            ));
            assert!(click_message(&click(MouseButton::Middle)).is_none());
            assert!(click_message(&click(MouseButton::Right)).is_none());
        }
    }
}
