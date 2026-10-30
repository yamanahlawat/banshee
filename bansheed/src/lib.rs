mod api;
mod args;
mod audio;
#[doc(hidden)]
pub mod bench;
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
mod level;
mod models;
mod on_path;
mod permissions;
mod readiness;
mod remote;
mod remote_probe;
mod service;
mod settings;
mod speech_to_text;
mod state;
mod status;
#[cfg(test)]
mod test_support;
mod text_to_speech;
mod turn_end;
mod turns;
mod uninstall;

use std::process::ExitCode;

use args::{Cli, CommandType};
use banshee_common::error::BansheeError;
use clap::Parser;

use crate::config::Config;

pub async fn run() -> ExitCode {
    banshee_common::logging::install("daemon");
    let cli = Cli::parse();
    // Unwrapped only by the arms that read it: RPC works without a parseable
    // config, and the checklist diagnoses a broken one
    let config_result = Config::load();

    let outcome = match cli.command {
        CommandType::Serve => {
            return exit_code(match config_result {
                Ok(config) => daemon::start(config).await,
                Err(error) => Err(error),
            });
        }
        CommandType::Stop => cli::stop().await,
        CommandType::Devices => cli::devices().await,
        CommandType::Voices => cli::voices().await,
        CommandType::Watch { waybar, events } => cli::watch(waybar, events).await,
        CommandType::Config {
            action: args::ConfigAction::Set { key, value },
        } => cli::config(key, value).await,
        CommandType::Config {
            action: args::ConfigAction::Remote,
        } => cli::config_remote().await,
        CommandType::Setup => cli::setup(config_result).await,
        CommandType::Status { json } => cli::status(json, config_result).await,
        CommandType::Speak { text } => cli::speak(text).await,
        CommandType::History => cli::history().await,
        CommandType::ClearHistory => cli::clear_history().await,
        CommandType::Record { action } => cli::record(action).await,
        CommandType::Start => cli::start(config_result).await,
        CommandType::Tray => cli::tray(),
        CommandType::Connect { agent, yes } => cli::connect(agent, yes),
        CommandType::Bind { compositor, yes } => cli::bind(compositor, yes, config_result).await,
        CommandType::Uninstall { data, yes } => cli::uninstall(data, yes).await,
        CommandType::TurnEnd { agent } => {
            turn_end::run(agent).await;
            Ok(())
        }
    };
    exit_code(outcome.map(|()| 0))
}

fn exit_code(outcome: Result<u8, BansheeError>) -> ExitCode {
    match outcome {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            banshee_common::logging::failure(&cli::failure_line(&error));
            ExitCode::FAILURE
        }
    }
}
