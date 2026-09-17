mod api;
mod args;
mod audio;
mod binding;
mod cli;
mod compositor;
mod config;
mod connect;
mod credentials;
mod daemon;
mod dictation;
mod history;
mod hotkey;
mod models;
mod permissions;
mod readiness;
mod remote;
mod remote_probe;
mod service;
mod settings;
mod speech_to_text;
mod state;
mod status;
mod tell;
#[cfg(test)]
mod test_support;
mod text_to_speech;

use std::process::ExitCode;

use args::{Cli, CommandType};
use clap::Parser;

use crate::config::Config;

#[tokio::main]
async fn main() -> ExitCode {
    banshee_common::logging::install();
    let cli = Cli::parse();
    // Unwrapped only by the arms that read it: RPC works without a parseable
    // config, and the checklist diagnoses a broken one
    let config_result = Config::load();

    let outcome = match cli.command {
        CommandType::Serve => match config_result {
            Ok(config) => daemon::start(config).await,
            Err(error) => Err(error),
        },
        CommandType::Stop => cli::stop().await,
        CommandType::Devices => cli::devices().await,
        CommandType::Voices => cli::voices().await,
        CommandType::Watch { waybar } => cli::watch(waybar).await,
        CommandType::Config {
            action: args::ConfigAction::Set { key, value },
        } => cli::config(key, value).await,
        CommandType::Config {
            action: args::ConfigAction::Remote,
        } => cli::config_remote().await,
        CommandType::Setup => cli::setup(config_result).await,
        CommandType::Status { json } => cli::status(json, config_result).await,
        CommandType::Listen => cli::listen().await,
        CommandType::Speak { text } => cli::speak(text).await,
        CommandType::Tell { text, undo } => cli::tell(text, undo, config_result),
        CommandType::History => cli::history().await,
        CommandType::ClearHistory => cli::clear_history().await,
        CommandType::Record { action } => cli::record(action).await,
        CommandType::Start => cli::start(config_result).await,
        CommandType::Tray { uninstall } => cli::tray(uninstall),
        CommandType::Connect { agent, yes } => cli::connect(agent, yes),
        CommandType::Bind { compositor, yes } => cli::bind(compositor, yes, config_result).await,
        CommandType::Service { action } => cli::service(action),
    };
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{}", cli::failure_line(&error));
            ExitCode::FAILURE
        }
    }
}
