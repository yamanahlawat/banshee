use std::io::{IsTerminal, Write};

use banshee_common::{Voice, error::BansheeError, utils};

use crate::{
    args, audio, config::Config, connect, daemon, hotkey, models, permissions, service, settings,
    status, text_to_speech,
};

fn show_progress(progress: banshee_common::DownloadProgress) {
    // Rewritten in place, so a percentage does not scroll the screen away
    let ending = if progress.state == banshee_common::DownloadState::Downloading {
        '\r'
    } else {
        '\n'
    };
    // Erase to end of line, so a shorter line does not leave the tail of the
    // longer one behind it. A redirected stdout takes neither the erase nor the
    // carriage return: both reach a log file as themselves.
    let (erase, ending) = if std::io::stdout().is_terminal() {
        ("\x1b[K", ending)
    } else {
        ("", '\n')
    };
    print!("{}{erase}{ending}", progress_line(&progress));
    let _ = std::io::stdout().flush();
}

// One writer at a time: the `.part` file that makes resume possible has a
// stable name, so this process must not fetch alongside a daemon already doing it
async fn follow_daemon_download(mut progress: utils::Subscription) -> Result<(), BansheeError> {
    let reply = utils::call_daemon(
        banshee_common::BANSHEE_DOWNLOAD_MODELS,
        serde_json::json!({}),
    )
    .await?;
    // The counter terminates because only one download runs at a time, so every
    // notification on this connection belongs to the batch just asked for
    let mut pending: usize = reply
        .get("downloading")
        .and_then(|names| names.as_array())
        .map_or(0, Vec::len);
    if pending == 0 {
        println!("Everything is already downloaded.");
        return Ok(());
    }

    while pending > 0 {
        let Some(params) = progress
            .next_of(banshee_common::BANSHEE_DOWNLOAD_PROGRESS)
            .await?
        else {
            return Err(BansheeError::Other(
                "The daemon stopped before the download finished".to_string(),
            ));
        };
        let reported: banshee_common::DownloadProgress = serde_json::from_value(params)?;
        let done = reported.state != banshee_common::DownloadState::Downloading;
        show_progress(reported);
        if done {
            pending -= 1;
        }
    }
    Ok(())
}

fn progress_line(progress: &banshee_common::DownloadProgress) -> String {
    use banshee_common::DownloadState;
    match progress.state {
        DownloadState::Done => format!("{} downloaded", progress.model),
        DownloadState::Failed => format!("{} failed", progress.model),
        DownloadState::Downloading => {
            match models::download::percent(progress.bytes, progress.total) {
                // A daemon older than this field set sends no count at all,
                // and a real run always has count >= 1, so 0 marks a message
                // with no place to report
                Some(done) if progress.count > 0 => format!(
                    "{}, {} of {}  {done}%",
                    progress.label, progress.index, progress.count
                ),
                Some(done) => format!("{} {done}%", progress.model),
                // No Content-Length, so there is no bar to draw: count what arrived
                None => format!("{} {} MB", progress.model, progress.bytes / 1_048_576),
            }
        }
    }
}

// Defaulting to empty would report a newer daemon's reply as nothing at all
fn decoded<T: serde::de::DeserializeOwned>(reply: &serde_json::Value, key: &str) -> T {
    match serde_json::from_value(reply.get(key).cloned().unwrap_or_default()) {
        Ok(value) => value,
        Err(error) => {
            eprintln!("The daemon sent a reply this build cannot read: {error}");
            std::process::exit(1);
        }
    }
}

// One object per line, as Waybar requires. The same word lands in three keys
// because Waybar splits them: `text` shows, `alt` picks a format-icon, `class`
// picks CSS. Readiness is left out: blockers are answered once and never
// pushed, so a bar would show them stale.
fn waybar_line(word: &str, device: Option<&str>, missing: Option<&str>) -> String {
    let tooltip = match (device, missing) {
        // Nothing to say about the microphone, so the word stands alone
        (None, None) => format!("Banshee is {word}"),
        _ => format!(
            "Banshee is {word}. Microphone: {}",
            banshee_common::microphone_label(device, missing)
        ),
    };
    serde_json::json!({
        "text": word,
        "alt": word,
        "class": word,
        "tooltip": tooltip,
    })
    .to_string()
}

fn watch_line(waybar: bool, word: &str, device: Option<&str>, missing: Option<&str>) -> String {
    if waybar {
        waybar_line(word, device, missing)
    } else {
        word.to_string()
    }
}

