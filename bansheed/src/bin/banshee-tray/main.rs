//! The menu bar indicator.
//!
//! A separate process from the daemon. AppKit owns the main thread on macOS and
//! gtk owns it elsewhere, while the daemon's thread belongs to tokio. Reading
//! the socket is all this does, so it needs no TCC grants of its own.

fn main() {
    if let Err(error) = tray::run() {
        eprintln!("banshee-tray: {error}");
        std::process::exit(1);
    }
}

mod chip;
#[cfg(any(target_os = "macos", test))]
mod figure;
#[cfg(target_os = "macos")]
mod panel;

mod tray {
    use std::time::{Duration, Instant};

    use banshee_common::cue::Signal;
    use banshee_common::feedback::FeedbackMode;
    use banshee_common::{
        Activity, BANSHEE_CUE, BANSHEE_HISTORY, BANSHEE_LEVEL, BANSHEE_STATE_CHANGED, EVENT_CUES,
        EVENT_LEVEL, EVENT_STATE, utils,
    };
    #[cfg(not(target_os = "macos"))]
    use gtk::glib;
    use serde::Deserialize;
    use serde_json::Value;
    use tray_icon::menu::{IsMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
    use tray_icon::{Icon, TrayIcon, TrayIconBuilder};
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

    // A dead socket answers nothing, so the only way back is to ask again.
    // Nothing measured this number. It trades how long the icon can sit on a
    // stale `Not running` after the daemon returns against how often an idle
    // machine wakes to a failing connect.
    const RETRY: Duration = Duration::from_secs(2);

    // The gtk loop keeps no queue for these messages, so it reads the channel on
    // a timer. Nothing measured this number. It trades how soon the icon answers
    // a click against how often an idle loop wakes to an empty channel.
    #[cfg(not(target_os = "macos"))]
    const POLL: Duration = Duration::from_millis(100);

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
                Indicator::NotRunning => "Not running",
            }
        }
    }

    // The mark ships as six rendered states, drawn from the same geometry the
    // window uses. macOS paints a template image from its alpha alone, so each
    // asset is black with the drawing in the alpha channel. tray-icon renders
    // any icon 18pt tall, which makes 36px its 2x asset.
    fn glyph(indicator: Indicator) -> Result<(Vec<u8>, u32, u32), Box<dyn std::error::Error>> {
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

    fn icon(indicator: Indicator) -> Result<Icon, Box<dyn std::error::Error>> {
        let (pixels, width, height) = glyph(indicator)?;
        Ok(Icon::from_rgba(pixels, width, height)?)
    }

    enum Message {
        State(Indicator),
        // One message for the pair, so user_event drops an unchanged picture
        // before it costs a redraw
        Device(Device),
        History(bool),
        Remote(Remote),
        Chip(chip::Input),
        Level(f32),
        Draws(bool),
        Quit,
        Open,
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
            eprintln!("banshee-tray: the chip must be built on the main thread");
            return None;
        };
        let open = Box::new(move || {
            postbox.post(Message::Open);
        });
        Some(Box::new(super::panel::Panel::new(mtm, open)))
    }

    /// Hands a `Message` to whoever owns the tray. winit carries one as a user
    /// event, and the gtk loop reads one from a channel.
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

    #[derive(Debug, Default, PartialEq, Eq)]
    struct Device {
        open: Option<String>,
        missing: Option<String>,
    }

    impl Device {
        fn of(state: &Value) -> Self {
            Self {
                open: banshee_common::audio_device(state).map(str::to_string),
                missing: banshee_common::missing_device(state).map(str::to_string),
            }
        }
    }

    fn history_enabled_of(status: &Value) -> bool {
        status
            .get("history_enabled")
            .and_then(Value::as_bool)
            .unwrap_or(false)
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

    // Reads as the state, then what it is listening with. A dead daemon has no
    // device to name, so the second line carries the way back instead.
    fn device_line(indicator: Indicator, device: &Device) -> String {
        if indicator == Indicator::NotRunning {
            return "Start with: banshee start".to_string();
        }
        let label =
            banshee_common::microphone_label(device.open.as_deref(), device.missing.as_deref());
        // This menu carries no caption, and "Not open" alone names nothing.
        if device.open.is_some() {
            label
        } else {
            format!("Microphone: {label}")
        }
    }

    enum Row {
        Info(String),
        Separator,
        Action(&'static str, String, bool),
    }

    fn copy_last_enabled(indicator: Indicator, history_enabled: bool) -> bool {
        indicator != Indicator::NotRunning && history_enabled
    }

    fn menu_rows(
        indicator: Indicator,
        device: &Device,
        history_enabled: bool,
        remote: &Remote,
    ) -> Vec<Row> {
        vec![
            Row::Info(indicator.label().to_string()),
            Row::Info(device_line(indicator, device)),
            Row::Info(remote_line(indicator, remote)),
            Row::Separator,
            Row::Action(
                COPY_LAST_ID,
                "Copy last dictation".to_string(),
                copy_last_enabled(indicator, history_enabled),
            ),
            Row::Action(OPEN_ID, "Open Banshee".to_string(), true),
            Row::Separator,
            Row::Action(QUIT_ID, "Quit Banshee".to_string(), true),
        ]
    }

    #[cfg(test)]
    fn menu_labels(
        indicator: Indicator,
        device: &Device,
        history_enabled: bool,
        remote: &Remote,
    ) -> Vec<String> {
        menu_rows(indicator, device, history_enabled, remote)
            .into_iter()
            .map(|row| match row {
                Row::Info(text) | Row::Action(_, text, _) => text,
                Row::Separator => "---".to_string(),
            })
            .collect()
    }

    struct Ui {
        tray: TrayIcon,
        state_item: MenuItem,
        device_item: MenuItem,
        remote_item: MenuItem,
        copy_item: MenuItem,
        // The menu owns the native objects; dropping it empties the tray
        _menu: Menu,
    }

    struct App<P: Postbox> {
        ui: Option<Ui>,
        indicator: Indicator,
        device: Device,
        history_enabled: bool,
        remote: Remote,
        chip: chip::Chip,
        drawing: bool,
        scene: Option<chip::Scene>,
        surface: Option<Box<dyn Surface>>,
        postbox: P,
        begun: bool,
    }

    impl<P: Postbox> App<P> {
        fn new(postbox: P) -> Self {
            Self {
                ui: None,
                indicator: Indicator::NotRunning,
                device: Device::default(),
                history_enabled: false,
                remote: Remote::default(),
                chip: chip::Chip::default(),
                drawing: false,
                scene: None,
                surface: None,
                postbox,
                begun: false,
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
                    Err(error) => eprintln!("banshee-tray: {error}"),
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
            // Redrawing costs a decode, a re-encode and a menu bar repaint, and
            // the reconnect loop repeats itself once a cycle while the daemon is
            // down, so an unchanged value must not reach show()
            let changed = match message {
                // The row says Quit Banshee, so it stops Banshee. This menu is
                // the only way to let go of the microphone and the hotkey
                // without a terminal.
                Message::Quit => {
                    close_the_window().unwrap_or_else(|e| eprintln!("banshee-tray: {e}"));
                    stop_the_daemon().unwrap_or_else(|e| eprintln!("banshee-tray: {e}"));
                    return Flow::Exit;
                }
                Message::State(indicator) => {
                    let moved = self.indicator != indicator;
                    self.indicator = indicator;
                    moved
                }
                Message::Device(device) => {
                    let moved = self.device != device;
                    self.device = device;
                    moved
                }
                Message::History(enabled) => {
                    let moved = self.history_enabled != enabled;
                    self.history_enabled = enabled;
                    moved
                }
                Message::Remote(remote) => {
                    let moved = self.remote != remote;
                    self.remote = remote;
                    moved
                }
                Message::Chip(input) => {
                    self.chip.feed(input, Instant::now());
                    self.redraw();
                    return Flow::Stay;
                }
                Message::Level(level) => {
                    if self.drawing
                        && let Some(surface) = self.surface.as_deref_mut()
                    {
                        surface.level(level);
                    }
                    return Flow::Stay;
                }
                Message::Draws(drawing) => {
                    self.drawing = drawing;
                    self.redraw();
                    return Flow::Stay;
                }
                Message::Open => {
                    open_the_window().unwrap_or_else(|error| eprintln!("banshee-tray: {error}"));
                    return Flow::Stay;
                }
                Message::CopyLast => {
                    spawn_copy_last(self.postbox.clone());
                    return Flow::Stay;
                }
                Message::Copied(text) => {
                    copy_to_clipboard(&text)
                        .unwrap_or_else(|error| eprintln!("banshee-tray: {error}"));
                    return Flow::Stay;
                }
            };
            if changed {
                self.show();
            }
            Flow::Stay
        }

        fn show(&self) {
            let Some(ui) = &self.ui else { return };
            ui.state_item.set_text(self.indicator.label());
            ui.device_item
                .set_text(device_line(self.indicator, &self.device));
            ui.remote_item
                .set_text(remote_line(self.indicator, &self.remote));
            ui.copy_item
                .set_enabled(copy_last_enabled(self.indicator, self.history_enabled));
            if let Err(error) = draw(&ui.tray, self.indicator) {
                eprintln!("banshee-tray: could not draw the icon: {error}");
            }
        }
    }

    fn draw(tray: &TrayIcon, indicator: Indicator) -> Result<(), Box<dyn std::error::Error>> {
        // macOS tints a template image itself, so it takes the flag. Off macOS
        // `set_icon_with_as_template` discards its arguments and answers Ok, so
        // the icon would never change there.
        #[cfg(target_os = "macos")]
        tray.set_icon_with_as_template(Some(icon(indicator)?), true)?;
        #[cfg(not(target_os = "macos"))]
        tray.set_icon(Some(icon(indicator)?))?;
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
        let mut info_items: Vec<MenuItem> = Vec::new();
        let mut copy_item: Option<MenuItem> = None;
        let mut items: Vec<Box<dyn IsMenuItem>> = Vec::new();
        for row in menu_rows(
            Indicator::NotRunning,
            &Device::default(),
            false,
            &Remote::default(),
        ) {
            match row {
                // Informational, so neither row takes a click
                Row::Info(text) => {
                    let item = MenuItem::new(text, false, None);
                    info_items.push(item.clone());
                    items.push(Box::new(item));
                }
                Row::Separator => items.push(Box::new(PredefinedMenuItem::separator())),
                Row::Action(id, text, enabled) => {
                    let item = MenuItem::with_id(id, text, enabled, None);
                    if id == COPY_LAST_ID {
                        copy_item = Some(item.clone());
                    }
                    items.push(Box::new(item));
                }
            }
        }
        let [state_item, device_item, remote_item] = <[MenuItem; 3]>::try_from(info_items)
            .map_err(|_| "menu_rows must carry exactly three info rows")?;
        let copy_item = copy_item.ok_or("menu_rows must include the copy action")?;

        let menu = Menu::new();
        let refs: Vec<&dyn IsMenuItem> = items.iter().map(Box::as_ref).collect();
        menu.append_items(&refs)?;

        let tray = TrayIconBuilder::new()
            // A bar pins an item by the id it reports. The crate's default id
            // carries the process id, so a pin would not survive a restart.
            .with_id("banshee")
            .with_menu(Box::new(menu.clone()))
            .with_icon(icon(Indicator::NotRunning)?)
            .with_icon_as_template(true)
            .with_tooltip("Banshee")
            .build()?;

        Ok(Ui {
            tray,
            state_item,
            device_item,
            remote_item,
            copy_item,
            _menu: menu,
        })
    }

    /// Any failure to read the socket is the daemon being unreachable: a state to show, not an
    /// error to report. `to_draw` asks for the cue and level events too, and silences the
    /// earcons `visual` gives the screen instead.
    async fn watch(postbox: impl Postbox, to_draw: bool) {
        let send = |message| postbox.post(message);
        let events: &[&str] = if to_draw {
            &[EVENT_STATE, EVENT_CUES, EVENT_LEVEL]
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
                    || !send(Message::History(history_enabled_of(&status)))
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
                        _ => true,
                    };
                    if !delivered {
                        return;
                    }
                }
            }
            if !send(Message::State(Indicator::NotRunning))
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
                Err(error) => return eprintln!("banshee-tray: {error}"),
            };
            let reply = runtime.block_on(async {
                tokio::time::timeout(
                    COPY_WAIT,
                    utils::call_daemon(BANSHEE_HISTORY, serde_json::json!({ "limit": 1 })),
                )
                .await
            });
            let Ok(reply) = reply else {
                return eprintln!("banshee-tray: the daemon did not answer in time");
            };
            match reply.as_ref().ok().and_then(last_history_entry) {
                Some(text) => {
                    postbox.post(Message::Copied(text.to_string()));
                }
                None => eprintln!("banshee-tray: no dictation to copy"),
            }
        });
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
                Err(error) => return eprintln!("banshee-tray: no clipboard: {error}"),
            };
            // A dictation paste is excluded from clipboard history. This copy
            // is deliberate, so it belongs there.
            if let Err(error) = clipboard.set().wait_until(Instant::now() + HOLD).text(text) {
                eprintln!("banshee-tray: the copy did not reach the clipboard: {error}");
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
        use std::os::fd::AsRawFd;

        let dir = dirs::home_dir()
            .ok_or("home dir not found")?
            .join(".banshee");
        std::fs::create_dir_all(&dir)?;
        let file = std::fs::File::create(dir.join("tray.lock"))?;

        unsafe extern "C" {
            fn flock(fd: i32, operation: i32) -> i32;
        }
        const EXCLUSIVE_WITHOUT_WAITING: i32 = 2 | 4;
        // SAFETY: `file` is open for the whole call, so the descriptor is valid, and
        // flock reads nothing else.
        if unsafe { flock(file.as_raw_fd(), EXCLUSIVE_WITHOUT_WAITING) } != 0 {
            return Err("the menu bar icon is already running".into());
        }
        Ok(file)
    }

    fn menu_message(id: &str) -> Option<Message> {
        match id {
            QUIT_ID => Some(Message::Quit),
            COPY_LAST_ID => Some(Message::CopyLast),
            OPEN_ID => Some(Message::Open),
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
                Err(error) => eprintln!("banshee-tray: {error}"),
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

    // tray-icon builds its menu out of gtk widgets, so a gtk loop must own the
    // thread that builds the icon. winit's loop is not one.
    #[cfg(not(target_os = "macos"))]
    pub fn run() -> Result<(), Box<dyn std::error::Error>> {
        // Held for the whole run: dropping it would free the lock
        let _lock = claim_the_menu_bar()?;

        gtk::init()?;

        let (sender, receiver) = std::sync::mpsc::channel::<Message>();

        let menu_sender = sender.clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            if let Some(message) = menu_message(event.id.0.as_str()) {
                menu_sender.post(message);
            }
        }));

        spawn_watch(sender.clone(), false);

        let mut app = App::new(sender);
        app.start();

        glib::timeout_add_local(POLL, move || {
            while let Ok(message) = receiver.try_recv() {
                match app.handle(message) {
                    Flow::Stay => {}
                    Flow::Exit => {
                        gtk::main_quit();
                        return glib::ControlFlow::Break;
                    }
                }
            }
            glib::ControlFlow::Continue
        });
        gtk::main();
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

        const STATES: [Indicator; 6] = [
            Indicator::Idle,
            Indicator::Recording,
            Indicator::Speaking,
            Indicator::Listening,
            Indicator::Busy,
            Indicator::NotRunning,
        ];

        fn mask(indicator: Indicator) -> (Vec<u8>, u32, u32) {
            glyph(indicator).expect("a shipped asset must decode")
        }

        // The assets are black with the drawing in the alpha, which macOS tints
        // itself. Any other platform draws what it is given, so a black glyph on a
        // dark bar is invisible.
        #[cfg(not(target_os = "macos"))]
        #[test]
        fn the_linux_glyph_is_not_black() {
            let (pixels, _, _) = glyph(Indicator::Idle).expect("the idle asset decodes");
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
            let (tinted, w, h) = glyph(Indicator::Recording).expect("the asset decodes");
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

        fn device(open: Option<&str>, missing: Option<&str>) -> Device {
            Device {
                open: open.map(str::to_string),
                missing: missing.map(str::to_string),
            }
        }

        #[test]
        fn a_closed_stream_names_the_microphone_and_an_open_one_does_not() {
            assert_eq!(
                device_line(Indicator::Idle, &device(None, None)),
                "Microphone: Not open"
            );
            assert_eq!(
                device_line(Indicator::Idle, &device(Some("Yeti Nano"), None)),
                "Yeti Nano"
            );
        }

        #[test]
        fn the_menu_lists_copy_and_open_between_the_device_and_quit() {
            let labels = menu_labels(
                Indicator::Idle,
                &device(Some("MacBook Pro Microphone"), None),
                true,
                &Remote::default(),
            );
            assert_eq!(
                labels,
                vec![
                    "Idle",
                    "MacBook Pro Microphone",
                    "Audio and text stay on this machine",
                    "---",
                    "Copy last dictation",
                    "Open Banshee",
                    "---",
                    "Quit Banshee",
                ]
            );
        }

        #[test]
        fn the_copy_row_is_disabled_when_history_is_off() {
            fn copy_enabled(indicator: Indicator, history_enabled: bool) -> bool {
                menu_rows(
                    indicator,
                    &Device::default(),
                    history_enabled,
                    &Remote::default(),
                )
                .into_iter()
                .find_map(|row| match row {
                    Row::Action(id, _, enabled) if id == COPY_LAST_ID => Some(enabled),
                    _ => None,
                })
                .expect("menu_rows must include the copy action")
            }

            assert!(!copy_enabled(Indicator::Idle, false));
            assert!(copy_enabled(Indicator::Idle, true));
        }

        // A stopped daemon is the state the window has most to say about: it
        // names the fix and prints the command that applies it.
        #[test]
        fn the_open_row_stays_live_even_when_the_daemon_is_not() {
            for indicator in [Indicator::Idle, Indicator::NotRunning] {
                let open = menu_rows(indicator, &Device::default(), false, &Remote::default())
                    .into_iter()
                    .find_map(|row| match row {
                        Row::Action(id, _, enabled) if id == OPEN_ID => Some(enabled),
                        _ => None,
                    })
                    .expect("menu_rows must include the open action");
                assert!(open, "{indicator:?} must still offer the window");
            }
        }

        #[test]
        fn the_menu_carries_exactly_three_info_rows() {
            for indicator in [Indicator::Idle, Indicator::NotRunning] {
                let info_rows = menu_rows(indicator, &Device::default(), true, &Remote::default())
                    .into_iter()
                    .filter(|row| matches!(row, Row::Info(_)))
                    .count();
                assert_eq!(
                    info_rows, 3,
                    "{indicator:?} must carry exactly three info rows"
                );
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

        // The speaker is chosen at startup, so a voice cleared in the file
        // since then changes nothing until the restart. A row that read the
        // live voice beside the frozen host would claim a privacy this
        // machine does not have.
        #[test]
        fn a_voice_cleared_after_startup_still_names_the_host() {
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
        fn a_dead_daemon_offers_the_way_back_instead_of_a_device() {
            assert_eq!(
                device_line(Indicator::NotRunning, &device(Some("Yeti"), Some("yeti"))),
                "Start with: banshee start"
            );
        }

        #[test]
        fn a_pushed_update_carries_both_device_fields() {
            let pushed = serde_json::json!({
                "recording": false,
                "speaking": false,
                "audio_device": "MacBook Pro Microphone",
                "missing_device": "yeti",
            });
            assert_eq!(
                Device::of(&pushed),
                device(Some("MacBook Pro Microphone"), Some("yeti"))
            );
        }

        #[test]
        fn a_silent_daemon_is_not_running_rather_than_idle() {
            assert_eq!(Indicator::of(None), Indicator::NotRunning);
            assert_eq!(Indicator::of(Some(&live(false, false))), Indicator::Idle);
        }

        fn alpha(indicator: Indicator) -> Vec<u8> {
            mask(indicator)
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
            for (index, one) in STATES.iter().enumerate() {
                for other in &STATES[index + 1..] {
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
            let (pixels, width, height) = mask(Indicator::Idle);
            assert_eq!(pixels.len(), (width * height * 4) as usize);
        }

        // Off macOS, tint() paints real colour into these same pixels on purpose,
        // so this invariant is macOS's alone.
        #[cfg(target_os = "macos")]
        #[test]
        fn every_glyph_draws_in_alpha_only() {
            // A template image is painted by macOS, so a coloured asset renders
            // blank. Every state ships, so every state is checked.
            for indicator in STATES {
                let (pixels, _, _) = mask(indicator);
                assert!(
                    pixels
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .all(|pixel| pixel[..3] == [0, 0, 0]),
                    "{indicator:?} carries colour, which the template renderer drops"
                );
                assert!(
                    pixels.as_chunks::<4>().0.iter().any(|pixel| pixel[3] > 0),
                    "{indicator:?} is an empty mask"
                );
            }
        }

        #[test]
        fn filling_the_shroud_only_adds_ink() {
            // Recording differs from Idle by the fill, so the outline it shares
            // must stay exactly where it was
            let (idle, _, _) = mask(Indicator::Idle);
            let (recording, _, _) = mask(Indicator::Recording);
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
            for indicator in STATES {
                let (pixels, width, height) = mask(indicator);
                let (w, h) = (width as usize, height as usize);
                let alpha = |row: usize, column: usize| pixels[(row * w + column) * 4 + 3];
                for column in 0..w {
                    assert_eq!(alpha(0, column), 0, "{indicator:?} touches the top");
                    assert_eq!(alpha(h - 1, column), 0, "{indicator:?} touches the base");
                }
                for row in 0..h {
                    assert_eq!(alpha(row, 0), 0, "{indicator:?} touches the left");
                    assert_eq!(alpha(row, w - 1), 0, "{indicator:?} touches the right");
                }
            }
        }
    }
}
