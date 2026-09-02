mod api;
mod args;
mod audio;
mod binding;
mod cli;
mod config;
mod connect;
mod daemon;
mod dictation;
mod history;
mod hotkey;
mod models;
mod permissions;
mod readiness;
mod service;
mod settings;
mod speech_to_text;
mod state;
mod status;
#[cfg(test)]
mod test_support;
mod text_to_speech;

use args::{Cli, CommandType};
use banshee_common::error::BansheeError;
use clap::Parser;

use crate::config::Config;

#[tokio::main]
async fn main() -> Result<(), BansheeError> {
    let cli = Cli::parse();
    // Unwrapped only by the arms that read it: RPC works without a parseable
    // config, and the checklist diagnoses a broken one
    let config_result =
        Config::load().inspect_err(|error| eprintln!("Failed to load config: {error}"));

    match cli.command {
        CommandType::Serve => daemon::start(config_result?).await,
        CommandType::Stop => cli::stop().await,
        CommandType::Devices => cli::devices().await,
        CommandType::Voices => cli::voices().await,
        CommandType::Watch { waybar } => cli::watch(waybar).await,
        CommandType::Config {
            action: args::ConfigAction::Set { key, value },
        } => cli::config(key, value).await,
        CommandType::Setup => cli::setup(config_result).await,
        CommandType::Status { json } => cli::status(json, config_result).await,
        CommandType::Listen => cli::listen().await,
        CommandType::Speak { text } => cli::speak(text).await,
        CommandType::History => cli::history().await,
        CommandType::ClearHistory => cli::clear_history().await,
        CommandType::Record { action } => cli::record(action).await,
        CommandType::Start => cli::start(config_result),
        CommandType::Tray { uninstall } => cli::tray(uninstall),
        CommandType::Connect { agent, yes } => cli::connect(agent, yes),
        CommandType::Service { action } => cli::service(action),
    }
}
