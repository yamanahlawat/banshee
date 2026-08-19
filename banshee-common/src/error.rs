use thiserror::Error;

#[derive(Error, Debug)]
pub enum BansheeError {
    #[error("Audio device not found!")]
    NoAudioDevice,

    #[error("History is not enabled. Please enable it in the configuration.")]
    HistoryNotEnabled,

    #[error("Transcription failed: {0}")]
    Transcription(String),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error("JSON serialization/deserialization error: {0}")]
    Serde(#[from] serde_json::Error),

    // toml's own message names itself and points at the offending line
    #[error(transparent)]
    Toml(#[from] toml::de::Error),

    #[error("RPC error: {code}: {message}")]
    Rpc { code: i32, message: String },

    #[error("Internal error: {0}")]
    Other(String),
}

impl BansheeError {
    /// The text a client should show, without the code in front of it.
    pub fn rpc_message(&self) -> String {
        match self {
            BansheeError::Rpc { message, .. } => message.clone(),
            other => other.to_string(),
        }
    }

    pub fn rpc_code(&self) -> i32 {
        match self {
            BansheeError::NoAudioDevice => -32000,
            BansheeError::HistoryNotEnabled => -32003,
            BansheeError::Rpc { code, .. } => *code,
            BansheeError::Transcription(_)
            | BansheeError::Io(_)
            | BansheeError::Serde(_)
            | BansheeError::Toml(_)
            | BansheeError::Other(_) => -32603,
        }
    }
}
