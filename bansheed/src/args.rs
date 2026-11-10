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
    Tray,
    /// Download required models locally
    Setup {
        /// Download this listening model instead, by its id
        #[clap(long)]
        model: Option<String>,
        /// Accept the model's licence where it asks for acceptance
        #[clap(long, requires = "model")]
        accept_licence: bool,
        /// List every listening model Banshee can download
        #[clap(long, conflicts_with_all = ["model", "accept_licence"])]
        list: bool,
    },
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
        /// Print these events as JSON lines instead: cues, level, words, question
        #[clap(long, value_delimiter = ',', conflicts_with = "waybar")]
        events: Vec<String>,
    },
    /// List the text-to-speech voices that are on disk
    Voices,
    /// Change a setting in config.toml
    Config {
        #[clap(subcommand)]
        action: ConfigAction,
    },
    /// Start, stop or toggle push-to-talk recording (for compositor keybinds and scripts)
    Record {
        #[clap(subcommand)]
        action: RecordAction,
    },
    /// Speaks a message via text-to-speech
    Speak { text: String },
    /// List your dictations and the questions agents asked you, with your answers
    History,
    /// Clear your dictations and the questions agents asked you, with your answers
    ClearHistory,
    /// Runs the daemon in the foreground (what the start-at-login service executes)
    Serve,
    /// Remove Banshee: stops it, takes it out of login, and names what owns the rest
    Uninstall {
        /// Also delete ~/.banshee: the models, the history and the keys
        #[clap(long)]
        data: bool,
        /// Remove without asking
        #[clap(long)]
        yes: bool,
    },
    /// Connect a coding agent to Banshee: Antigravity, Claude Code, Codex, Copilot, Cursor, OpenCode or Pi
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
    /// Answers an agent's Stop hook: whether the agent spoke this turn
    #[clap(hide = true)]
    TurnEnd {
        /// The agent whose hook runs this
        agent: crate::turn_end::GatedAgent,
    },
    /// Prints the frontmost app's pid, read fresh in this process
    #[cfg(target_os = "macos")]
    #[clap(hide = true)]
    FrontmostPid,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum AgentName {
    Antigravity,
    Claude,
    Codex,
    Copilot,
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
    /// Write one setting, as in: banshee config set stt.model whisper-large-v3-turbo, then banshee setup
    Set {
        /// A section and a field from config.toml, as in stt.vad_threshold. A key that is no setting is refused
        key: String,
        /// Left out for stt.remote.api_key or tts.remote.api_key, which is then read without echo
        value: Option<String>,
    },
    /// Set up a remote listener and speaker: each server, model, voice and key in turn
    Remote,
}

#[derive(Debug, Subcommand)]
pub enum RecordAction {
    /// Begin recording (like pressing the hotkey)
    Start,
    /// Stop recording and transcribe (like releasing the hotkey)
    Stop,
    /// Start when idle, stop when recording (one key for compositors with no release bind)
    Toggle,
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::Cli;

    #[test]
    fn watch_refuses_waybar_together_with_events() {
        Cli::try_parse_from(["banshee", "watch", "--events", "cues"])
            .expect("events alone must parse");
        Cli::try_parse_from(["banshee", "watch", "--waybar", "--events", "cues"])
            .expect_err("waybar and events together must be refused");
    }

    #[test]
    fn a_record_bind_that_passes_dictate_is_refused() {
        for action in ["start", "toggle"] {
            Cli::try_parse_from(["banshee", "record", action, "--dictate"])
                .expect_err("--dictate is not a flag");
        }
    }
}
