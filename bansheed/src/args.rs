use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[clap(author, version, about)]
pub struct Cli {
    #[clap(subcommand)]
    pub command: CommandType,
}

#[derive(Debug, Subcommand)]
pub enum CommandType {
    /// Starts the daemon now and at every login
    Start,
    /// Stops the running daemon
    Stop,
    /// Download required models locally
    Setup,
    /// Checks the health of the running daemon
    Status,
    /// Diagnose setup problems and report fixes
    Doctor,
    /// Gets latest transcription
    Listen,
    /// Start or stop push-to-talk recording (for compositor keybinds and scripts)
    Record {
        #[clap(subcommand)]
        action: RecordAction,
    },
    /// Speaks a message via text-to-speech
    Speak { text: String },
    /// List all transcriptions in the database
    History,
    /// Clears all transcriptions in the database
    ClearHistory,
    /// Runs the daemon in the foreground (what the launch agent executes)
    Serve,
    /// Manage the start-at-login service (macOS launchd)
    Service {
        #[clap(subcommand)]
        action: ServiceAction,
    },
}

#[derive(Debug, Subcommand)]
pub enum ServiceAction {
    /// Stop and remove the launch agent
    Uninstall,
}

#[derive(Debug, Subcommand)]
pub enum RecordAction {
    /// Begin recording (like pressing the hotkey)
    Start {
        /// Type the transcription into the focused app instead of saving it
        #[clap(long)]
        dictate: bool,
    },
    /// Stop recording and transcribe (like releasing the hotkey)
    Stop,
}
