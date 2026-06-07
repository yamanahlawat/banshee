mod api;
mod args;
mod audio;
mod daemon;
mod hotkey;

use args::{Cli, CommandType};
use clap::Parser;

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    match cli.command {
        CommandType::Serve => {
            let (_stream, consumer) =
                audio::start_audio_capture().expect("Failed to start audio capture");
            hotkey::hotkey_listener(consumer);
            if let Err(error) = daemon::run().await {
                eprintln!("Daemon crashed {error}")
            }
        }
        CommandType::Setup => {
            println!("Download models offline!");
        }
        CommandType::Status => {
            println!("Querying the running daemon!");
        }
        CommandType::Listen => {
            println!("Getting latest transcription!");
        }
        CommandType::Speak { text } => {
            println!("Telling the daemon to speak {text}");
        }
    }
}
