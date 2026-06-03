use dirs;
use std::path::PathBuf;

pub fn get_socket_path() -> Option<PathBuf> {
    let Some(base_path) = dirs::home_dir() else {
        return None;
    };

    Some(base_path.join(".banshee").join("banshee.sock"))
}

