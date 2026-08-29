//! The macOS menu bar indicator.
//!
//! A separate process from the daemon. AppKit must own the main thread, and the
//! daemon's belongs to tokio. Reading the socket is all this does, so it needs
//! no TCC grants of its own.

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("banshee-tray runs on macOS only. Elsewhere use: banshee watch --waybar");
    std::process::exit(1);
}

#[cfg(target_os = "macos")]
fn main() {
    if let Err(error) = mac::run() {
        eprintln!("banshee-tray: {error}");
        std::process::exit(1);
    }
}

#[cfg(target_os = "macos")]
mod mac {
    use std::time::Duration;

    use banshee_common::{Activity, BANSHEE_HISTORY, BANSHEE_STATE_CHANGED, EVENT_STATE, utils};
    use serde_json::Value;
    use tray_icon::menu::{IsMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
    use tray_icon::{Icon, TrayIcon, TrayIconBuilder};
    use winit::application::ApplicationHandler;
    use winit::event::WindowEvent;
    use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
    use winit::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS};
    use winit::window::WindowId;

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

    /// What the menu bar shows. `Activity` ranks the two booleans the daemon
    /// pushes; the fourth state is the daemon failing to answer at all.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Indicator {
        Idle,
        Recording,
        Speaking,
        NotRunning,
    }

    impl Indicator {
        fn of(state: Option<&Value>) -> Self {
            match state.map(Activity::of) {
                None => Indicator::NotRunning,
                Some(Activity::Idle) => Indicator::Idle,
                Some(Activity::Recording) => Indicator::Recording,
                Some(Activity::Speaking) => Indicator::Speaking,
            }
        }

        fn label(self) -> &'static str {
            match self {
                Indicator::Idle => "Idle",
                Indicator::Recording => "Recording",
                Indicator::Speaking => "Speaking",
                Indicator::NotRunning => "Not running",
            }
        }
    }

    // The mark ships as four rendered states, generated from
    // assets/banshee-mark.svg. macOS paints a template image from its alpha
    // alone, so each asset is black with the drawing in the alpha channel.
    // tray-icon renders any icon 18pt tall, which makes 36px its 2x asset.
    fn glyph(indicator: Indicator) -> Result<(Vec<u8>, u32, u32), Box<dyn std::error::Error>> {
        let asset: &[u8] = match indicator {
            Indicator::Idle => include_bytes!("../../assets/tray/mark-idle.png"),
            Indicator::Recording => include_bytes!("../../assets/tray/mark-recording.png"),
            Indicator::Speaking => include_bytes!("../../assets/tray/mark-speaking.png"),
            Indicator::NotRunning => include_bytes!("../../assets/tray/mark-notrunning.png"),
        };
        let mut reader = png::Decoder::new(std::io::Cursor::new(asset)).read_info()?;
        let mut pixels = vec![0; reader.output_buffer_size().ok_or("icon too large")?];
        let info = reader.next_frame(&mut pixels)?;
        pixels.truncate(info.buffer_size());
        Ok((pixels, info.width, info.height))
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
        Quit,
        Open,
        CopyLast,
        Copied(String),
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

    // Reads as the state, then what it is listening with. A dead daemon has no
    // device to name, so the second line carries the way back instead.
    fn device_line(indicator: Indicator, device: &Device) -> String {
        if indicator == Indicator::NotRunning {
            return "Start with: banshee start".to_string();
        }
        banshee_common::microphone_label(device.open.as_deref(), device.missing.as_deref())
    }

    /// One row of the menu, before it becomes a native menu item.
    enum Row {
        Info(String),
        Separator,
        Action(&'static str, String, bool),
    }

    fn copy_last_enabled(indicator: Indicator, history_enabled: bool) -> bool {
        indicator != Indicator::NotRunning && history_enabled
    }

    fn menu_rows(indicator: Indicator, device: &Device, history_enabled: bool) -> Vec<Row> {
        vec![
            Row::Info(indicator.label().to_string()),
            Row::Info(device_line(indicator, device)),
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
    fn menu_labels(indicator: Indicator, device: &Device, history_enabled: bool) -> Vec<String> {
        menu_rows(indicator, device, history_enabled)
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
        copy_item: MenuItem,
        // The menu owns the native objects; dropping it empties the tray
        _menu: Menu,
    }

    struct App {
        ui: Option<Ui>,
        indicator: Indicator,
        device: Device,
        history_enabled: bool,
        proxy: EventLoopProxy<Message>,
    }

    impl App {
        fn show(&self) {
            let Some(ui) = &self.ui else { return };
            ui.state_item.set_text(self.indicator.label());
            ui.device_item
                .set_text(device_line(self.indicator, &self.device));
            ui.copy_item
                .set_enabled(copy_last_enabled(self.indicator, self.history_enabled));
            if let Err(error) = draw(&ui.tray, self.indicator) {
                eprintln!("banshee-tray: could not draw the icon: {error}");
            }
        }
    }

    fn draw(tray: &TrayIcon, indicator: Indicator) -> Result<(), Box<dyn std::error::Error>> {
        tray.set_icon_with_as_template(Some(icon(indicator)?), true)?;
        Ok(())
    }

    impl ApplicationHandler<Message> for App {
        fn resumed(&mut self, _event_loop: &ActiveEventLoop) {
            if self.ui.is_some() {
                return;
            }
            match build_ui() {
                Ok(ui) => {
                    self.ui = Some(ui);
                    self.show();
                }
                Err(error) => eprintln!("banshee-tray: {error}"),
            }
        }

        fn user_event(&mut self, event_loop: &ActiveEventLoop, message: Message) {
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
                    return event_loop.exit();
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
                Message::Open => {
                    return open_the_window()
                        .unwrap_or_else(|error| eprintln!("banshee-tray: {error}"));
                }
                Message::CopyLast => return spawn_copy_last(self.proxy.clone()),
                Message::Copied(text) => {
                    return copy_to_clipboard(&text)
                        .unwrap_or_else(|error| eprintln!("banshee-tray: {error}"));
                }
            };
            if changed {
                self.show();
            }
        }

        fn window_event(&mut self, _: &ActiveEventLoop, _: WindowId, _: WindowEvent) {}
    }

    fn build_ui() -> Result<Ui, Box<dyn std::error::Error>> {
        let mut info_items: Vec<MenuItem> = Vec::new();
        let mut copy_item: Option<MenuItem> = None;
        let mut items: Vec<Box<dyn IsMenuItem>> = Vec::new();
        for row in menu_rows(Indicator::NotRunning, &Device::default(), false) {
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
        let [state_item, device_item] = <[MenuItem; 2]>::try_from(info_items)
            .map_err(|_| "menu_rows must carry exactly two info rows")?;
        let copy_item = copy_item.ok_or("menu_rows must include the copy action")?;

        let menu = Menu::new();
        let refs: Vec<&dyn IsMenuItem> = items.iter().map(Box::as_ref).collect();
        menu.append_items(&refs)?;

        let tray = TrayIconBuilder::new()
            .with_menu(Box::new(menu.clone()))
            .with_icon(icon(Indicator::NotRunning)?)
            .with_icon_as_template(true)
            .with_tooltip("Banshee")
            .build()?;

        Ok(Ui {
            tray,
            state_item,
            device_item,
            copy_item,
            _menu: menu,
        })
    }

    /// Subscribes on a worker thread and hands each change to the main thread.
    /// Any failure to read the socket is the daemon being unreachable, which is
    /// a state to show rather than an error to report.
    async fn watch(proxy: EventLoopProxy<Message>) {
        // send_event fails only once the event loop is gone, which means the
        // process is on its way out and this thread with it
        let send = |message| proxy.send_event(message).is_ok();
        loop {
            if let Ok((status, mut changes)) = utils::Subscription::open(&[EVENT_STATE]).await {
                if !send(Message::Device(Device::of(&status)))
                    || !send(Message::State(Indicator::of(Some(&status))))
                    || !send(Message::History(history_enabled_of(&status)))
                {
                    return;
                }
                // Every push carries the device too: the watchdog rebinds while
                // the daemon idles, so no other field has to move with it
                while let Ok(Some(state)) = changes.next_of(BANSHEE_STATE_CHANGED).await {
                    if !send(Message::Device(Device::of(&state)))
                        || !send(Message::State(Indicator::of(Some(&state))))
                    {
                        return;
                    }
                }
            }
            if !send(Message::State(Indicator::NotRunning)) {
                return;
            }
            tokio::time::sleep(RETRY).await;
        }
    }

    /// Reads the last spoken transcription on a worker thread and sends its
    /// text to the main thread.
    fn spawn_copy_last(proxy: EventLoopProxy<Message>) {
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
                    let _ = proxy.send_event(Message::Copied(text.to_string()));
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

    fn copy_to_clipboard(text: &str) -> Result<(), Box<dyn std::error::Error>> {
        arboard::Clipboard::new()?.set_text(text)?;
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
        if unsafe { flock(file.as_raw_fd(), EXCLUSIVE_WITHOUT_WAITING) } != 0 {
            return Err("the menu bar icon is already running".into());
        }
        Ok(file)
    }

    pub fn run() -> Result<(), Box<dyn std::error::Error>> {
        // Held for the whole run: dropping it would free the lock
        let _lock = claim_the_menu_bar()?;

        let event_loop = EventLoop::<Message>::with_user_event()
            // No Dock icon and no menu bar of its own: this is furniture
            .with_activation_policy(ActivationPolicy::Accessory)
            .build()?;
        event_loop.set_control_flow(ControlFlow::Wait);

        let menu_proxy = event_loop.create_proxy();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            let message = match event.id.0.as_str() {
                QUIT_ID => Some(Message::Quit),
                COPY_LAST_ID => Some(Message::CopyLast),
                OPEN_ID => Some(Message::Open),
                _ => None,
            };
            if let Some(message) = message {
                let _ = menu_proxy.send_event(message);
            }
        }));

        // The subscription needs a runtime, and this thread is AppKit's
        let watch_proxy = event_loop.create_proxy();
        std::thread::spawn(move || {
            match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime.block_on(watch(watch_proxy)),
                Err(error) => eprintln!("banshee-tray: {error}"),
            }
        });

        let mut app = App {
            ui: None,
            indicator: Indicator::NotRunning,
            device: Device::default(),
            history_enabled: false,
            proxy: event_loop.create_proxy(),
        };
        event_loop.run_app(&mut app)?;
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn live(recording: bool, speaking: bool) -> Value {
            serde_json::json!({"recording": recording, "speaking": speaking})
        }

        const STATES: [Indicator; 4] = [
            Indicator::Idle,
            Indicator::Recording,
            Indicator::Speaking,
            Indicator::NotRunning,
        ];

        fn mask(indicator: Indicator) -> (Vec<u8>, u32, u32) {
            glyph(indicator).expect("a shipped asset must decode")
        }

        fn device(open: Option<&str>, missing: Option<&str>) -> Device {
            Device {
                open: open.map(str::to_string),
                missing: missing.map(str::to_string),
            }
        }

        #[test]
        fn the_menu_lists_copy_and_open_between_the_device_and_quit() {
            let labels = menu_labels(
                Indicator::Idle,
                &device(Some("MacBook Pro Microphone"), None),
                true,
            );
            assert_eq!(
                labels,
                vec![
                    "Idle",
                    "MacBook Pro Microphone",
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
                menu_rows(indicator, &Device::default(), history_enabled)
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
                let open = menu_rows(indicator, &Device::default(), false)
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
        fn the_menu_carries_exactly_two_info_rows() {
            for indicator in [Indicator::Idle, Indicator::NotRunning] {
                let info_rows = menu_rows(indicator, &Device::default(), true)
                    .into_iter()
                    .filter(|row| matches!(row, Row::Info(_)))
                    .count();
                assert_eq!(
                    info_rows, 2,
                    "{indicator:?} must carry exactly two info rows"
                );
            }
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