fn state_word(state: &serde_json::Value) -> &'static str {
    match banshee_common::Activity::of(state) {
        banshee_common::Activity::Idle => "idle",
        banshee_common::Activity::Recording => "recording",
        banshee_common::Activity::Speaking => "speaking",
        banshee_common::Activity::Listening => "listening",
    }
}

/// A device can be both, and hiding either label would read as its being false.
fn device_labels(device: &banshee_common::InputDevice, current: Option<&str>) -> String {
    let mut labels = Vec::new();
    if device.default {
        labels.push("system default");
    }
    if current == Some(device.name.as_str()) {
        labels.push("in use");
    }
    labels.join(", ")
}

/// An answer meant for the caller reads on its own; anything else is this
/// process failing to ask.
fn fail(error: &BansheeError) -> ! {
    match error {
        BansheeError::Rejected(_) | BansheeError::Rpc { .. } => {
            eprintln!("{}", error.rpc_message())
        }
        other => eprintln!("Could not reach the daemon: {other}"),
    }
    std::process::exit(1)
}

fn daemon_is_down(error: &BansheeError) -> bool {
    match error {
        BansheeError::Io(io) => matches!(
            io.kind(),
            std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
        ),
        // A socket orphaned by an unclean exit accepts the connection then
        // closes it. tokio's nonblocking connect cannot tell; a blocking one can.
        BansheeError::Serde(_) => {
            utils::get_socket_path().is_some_and(|path| !daemon::socket_answers(&path))
        }
        _ => false,
    }
}

pub async fn stop() -> Result<(), BansheeError> {
    match utils::call_daemon(banshee_common::BANSHEE_STOP, serde_json::json!({})).await {
        Ok(_) => println!("Daemon stopped."),
        Err(error) if daemon_is_down(&error) => println!("Daemon is not running."),
        Err(error) => eprintln!("Failed to stop daemon: {error}"),
    }
    Ok(())
}

pub async fn devices() -> Result<(), BansheeError> {
    let (devices, current) = match utils::call_daemon(
        banshee_common::BANSHEE_LIST_INPUT_DEVICES,
        serde_json::json!({}),
    )
    .await
    {
        Ok(reply) => {
            let devices: Vec<banshee_common::InputDevice> = decoded(&reply, "devices");
            let current = reply
                .get("current")
                .and_then(|name| name.as_str())
                .map(str::to_string);
            (devices, current)
        }
        // You need the names before you can start a daemon on the right one
        Err(error) if daemon_is_down(&error) => (audio::input_devices(), None),
        Err(error) => {
            fail(&error);
        }
    };

    if devices.is_empty() {
        println!("No microphones found.");
        return Ok(());
    }
    let width = devices
        .iter()
        .map(|d| d.name.chars().count())
        .max()
        .unwrap_or(0);
    for device in &devices {
        let labels = device_labels(device, current.as_deref());
        if labels.is_empty() {
            println!("  {}", device.name);
        } else {
            println!("  {:width$}  {labels}", device.name);
        }
    }
    println!();
    println!("Record from one with: banshee config set audio.input_device \"<name>\"");
    Ok(())
}

pub async fn voices() -> Result<(), BansheeError> {
    let (voices, current) = match utils::call_daemon(
        banshee_common::BANSHEE_LIST_VOICES,
        serde_json::json!({}),
    )
    .await
    {
        Ok(reply) => (
            decoded::<Vec<Voice>>(&reply, "voices"),
            reply
                .get("current")
                .and_then(|voice| voice.as_str())
                .map(str::to_string),
        ),
        // A voice gets chosen before there is a daemon to ask. These
        // came off the disk, so every one of them is here.
        Err(error) if daemon_is_down(&error) => (
            models::installed_voices()
                .iter()
                .map(|id| text_to_speech::voices::describe(id, true))
                .collect(),
            None,
        ),
        Err(error) => fail(&error),
    };

    // The daemon names every voice it can describe, so what this has to
    // print is the ones that are here: `banshee voices` promises that
    // every name it lists works today.
    let held: Vec<&Voice> = voices.iter().filter(|voice| voice.downloaded).collect();
    if held.is_empty() {
        println!("No voices found. Download one with: banshee setup");
        return Ok(());
    }
    for voice in held {
        let marker = if current.as_deref() == Some(voice.id.as_str()) {
            "*"
        } else {
            " "
        };
        println!(
            "{marker} {}  {}  ({})",
            voice.name, voice.description, voice.id
        );
    }
    println!();
    println!("Speak with one by: banshee config set tts.voice \"<id>\"");
    Ok(())
}

