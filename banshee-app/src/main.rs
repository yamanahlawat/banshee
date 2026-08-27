use banshee_app::commands::{Daemon, NO_HOME_DIR};
use banshee_app::{bridge, commands};
use tauri::{Emitter, Manager};

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_clipboard_manager::init())
        .setup(|app| {
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running the banshee-app window");
}
