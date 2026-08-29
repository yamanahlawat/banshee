use banshee_app::commands::{Daemon, NO_HOME_DIR};
use banshee_app::{bridge, commands};
use tauri::{Emitter, Manager};

// Opening a menu bar app is what puts its icon up. Off this thread, because
// launchd can take seconds and the window must not wait for it.
fn start_the_icon() {
    std::thread::spawn(|| {
        if let Err(error) = commands::open_the_tray() {
            eprintln!("banshee-app: the menu bar icon did not start: {error:?}");
        }
    });
}

fn main() {
    tauri::Builder::default()
        // The tray opens the window by running this binary, so a second press
        // starts a second process. That one hands the window over and exits.
        // This plugin registers before any other, as its own README requires.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
            // A second open reaches this process rather than a new one, so
            // neither the setup below nor the window's mount runs again.
            start_the_icon();
            let _ = app.emit("app:reopened", ());
        }))
        .plugin(tauri_plugin_clipboard_manager::init())
        .setup(|app| {
            start_the_icon();
            // No connection is opened here: a command's first use and a
            // reconnect after the daemon dies both run through the same
            // `commands::ensure_connected`, so the window always opens
            // whether or not the daemon is running yet.
            let daemon = Daemon::new();
            match daemon.socket_path() {
                Some(path) => {
                    let run = bridge::run(app.handle().clone(), path.to_path_buf());
                    tauri::async_runtime::spawn(run);
                }
                None => {
                    let _ = app.emit("daemon:down", serde_json::json!({ "reason": NO_HOME_DIR }));
                }
            }
            app.manage(daemon);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::status,
            commands::set_setting,
            commands::list_devices,
            commands::list_voices,
            commands::preview_voice,
            commands::download_models,
            commands::detect_agents,
            commands::plan_connect,
            commands::apply_connect,
            commands::history,
            commands::clear_history,
            commands::open_permission_pane,
            commands::copy_text,
            commands::start_daemon,
        ])
        .run(tauri::generate_context!())
        .expect("error while running the banshee-app window");
}