pub async fn watch(waybar: bool) -> Result<(), BansheeError> {
    let (mut state, mut changes) =
        match utils::Subscription::open(&[banshee_common::EVENT_STATE]).await {
            Ok(subscription) => subscription,
            Err(error) if daemon_is_down(&error) => {
                eprintln!("Daemon is not running.");
                std::process::exit(1);
            }
            Err(error) => fail(&error),
        };
    // No real line is empty, so the first one always prints
    let mut shown = String::new();
    loop {
        let line = watch_line(
            waybar,
            state_word(&state),
            banshee_common::audio_device(&state),
            banshee_common::missing_device(&state),
        );
        // The reader sees the line, so the line is what has to differ
        if line != shown {
            // `banshee watch | head` closes the pipe. That is the reader
            // having seen enough, not this command failing
            if writeln!(std::io::stdout(), "{line}").is_err() {
                return Ok(());
            }
            shown = line;
        }
        state = match changes.next_of(banshee_common::BANSHEE_STATE_CHANGED).await {
            Ok(Some(params)) => params,
            // There is no other clean end, so a supervisor can read the
            // exit code as one
            Ok(None) => {
                eprintln!("The daemon closed the connection.");
                std::process::exit(1);
            }
            Err(error) => fail(&error),
        };
    }
}

pub async fn config(key: String, value: String) -> Result<(), BansheeError> {
    // So `0.6` arrives as a number and `de` as a string
    let value: serde_json::Value =
        serde_json::from_str(&value).unwrap_or_else(|_| serde_json::Value::String(value.clone()));
    let assignments = settings::Assignments::from([(key.clone(), value)]);

    let outcome = match utils::call_daemon(
        banshee_common::BANSHEE_CONFIGURE,
        serde_json::json!({ "settings": &assignments, "persist": true }),
    )
    .await
    {
        Ok(reply) => Ok(reply
            .get("restart_required")
            .and_then(|keys| keys.as_array())
            .is_some_and(|keys| !keys.is_empty())),
        // A daemon that is down never writes, so the CLI can be the one writer
        Err(error) if daemon_is_down(&error) => settings::configure(None, &assignments, true)
            .map(|outcome| !outcome.restart_required.is_empty()),
        Err(error) => Err(error),
    };

    match outcome {
        Ok(restart_required) => {
            println!("Set {key} in config.toml.");
            if restart_required {
                println!("Restart to use it: banshee start");
            }
        }
        Err(error) => {
            fail(&error);
        }
    }
    Ok(())
}

pub async fn setup(config_result: Result<Config, BansheeError>) -> Result<(), BansheeError> {
    // Subscribed before the download is asked for, so the first
    // notifications are not lost in the gap
    let watching = utils::Subscription::open(&[banshee_common::EVENT_DOWNLOADS]).await;
    match watching {
        Ok((_, subscription)) => {
            if let Err(error) = follow_daemon_download(subscription).await {
                fail(&error);
            }
        }
        Err(error) if daemon_is_down(&error) => {
            // No daemon, so this process is the only writer there can be
            let config = config_result?;
            let dir = models::download::models_dir()?;
            let missing = models::download::still_missing(&models::download::wanted(&config), &dir);
            if missing.is_empty() {
                println!("Everything is already downloaded.");
                return Ok(());
            }
            // Not `fail`: nothing was asked of a daemon here, so the
            // reason stands on its own
            if let Err(error) =
                models::download::download_all(&dir, &missing, &mut show_progress).await
            {
                eprintln!("{error}");
                std::process::exit(1);
            }
        }
        Err(error) => fail(&error),
    }
    Ok(())
}

pub async fn status(
    json: bool,
    config_result: Result<Config, BansheeError>,
) -> Result<(), BansheeError> {
    if !json {
        if !status::run(config_result).await {
            std::process::exit(1);
        }
        return Ok(());
    }
    // The same probe the checklist uses, so the two halves of one command
    // cannot disagree about whether a daemon is up
    let reply = match status::probe_daemon().await {
        status::Daemon::Running { status, .. } | status::Daemon::Legacy(status) => status,
        _ => {
            println!("{}", serde_json::json!({"running": false}));
            std::process::exit(1);
        }
    };
    println!(
        "{}",
        serde_json::to_string_pretty(&reply).unwrap_or_else(|_| reply.to_string())
    );
    // The checklist's verdict, and absent leaves the code alone
    if reply.get("ready") == Some(&serde_json::Value::Bool(false)) {
        std::process::exit(1);
    }
    Ok(())
}

