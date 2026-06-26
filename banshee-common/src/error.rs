use thiserror::Error;

#[derive(Error, Debug)]
pub enum BansheeError {
    #[error("Audio device not found!")]
    NoAudioDevice,

    #[error("Transcription failed: {0}")]
    Transcription(String),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error("JSON serialization/deserialization error: {0}")]
    Serde(#[from] serde_json::Error),

    #[error("TOML serialization/deserialization error: {0}")]
    Toml(#[from] toml::de::Error),

    #[error("RPC error: {code}: {message}")]
    Rpc { code: i32, message: String },

    #[error("Internal error: {0}")]
    Other(String),
}

impl BansheeError {
    pub fn rpc_code(&self) -> i32 {
        match self {
            BansheeError::NoAudioDevice => -32000,
            BansheeError::Rpc { code, .. } => *code,
            BansheeError::Transcription(_)
            | BansheeError::Io(_)
            | BansheeError::Serde(_)
            | BansheeError::Toml(_)
            | BansheeError::Other(_) => -32603,
        }
    }
}
