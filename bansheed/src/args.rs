use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[clap(author, version, about)]
pub struct Cli {
    #[clap(subcommand)]
    pub command: CommandType,
}

#[derive(Debug, Subcommand)]
pub enum CommandType {
    /// Starts the background daemon
    Serve,
    /// Download required models locally
    Setup,
    /// Checks the health of the running daemon
    Status,
    /// Gets latest transcription
    Listen,
    /// Speaks a message via text-to-speech
    Speak { text: String },
    /// List all transcriptions in the database
    History,
    /// Clears all transcriptions in the database
    ClearHistory,
}
