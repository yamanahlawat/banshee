use std::path::PathBuf;

/// A folder under the temp dir, removed when the value drops, so a failed
/// assert leaves nothing behind.
pub(crate) struct TempDir(pub(crate) PathBuf);

impl TempDir {
    /// An empty folder named for this process and `name`.
    pub(crate) fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("banshee-common-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
