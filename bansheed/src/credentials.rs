use std::path::{Path, PathBuf};

use banshee_common::{error::BansheeError, utils::get_credentials_path};
use serde::{Deserialize, Serialize};

// Nested so the key reads as `[stt.remote] api_key`, the way config.toml spells it
#[derive(Deserialize, Serialize, Default)]
#[serde(default)]
struct File {
    stt: Stt,
}

#[derive(Deserialize, Serialize, Default)]
#[serde(default)]
struct Stt {
    remote: Remote,
}

#[derive(Deserialize, Serialize, Default)]
#[serde(default)]
struct Remote {
    api_key: Option<String>,
}

/// What the daemon reads at startup and what a write changes.
#[derive(Default, PartialEq, Eq)]
pub struct Credentials {
    pub stt_api_key: Option<String>,
}

// Hand written, not derived: a derive would put the key itself in every line
// that prints a struct holding this one
impl std::fmt::Debug for Credentials {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let key = match self.stt_api_key {
            Some(_) => "<set>",
            None => "<unset>",
        };
        write!(formatter, "Credentials {{ stt_api_key: {key} }}")
    }
}

impl Credentials {
    pub fn path() -> Result<PathBuf, BansheeError> {
        get_credentials_path()
            .ok_or_else(|| BansheeError::Other("Failed to get the credentials path".to_string()))
    }

    pub fn load() -> Result<Self, BansheeError> {
        Self::read(&Self::path()?)
    }

    /// A file that will not parse holds no key the engine can use, so it answers
    /// the same as an absent one. The status checklist reports the parse fault.
    pub fn stt_key_present() -> bool {
        Self::load().is_ok_and(|credentials| credentials.stt_api_key.is_some())
    }

    fn read(path: &Path) -> Result<Self, BansheeError> {
        let file = Self::read_file(path)?;
        Ok(Self {
            stt_api_key: file.stt.remote.api_key.filter(|key| !key.is_empty()),
        })
    }

    fn read_file(path: &Path) -> Result<File, BansheeError> {
        match std::fs::read_to_string(path) {
            // A TOML parse error quotes the line it stumbled on, so the
            // parser's own text would carry the key into a status reply
            Ok(text) => toml::from_str(&text).map_err(|_| {
                BansheeError::Other(format!(
                    "{} does not parse; fix it or delete it and set the key again: banshee config set stt.remote.api_key",
                    path.display()
                ))
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(File::default()),
            Err(error) => Err(error.into()),
        }
    }

    /// An empty key removes the one on file.
    pub fn set_stt_api_key(key: Option<&str>) -> Result<(), BansheeError> {
        Self::write_stt_api_key(&Self::path()?, key)
    }

    fn write_stt_api_key(path: &Path, key: Option<&str>) -> Result<(), BansheeError> {
        let mut file = Self::read_file(path)?;
        file.stt.remote.api_key = key.filter(|key| !key.is_empty()).map(str::to_string);
        let rendered =
            toml::to_string(&file).map_err(|error| BansheeError::Other(error.to_string()))?;
        banshee_common::utils::write_atomically(path, rendered.as_bytes(), Some(0o600))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::Credentials;
    use std::os::unix::fs::PermissionsExt;

    fn scratch(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("banshee-{name}-{}.toml", std::process::id()))
    }

    #[test]
    fn a_missing_file_holds_no_key() {
        let path = scratch("absent");
        let _ = std::fs::remove_file(&path);
        assert_eq!(Credentials::read(&path).unwrap(), Credentials::default());
    }

    #[test]
    fn a_written_key_is_read_back_and_the_file_is_owner_only() {
        let path = scratch("written");
        Credentials::write_stt_api_key(&path, Some("sk-test")).unwrap();
        assert_eq!(
            Credentials::read(&path).unwrap().stt_api_key.as_deref(),
            Some("sk-test")
        );
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(
            mode, 0o600,
            "the key file must be readable by its owner alone"
        );
        let _ = std::fs::remove_file(&path);
    }

    // A struct that embeds this one and derives Debug would put the key in the
    // first log line that prints it
    #[test]
    fn the_debug_rendering_says_the_key_is_there_and_not_what_it_is() {
        let set = Credentials {
            stt_api_key: Some("sk-test".to_string()),
        };
        let rendering = format!("{set:?}");
        assert!(
            !rendering.contains("sk-test"),
            "the key must not be rendered: {rendering}"
        );
        assert!(rendering.contains("<set>"), "{rendering}");
        assert!(
            format!("{:?}", Credentials::default()).contains("<unset>"),
            "an absent key must say so"
        );
    }

    #[test]
    fn a_file_that_does_not_parse_names_the_path_and_not_the_key() {
        let path = scratch("unparsable");
        std::fs::write(&path, "[stt.remote]\napi_key = sk-live-SECRET123\n").unwrap();
        let error = Credentials::read(&path).unwrap_err().to_string();
        assert!(
            !error.contains("sk-live-SECRET123"),
            "the key must not be in the error: {error}"
        );
        assert!(error.contains(&path.display().to_string()), "{error}");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn an_empty_key_removes_the_one_on_file() {
        let path = scratch("removed");
        Credentials::write_stt_api_key(&path, Some("sk-test")).unwrap();
        Credentials::write_stt_api_key(&path, Some("")).unwrap();
        assert_eq!(Credentials::read(&path).unwrap().stt_api_key, None);
        let _ = std::fs::remove_file(&path);
    }
}
