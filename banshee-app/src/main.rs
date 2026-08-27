use banshee_app::socket::Client;
use banshee_app::{bridge, commands};
use tauri::Manager;
use tokio::sync::Mutex;

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_clipboard_manager::init())
        .setup(|app| {
            tauri::async_runtime::spawn(bridge::run(app.handle().clone()));
            // No connection is opened here: a command's first use and a
            // reconnect after the daemon dies both run through the same
            // `commands::ensure_connected`, so the window always opens
            // whether or not the daemon is running yet.
            app.manage(Mutex::new(Option::<Client>::None));
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
