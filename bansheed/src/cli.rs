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

    let mut failed = Vec::new();
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
        note_progress(&reported, &mut pending, &mut failed);
        show_progress(reported);
    }
    downloads_settled(&failed)
}

fn note_progress(
    reported: &banshee_common::DownloadProgress,
    pending: &mut usize,
    failed: &mut Vec<String>,
) {
    match reported.state {
        banshee_common::DownloadState::Downloading => {}
        banshee_common::DownloadState::Done => *pending -= 1,
        banshee_common::DownloadState::Failed => {
            *pending -= 1;
            failed.push(reported.model.clone());
        }
    }
}

fn downloads_settled(failed: &[String]) -> Result<(), BansheeError> {
    if failed.is_empty() {
        return Ok(());
    }
    Err(BansheeError::Rejected(format!(
        "{} failed to download; run: banshee setup",
        failed.join(", ")
    )))
}

fn progress_line(progress: &banshee_common::DownloadProgress) -> String {
    use banshee_common::DownloadState;
    match progress.state {
        DownloadState::Done => format!("{} downloaded", progress.model),
        DownloadState::Failed => format!("{} failed", progress.model),
        DownloadState::Downloading => {
            match models::download::percent(progress.bytes, progress.total) {
                Some(done) => format!(
                    "{}, {} of {}  {done}%",
                    progress.label, progress.index, progress.count
                ),
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
    banshee_common::Activity::of(state).word()
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

/// An answer meant for the caller reads on its own, a socket nobody answers
/// is the daemon being away, and anything else already says what it is.
pub fn failure_line(error: &BansheeError) -> String {
    match error {
        BansheeError::Rejected(_) | BansheeError::Rpc { .. } => error.rpc_message(),
        away if daemon_is_down(away) => format!("Could not reach the daemon: {away}"),
        other => other.to_string(),
    }
}

/// Asks the daemon and prints its reply as the caller can read it.
async fn show(method: &str, params: serde_json::Value) -> Result<(), BansheeError> {
    let result = utils::call_daemon(method, params).await?;
    println!(
        "{}",
        serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string())
    );
    Ok(())
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
            utils::socket_path().is_some_and(|path| !daemon::socket_answers(&path))
        }
        _ => false,
    }
}

pub async fn stop() -> Result<(), BansheeError> {
    match utils::call_daemon(banshee_common::BANSHEE_STOP, serde_json::json!({})).await {
        Ok(_) => println!("Daemon stopped."),
        Err(error) if daemon_is_down(&error) => println!("Daemon is not running."),
        Err(error) => return Err(error),
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
        Err(error) => return Err(error),
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
                .map(|id| text_to_speech::local::voices::describe(id, true))
                .collect(),
            None,
        ),
        Err(error) => return Err(error),
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
            Err(error) => return Err(error),
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
            Err(error) => return Err(error),
        };
    }
}

/// One line from stdin, `None` when it is empty.
pub(crate) fn ask_line(prompt: &str) -> Result<Option<String>, BansheeError> {
    eprint!("{prompt}");
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    let answer = line.trim().to_string();
    Ok((!answer.is_empty()).then_some(answer))
}

fn key_prompt(side: crate::credentials::RemoteKey) -> &'static str {
    match side {
        crate::credentials::RemoteKey::Stt => "Key for the remote listener (not shown): ",
        crate::credentials::RemoteKey::Tts => "Key for the remote speaker (not shown): ",
    }
}

// A pipe is read as one line, so `printf 'sk-…\n' | banshee config set tts.remote.api_key` works
fn ask_key(side: crate::credentials::RemoteKey) -> Result<String, BansheeError> {
    if std::io::IsTerminal::is_terminal(&std::io::stdin()) {
        Ok(rpassword::prompt_password(key_prompt(side))?)
    } else {
        Ok(ask_line("")?.unwrap_or_default())
    }
}

/// Answers whether a restart is needed.
async fn write_settings(assignments: settings::Assignments) -> Result<bool, BansheeError> {
    match utils::call_daemon(
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
        Err(error) if daemon_is_down(&error) => settings::configure(None, assignments, true)
            .map(|outcome| !outcome.restart_required.is_empty()),
        Err(error) => Err(error),
    }
}

