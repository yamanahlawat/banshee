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
    /// Shows the menu bar icon now and at every login (macOS)
    Tray {
        /// Stop the icon and remove its launch agent
        #[clap(long)]
        uninstall: bool,
    },
    /// Download required models locally
    Setup,
    /// Reports what Banshee is doing and what stops it working
    Status {
        /// Print the daemon's raw reply instead of the checklist
        #[clap(long)]
        json: bool,
    },
    /// List the microphones Banshee can record from
    Devices,
    /// Follow what the daemon is doing, one line per change
    Watch {
        /// Emit Waybar custom-module JSON instead of one word
        #[clap(long)]
        waybar: bool,
    },
    /// List the text-to-speech voices that are on disk
    Voices,
    /// Change a setting in config.toml
    Config {
        #[clap(subcommand)]
        action: ConfigAction,
    },
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
    /// Connect a coding agent to Banshee: Antigravity, Claude Code, Codex, Cursor, OpenCode or Pi
    Connect {
        /// Which agent; omit to list what is installed and connected
        agent: Option<AgentName>,
        /// Apply without asking
        #[clap(long)]
        yes: bool,
    },
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum AgentName {
    Antigravity,
    Claude,
    Codex,
    Cursor,
    Opencode,
    Pi,
}

#[derive(Debug, Subcommand)]
pub enum ConfigAction {
    /// Write one setting, as in: banshee config set stt.language de
    Set {
        /// A section and a field from config.toml, as in stt.vad_threshold
        key: String,
        value: String,
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
