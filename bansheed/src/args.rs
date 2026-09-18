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
    /// Shows the tray icon now and at every login
    Tray {
        /// Stop the icon and remove its start-at-login service
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
    /// Start, stop or toggle push-to-talk recording (for compositor keybinds and scripts)
    Record {
        #[clap(subcommand)]
        action: RecordAction,
    },
    /// Speaks a message via text-to-speech
    Speak { text: String },
    /// Sends a command to your coding agent, which changes your desktop
    Tell {
        /// What to tell it. Leave it out with --undo.
        text: Option<String>,
        /// Put the config folders back as they were before the last command
        #[clap(long, conflicts_with = "text")]
        undo: bool,
    },
    /// List all transcriptions in the database
    History,
    /// Clears all transcriptions in the database
    ClearHistory,
    /// Runs the daemon in the foreground (what the start-at-login service executes)
    Serve,
    /// Manage the start-at-login service: launchd on macOS, systemd on Linux
    Service {
        #[clap(subcommand)]
        action: ServiceAction,
    },
    /// Remove Banshee: stops it, takes it out of login, and names what owns the rest
    Uninstall {
        /// Also delete ~/.banshee: the models, the history and the keys
        #[clap(long)]
        data: bool,
        /// Remove without asking
        #[clap(long)]
        yes: bool,
    },
    /// Connect a coding agent to Banshee: Antigravity, Claude Code, Codex, Cursor, OpenCode or Pi
    Connect {
        /// Which agent; omit to list what is installed and connected
        agent: Option<AgentName>,
        /// Apply without asking
        #[clap(long)]
        yes: bool,
    },
    /// Bind the push-to-talk key in the compositor's config: Hyprland
    Bind {
        /// Which compositor; omit to print the snippet for the one found
        compositor: Option<CompositorName>,
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

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum CompositorName {
    Hyprland,
}

#[derive(Debug, Subcommand)]
pub enum ConfigAction {
    /// Write one setting, as in: banshee config set stt.language de
    Set {
        /// A section and a field from config.toml, as in stt.vad_threshold
        key: String,
        /// Left out for stt.remote.api_key or tts.remote.api_key, which is then read without echo
        value: Option<String>,
    },
    /// Set up a remote listener and speaker: each server, model, voice and key in turn
    Remote,
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
        /// Send the transcription to your coding agent instead of typing it
        #[clap(long, conflicts_with = "dictate")]
        tell: bool,
    },
    /// Stop recording and transcribe (like releasing the hotkey)
    Stop,
    /// Start when idle, stop when recording (one key for compositors with no release bind)
    Toggle {
        /// Type the transcription into the focused app instead of saving it
        #[clap(long)]
        dictate: bool,
        /// Send the transcription to your coding agent instead of typing it
        #[clap(long, conflicts_with = "dictate")]
        tell: bool,
    },
}