pub async fn listen() -> Result<(), BansheeError> {
    match utils::call_daemon(
        banshee_common::BANSHEE_GET_TRANSCRIPTION,
        serde_json::json!({}),
    )
    .await
    {
        Ok(result) => println!(
            "{}",
            serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string())
        ),
        Err(error) => eprintln!("Failed to get transcription: {error}"),
    }
    Ok(())
}

pub async fn speak(text: String) -> Result<(), BansheeError> {
    match utils::call_daemon(
        banshee_common::BANSHEE_SPEAK,
        serde_json::json!({ "text": text }),
    )
    .await
    {
        Ok(result) => println!(
            "{}",
            serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string())
        ),
        Err(error) => eprintln!("Failed to send speak command: {error}"),
    }
    Ok(())
}

pub async fn history() -> Result<(), BansheeError> {
    match utils::call_daemon(banshee_common::BANSHEE_HISTORY, serde_json::json!({})).await {
        Ok(result) => println!(
            "{}",
            serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string())
        ),
        Err(error) => eprintln!("Failed to get history: {error}"),
    }
    Ok(())
}

pub async fn clear_history() -> Result<(), BansheeError> {
    match utils::call_daemon(banshee_common::BANSHEE_CLEAR_HISTORY, serde_json::json!({})).await {
        Ok(result) => println!(
            "{}",
            serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string())
        ),
        Err(error) => eprintln!("Failed to clear history: {error}"),
    }
    Ok(())
}

pub async fn record(action: args::RecordAction) -> Result<(), BansheeError> {
    let (method, params) = match action {
        args::RecordAction::Start { dictate } => (
            banshee_common::BANSHEE_RECORD_START,
            serde_json::json!({ "dictate": dictate }),
        ),
        args::RecordAction::Stop => (banshee_common::BANSHEE_RECORD_STOP, serde_json::json!({})),
    };
    if let Err(error) = utils::call_daemon(method, params).await {
        eprintln!("Failed to send record command: {error}");
    }
    Ok(())
}

pub fn start(config_result: Result<Config, BansheeError>) -> Result<(), BansheeError> {
    let log = service::install(service::Agent::Daemon)?;
    println!("Banshee is running, and starts again at login.");

    // The daemon reports these to its log, which nobody reads on a first run
    let mut blocked = false;
    let binding = match &config_result {
        Ok(config) => {
            let missing = models::missing(&models::required(config));
            if !missing.is_empty() {
                blocked = true;
                println!();
                println!("Models not downloaded yet: {}.", missing.join(", "));
                println!("It runs without them, but cannot record. Run: banshee setup");
            }
            Some((config.audio.hotkey, config.audio.hotkey_mode))
        }
        // main already printed why the config would not load
        Err(_) => {
            blocked = true;
            None
        }
    };

    permissions::grant_note();

    match binding {
        Some((hotkey, mode)) if !blocked => {
            println!();
            println!("{}", hotkey::usage_hint(hotkey, mode));
        }
        _ => println!("Logs: {log}"),
    }
    Ok(())
}

pub fn tray(uninstall: bool) -> Result<(), BansheeError> {
    if uninstall {
        if service::uninstall(service::Agent::Tray)? {
            println!("The menu bar icon no longer starts at login.");
        } else {
            println!("The menu bar icon was not set to start at login.");
        }
    } else {
        let log = service::install(service::Agent::Tray)?;
        println!("The menu bar icon is running, and comes back at login.");
        println!("Logs: {log}");
    }
    Ok(())
}

pub fn connect(agent: Option<args::AgentName>, yes: bool) -> Result<(), BansheeError> {
    if let Err(error) = connect::run(agent.map(Into::into), yes) {
        eprintln!("{error}");
        std::process::exit(1);
    }
    Ok(())
}

pub fn service(action: args::ServiceAction) -> Result<(), BansheeError> {
    match action {
        // Every entry, so none is left behind to fail at the next login
        args::ServiceAction::Uninstall => {
            let mut removed = false;
            for agent in service::Agent::ALL {
                if service::uninstall(agent)? {
                    println!("The {} no longer starts at login.", agent.name());
                    removed = true;
                }
            }
            if !removed {
                println!("Nothing was set to start at login.");
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
