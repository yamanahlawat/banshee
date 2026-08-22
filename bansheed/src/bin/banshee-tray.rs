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

    use banshee_common::{Activity, BANSHEE_STATE_CHANGED, EVENT_STATE, utils};
    use serde_json::Value;
    use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
    use tray_icon::{Icon, TrayIcon, TrayIconBuilder};
    use winit::application::ApplicationHandler;
    use winit::event::WindowEvent;
    use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
    use winit::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS};
    use winit::window::WindowId;

    const QUIT_ID: &str = "quit";

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
        Device(Option<String>),
        Quit,
    }

    struct Ui {
        tray: TrayIcon,
        state_item: MenuItem,
        device_item: MenuItem,
        // The menu owns the native objects; dropping it empties the tray
        _menu: Menu,
    }

    struct App {
        ui: Option<Ui>,
        indicator: Indicator,
        device: Option<String>,
    }

    impl App {
        // Reads as the state, then what it is listening with. A dead daemon has
        // no device to name, so the second line carries the way back instead.
        fn device_line(&self) -> String {
            match (self.indicator, &self.device) {
                (Indicator::NotRunning, _) => "Start with: banshee start".to_string(),
                (_, Some(device)) => device.clone(),
                (_, None) => "No microphone".to_string(),
            }
        }

        fn show(&self) {
            let Some(ui) = &self.ui else { return };
            ui.state_item.set_text(self.indicator.label());
            ui.device_item.set_text(self.device_line());
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
                Message::Quit => return event_loop.exit(),
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
            };
            if changed {
                self.show();
            }
        }

        fn window_event(&mut self, _: &ActiveEventLoop, _: WindowId, _: WindowEvent) {}
    }

    fn build_ui() -> Result<Ui, Box<dyn std::error::Error>> {
        // Informational, so neither row takes a click
        let state_item = MenuItem::new(Indicator::NotRunning.label(), false, None);
        let device_item = MenuItem::new("", false, None);
        let quit_item = MenuItem::with_id(QUIT_ID, "Quit Banshee", true, None);

        let menu = Menu::new();
        menu.append_items(&[
            &state_item,
            &device_item,
            &PredefinedMenuItem::separator(),
            &quit_item,
        ])?;

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
                let device = banshee_common::audio_device(&status).map(str::to_string);
                if !send(Message::Device(device))
                    || !send(Message::State(Indicator::of(Some(&status))))
                {
                    return;
                }
                while let Ok(Some(state)) = changes.next_of(BANSHEE_STATE_CHANGED).await {
                    if !send(Message::State(Indicator::of(Some(&state)))) {
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
            if event.id.0 == QUIT_ID {
                let _ = menu_proxy.send_event(Message::Quit);
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
            device: None,
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
