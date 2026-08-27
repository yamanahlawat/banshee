use banshee_app::bridge;

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            tauri::async_runtime::spawn(bridge::run(app.handle().clone()));
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running the banshee-app window");
}