/// What to write, or `None` for nothing typed. The key is not coerced as TOML,
/// because a coercion mangles a quoted token and refuses an all-digit one. An
/// empty argument removes the key, and the prompt has no way to ask for that.
fn key_change(
    argument: Option<String>,
    prompt: impl FnOnce() -> Result<String, BansheeError>,
) -> Result<Option<String>, BansheeError> {
    Ok(match argument {
        Some(argument) => Some(argument),
        None => match prompt()? {
            typed if typed.is_empty() => None,
            typed => Some(typed),
        },
    })
}

async fn config_api_key(
    side: crate::credentials::RemoteKey,
    change: Option<String>,
) -> Result<(), BansheeError> {
    let Some(key) = change else {
        println!("No key was typed; the key on file is unchanged.");
        return Ok(());
    };
    // The credentials file reads the empty string as the key being gone
    let done = if key.is_empty() { "Removed" } else { "Set" };
    let assignments = settings::Assignments::from([(side.setting().to_string(), key.into())]);
    match write_settings(assignments).await {
        Ok(restart_required) => {
            println!("{done} {}.", side.setting());
            if restart_required {
                println!("Restart to use it: banshee start");
            }
        }
        Err(error) => return Err(error),
    }
    Ok(())
}

pub async fn config(key: String, value: Option<String>) -> Result<(), BansheeError> {
    if let Some(side) = crate::credentials::RemoteKey::of_setting(&key) {
        return config_api_key(side, key_change(value, || ask_key(side))?).await;
    }
    let Some(value) = value else {
        return Err(BansheeError::Rejected(format!("'{key}' needs a value")));
    };
    // So `0.6` arrives as a number and `de` as a string
    let value: serde_json::Value =
        serde_json::from_str(&value).unwrap_or_else(|_| serde_json::Value::String(value.clone()));
    let assignments = settings::Assignments::from([(key.clone(), value)]);

    match write_settings(assignments).await {
        Ok(restart_required) => {
            println!("Set {key} in config.toml.");
            if restart_required {
                println!("Restart to use it: banshee start");
            }
        }
        Err(error) => return Err(error),
    }
    Ok(())
}

/// The speech endpoint has no default voice, so an empty voice cannot send
/// text out.
fn speaker_sends_text_out(voice: &str) -> bool {
    !voice.is_empty()
}

pub async fn config_remote() -> Result<(), BansheeError> {
    use crate::credentials::RemoteKey;

    let config = Config::load().unwrap_or_default();
    let listener = config.stt.remote;
    let speaker = config.tts.remote;

    // The server is asked first on each side: a key alone assumes OpenAI, and
    // the person may be calling Groq or a server of their own.
    println!("The listener: what hears your audio.");
    let stt_base_url = ask_line(&format!("Server /v1 root [{}]: ", listener.base_url))?
        .unwrap_or(listener.base_url);
    let stt_model = ask_line(&format!("Model [{}]: ", listener.model))?.unwrap_or(listener.model);
    let stt_key = key_change(None, || ask_key(RemoteKey::Stt))?;

    println!();
    println!("The speaker: what says Banshee's replies.");
    let tts_base_url =
        ask_line(&format!("Server /v1 root [{}]: ", speaker.base_url))?.unwrap_or(speaker.base_url);
    let tts_model = ask_line(&format!("Model [{}]: ", speaker.model))?.unwrap_or(speaker.model);
    let tts_voice = ask_line(&format!("Voice [{}]: ", speaker.voice))?.unwrap_or(speaker.voice);
    let tts_key = key_change(None, || ask_key(RemoteKey::Tts))?;

    let mut assignments = settings::Assignments::from([
        ("stt.provider".to_string(), "remote".into()),
        (
            "stt.remote.base_url".to_string(),
            stt_base_url.clone().into(),
        ),
        ("stt.remote.model".to_string(), stt_model.clone().into()),
        (
            "tts.remote.base_url".to_string(),
            tts_base_url.clone().into(),
        ),
        ("tts.remote.model".to_string(), tts_model.clone().into()),
        ("tts.remote.voice".to_string(), tts_voice.clone().into()),
    ]);
    let tts_provider = if speaker_sends_text_out(&tts_voice) {
        "remote"
    } else {
        "local"
    };
    assignments.insert("tts.provider".to_string(), tts_provider.into());
    if let Some(key) = stt_key {
        assignments.insert(RemoteKey::Stt.setting().to_string(), key.into());
    }
    if let Some(key) = tts_key {
        assignments.insert(RemoteKey::Tts.setting().to_string(), key.into());
    }

    match write_settings(assignments).await {
        Ok(_) => {
            println!();
            println!(
                "Listening through {} with {stt_model}.",
                crate::config::host_of(&stt_base_url)
            );
            report_key(RemoteKey::Stt);
            // The prompt above asks for the speaker's key either way; only a
            // speaker that sends text out reports it.
            if speaker_sends_text_out(&tts_voice) {
                println!(
                    "Speaking through {} as {tts_voice} with {tts_model}.",
                    crate::config::host_of(&tts_base_url)
                );
                report_key(RemoteKey::Tts);
            } else {
                println!("The speaker stays on this machine: name a voice to send text out.");
            }
            println!("Restart to use it: banshee start");
        }
        Err(error) => return Err(error),
    }
    Ok(())
}

