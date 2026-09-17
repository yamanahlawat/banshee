use thiserror::Error;

#[derive(Error, Debug)]
pub enum BansheeError {
    #[error("History is not enabled. Please enable it in the configuration.")]
    HistoryNotEnabled,

    #[error("Transcription failed: {0}")]
    Transcription(String),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    // The path an `Io` cannot carry. An errno with no file names nothing to act on.
    #[error("{path}: {source}")]
    File {
        path: String,
        source: std::io::Error,
    },

    #[error("JSON serialization/deserialization error: {0}")]
    Serde(#[from] serde_json::Error),

    // toml's own message names itself and points at the offending line
    #[error(transparent)]
    Toml(#[from] toml::de::Error),

    #[error("RPC error: {code}: {message}")]
    Rpc { code: i32, message: String },

    // The caller's input, as against anything the daemon failed to do
    #[error("{0}")]
    Rejected(String),

    // No prefix: most of these are sentences a person acts on
    #[error("{0}")]
    Other(String),
}

impl BansheeError {
    /// An io failure that names the file it touched.
    pub fn file(path: impl AsRef<std::path::Path>, source: std::io::Error) -> Self {
        BansheeError::File {
            path: path.as_ref().display().to_string(),
            source,
        }
    }

    /// The text a client should show, without the code in front of it.
    pub fn rpc_message(&self) -> String {
        match self {
            BansheeError::Rpc { message, .. } => message.clone(),
            other => other.to_string(),
        }
    }

    pub fn rpc_code(&self) -> i32 {
        use crate::rpc_code;
        match self {
            BansheeError::HistoryNotEnabled => rpc_code::HISTORY_OFF,
            BansheeError::Rpc { code, .. } => *code,
            BansheeError::Rejected(_) => rpc_code::INVALID_PARAMS,
            BansheeError::Transcription(_)
            | BansheeError::Io(_)
            | BansheeError::File { .. }
            | BansheeError::Serde(_)
            | BansheeError::Toml(_)
            | BansheeError::Other(_) => rpc_code::INTERNAL,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_failure_names_the_path_beside_the_reason() {
        let denied = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        let bare = BansheeError::Io(std::io::Error::from(std::io::ErrorKind::PermissionDenied));
        let named = BansheeError::file("/home/ada/.config/hypr", denied);

        assert!(
            !bare.to_string().contains('/'),
            "the bare errno names no file: {bare}"
        );
        assert_eq!(
            named.to_string(),
            format!("/home/ada/.config/hypr: {bare}"),
            "status reads this sentence out, so it must name the file"
        );
    }

    #[test]
    fn an_other_error_reads_as_it_was_written() {
        let error = BansheeError::Other("config.toml does not parse: line 3".into());
        assert_eq!(
            error.to_string(),
            "config.toml does not parse: line 3",
            "the text is the sentence a person reads, so nothing goes in front of it"
        );
    }
}