fn report_key(side: crate::credentials::RemoteKey) {
    if crate::credentials::Credentials::present(side) {
        println!("The {} key is set.", side.side());
    } else {
        println!(
            "No key for the remote {} yet: banshee config set {}",
            side.side(),
            side.setting()
        );
    }
}

pub async fn setup(config_result: Result<Config, BansheeError>) -> Result<(), BansheeError> {
    download_missing(config_result.as_ref().ok()).await
}

/// Fetches the models that are not on disk, through the daemon when one runs.
/// The config is required only when no daemon runs.
pub async fn download_missing(config: Option<&Config>) -> Result<(), BansheeError> {
    // Subscribed before the download is asked for, so the first
    // notifications are not lost in the gap
    let watching = utils::Subscription::open(&[banshee_common::EVENT_DOWNLOADS]).await;
    match watching {
        Ok((_, subscription)) => {
            follow_daemon_download(subscription).await?;
        }
        Err(error) if daemon_is_down(&error) => {
            // No daemon answers, and the daemon never downloads unasked, so this process is the only writer.
            let config = config.ok_or_else(|| {
                BansheeError::Rejected(
                    "the config did not load, so the models to fetch are unknown; fix config.toml and run: banshee setup"
                        .to_string(),
                )
            })?;
            let dir = models::download::models_dir()?;
            let missing = models::download::still_missing(&models::download::wanted(config), &dir);
            if missing.is_empty() {
                println!("Everything is already downloaded.");
                return Ok(());
            }
            // Printed here: `failure_line` reads a missing file as the daemon
            // being away, and this one is the model's
            if let Err(error) =
                models::download::download_all(&dir, &missing, &mut show_progress).await
            {
                eprintln!("{error}");
                std::process::exit(1);
            }
        }
        Err(error) => return Err(error),
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
        status::Daemon::Running { status, .. } => status,
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
    show(
        banshee_common::BANSHEE_GET_TRANSCRIPTION,
        serde_json::json!({}),
    )
    .await
}

pub async fn speak(text: String) -> Result<(), BansheeError> {
    show(
        banshee_common::BANSHEE_SPEAK,
        serde_json::json!({ "text": text }),
    )
    .await
}

/// Prints what the agent wrote. A refused `speak_status` leaves this print as
/// the only thing the user gets.
pub fn tell(
    text: Option<String>,
    undo: bool,
    config: Result<Config, BansheeError>,
) -> Result<(), BansheeError> {
    let config = config?;
    if undo {
        println!("{}", crate::tell::undo(&config.tell)?);
        return Ok(());
    }
    let text =
        text.ok_or_else(|| BansheeError::Rejected("say what to tell it, or pass --undo".into()))?;
    let told = crate::tell::run(&text, &config.tell, &|line| println!("{line}"))?;
    for warning in &told.warnings {
        eprintln!("{}", warning.text());
    }
    if let Some(reply) = told.reply {
        println!("{reply}");
    }
    Ok(())
}

pub async fn history() -> Result<(), BansheeError> {
    show(banshee_common::BANSHEE_HISTORY, serde_json::json!({})).await
}

pub async fn clear_history() -> Result<(), BansheeError> {
    show(banshee_common::BANSHEE_CLEAR_HISTORY, serde_json::json!({})).await
}

pub async fn record(action: args::RecordAction) -> Result<(), BansheeError> {
    let (method, params) = match action {
        args::RecordAction::Start { dictate, tell } => (
            banshee_common::BANSHEE_RECORD_START,
            serde_json::json!({ "dictate": dictate, "tell": tell }),
        ),
        args::RecordAction::Stop => (banshee_common::BANSHEE_RECORD_STOP, serde_json::json!({})),
        args::RecordAction::Toggle { dictate, tell } => (
            banshee_common::BANSHEE_RECORD_TOGGLE,
            serde_json::json!({ "dictate": dictate, "tell": tell }),
        ),
    };
    utils::call_daemon(method, params).await?;
    Ok(())
}

pub async fn start(config_result: Result<Config, BansheeError>) -> Result<(), BansheeError> {
    let log = service::install(service::Agent::Daemon)?;
    println!("Banshee is running, and starts again at login.");

    // The daemon reports these to its log, which nobody reads on a first run
    let mut blocked = false;
    let binding = match &config_result {
        Ok(config) => {
            let missing = models::missing(&models::required(config));
            if !missing.is_empty() {
                println!();
                println!(
                    "Downloading the models it needs (~860 MB): {}.",
                    missing.join(", ")
                );
                println!("Ctrl-C leaves the daemon running; banshee setup resumes the download.");
                download_missing(Some(config)).await?;
                println!("Restarting the daemon so it loads the models.");
                // The daemon builds its pipeline once at start, so the models it lacked need a restart to load.
                service::install(service::Agent::Daemon)?;
            }
            // A remote listener downloads nothing, so the models say nothing
            // about whether it can hear.
            let listener = crate::credentials::RemoteKey::Stt;
            if config.stt.provider.is_remote()
                && !crate::credentials::Credentials::present(listener)
            {
                blocked = true;
                println!();
                println!(
                    "No key for the remote {}: banshee config set {}",
                    listener.side(),
                    listener.setting()
                );
            }
            Some((config.audio.hotkey, config.audio.hotkey_mode))
        }
        Err(error) => {
            eprintln!("Failed to load config: {error}");
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

pub async fn bind(
    compositor: Option<args::CompositorName>,
    yes: bool,
    config_result: Result<Config, BansheeError>,
) -> Result<(), BansheeError> {
    let audio = match config_result {
        Ok(config) => config.audio,
        // Printing the block, or the macOS answer, saves nothing
        Err(_) if compositor.is_none() || cfg!(target_os = "macos") => {
            crate::config::AudioConfig::default()
        }
        Err(error) => return Err(error),
    };
    let (hotkey, mode) = crate::compositor::run(compositor, yes, audio.hotkey, audio.hotkey_mode)
        .unwrap_or_else(|error| {
            eprintln!("{error}");
            std::process::exit(1)
        });
    let mut assignments = settings::Assignments::new();
    if hotkey != audio.hotkey {
        assignments.insert("audio.hotkey".to_string(), serde_json::json!(hotkey));
    }
    if mode != audio.hotkey_mode {
        assignments.insert("audio.hotkey_mode".to_string(), serde_json::json!(mode));
    }
    if assignments.is_empty() {
        return Ok(());
    }
    let names = assignments
        .keys()
        .cloned()
        .collect::<Vec<_>>()
        .join(" and ");
    if let Err(error) = write_settings(assignments).await {
        eprintln!("Hyprland is bound, but {names} could not be saved to config.toml: {error}");
        std::process::exit(1);
    }
    println!("Set {names} in config.toml.");
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
