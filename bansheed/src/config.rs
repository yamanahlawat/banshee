use std::path::{Path, PathBuf};

use banshee_common::{error::BansheeError, utils::get_config_path};
use serde::{Deserialize, Deserializer, Serialize};

// Every section denies unknown fields: TOML binds a key to whatever table
// precedes it, so a misplaced setting parses fine and silently does nothing.

#[derive(Deserialize, Serialize, Debug)]
#[serde(default, deny_unknown_fields)]
pub struct DaemonConfig {
    pub always_on: bool,
    pub save_history: bool,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            always_on: true,
            save_history: true,
        }
    }
}

#[derive(Deserialize, Serialize, Debug, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum HotkeyMode {
    Hold,
    Toggle,
}

#[derive(Deserialize, Serialize, Debug, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum BargeInMode {
    Stop,
    Duck,
    None,
}

#[derive(Deserialize, Serialize, Debug)]
#[serde(default, deny_unknown_fields)]
pub struct AudioCuesConfig {
    pub enabled: bool,
    pub start: Option<PathBuf>,
    pub stop: Option<PathBuf>,
    pub ready: Option<PathBuf>,
    pub error: Option<PathBuf>,
}

impl Default for AudioCuesConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            start: None,
            stop: None,
            ready: None,
            error: None,
        }
    }
}

#[derive(Deserialize, Serialize, Debug)]
#[serde(default, deny_unknown_fields)]
pub struct AudioConfig {
    pub input_device: String,
    pub hotkey: crate::binding::Hotkey,
    pub hotkey_mode: HotkeyMode,
    pub barge_in: BargeInMode,
    pub cues: AudioCuesConfig,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            input_device: crate::audio::DEFAULT_INPUT_DEVICE.to_string(),
            hotkey: crate::binding::Hotkey::default(),
            hotkey_mode: HotkeyMode::Hold,
            barge_in: BargeInMode::Stop,
            cues: AudioCuesConfig::default(),
        }
    }
}

#[derive(Deserialize, Serialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum STTPreset {
    Fast,
    Balanced,
    Quality,
}

impl STTPreset {
    pub const ALL: [STTPreset; 3] = [STTPreset::Fast, STTPreset::Balanced, STTPreset::Quality];

    pub fn model_name(&self) -> &'static str {
        match self {
            STTPreset::Fast => "ggml-base.en.bin",
            STTPreset::Balanced => "ggml-large-v3-turbo-q5_0.bin",
            STTPreset::Quality => "ggml-large-v3-q5_0.bin",
        }
    }
}

fn language<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    let value = String::deserialize(deserializer)?;
    Ok(spoken_or_english(value))
}

/// The engine's table plus auto, the config's own word for detect it; read and write both ask this.
pub fn known_language(value: &str) -> bool {
    value == "auto" || whisper_rs::get_lang_id(value).is_some()
}

/// A code Whisper does not know, read as English rather than refused. Nothing
/// read this field before, so a config written then can hold anything, and a
/// daemon that exits on it is a daemon launchd restarts for ever. `banshee
/// config set` refuses the same value at the boundary, where a person is there
/// to read why.
fn spoken_or_english(value: String) -> String {
    if known_language(&value) {
        return value;
    }
    eprintln!("banshee: '{value}' is not a language Whisper knows, so English is read instead");
    "en".to_string()
}

// Out of range no probability ever matches, so VAD stops firing with no error
fn probability<'de, D: Deserializer<'de>>(deserializer: D) -> Result<f32, D::Error> {
    let value = f32::deserialize(deserializer)?;
    if !(0.0..=1.0).contains(&value) {
        return Err(serde::de::Error::custom(format!(
            "must be between 0.0 and 1.0, got {value}"
        )));
    }
    Ok(value)
}

// Below 0.5 an utterance drags, above 2.0 it slurs. The window's slider offers
// this range, and the file has to refuse what the slider cannot ask for.
fn rate<'de, D: Deserializer<'de>>(deserializer: D) -> Result<f32, D::Error> {
    let value = f32::deserialize(deserializer)?;
    if !(0.5..=2.0).contains(&value) {
        return Err(serde::de::Error::custom(format!(
            "must be between 0.5 and 2.0, got {value}"
        )));
    }
    Ok(value)
}

#[derive(Deserialize, Serialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SttProvider {
    Local,
    Remote,
}

impl SttProvider {
    pub fn is_remote(self) -> bool {
        match self {
            SttProvider::Local => false,
            SttProvider::Remote => true,
        }
    }
}

// The message names the dotted key, because the same refusal reaches the window
// as a toast, where no line and column are shown.
fn non_empty<'de, D: Deserializer<'de>>(deserializer: D, key: &str) -> Result<String, D::Error> {
    let value = String::deserialize(deserializer)?;
    if value.is_empty() {
        return Err(serde::de::Error::custom(format!("{key} needs a value")));
    }
    Ok(value)
}

fn remote_base_url<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    non_empty(deserializer, "stt.remote.base_url")
}

fn remote_model<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    non_empty(deserializer, "stt.remote.model")
}

fn remote_tts_base_url<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    non_empty(deserializer, "tts.remote.base_url")
}

fn remote_tts_model<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    non_empty(deserializer, "tts.remote.model")
}

/// Where `[stt] provider = "remote"` sends the audio. The key is not here: it
/// lives in the credentials file, and `api_key` in this table is refused.
#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct RemoteSttConfig {
    /// The `/v1` root, as in `https://api.openai.com/v1`.
    #[serde(deserialize_with = "remote_base_url")]
    pub base_url: String,
    #[serde(deserialize_with = "remote_model")]
    pub model: String,
}

impl Default for RemoteSttConfig {
    fn default() -> Self {
        Self {
            base_url: "https://api.openai.com/v1".to_string(),
            model: "whisper-1".to_string(),
        }
    }
}

/// The host a person recognises in "audio goes to api.openai.com", and nothing
/// else the URL carries. Empty when the string is no URL, because a name is
/// worth having only if it is the real one.
pub fn host_of(base_url: &str) -> String {
    reqwest::Url::parse(base_url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_string))
        .unwrap_or_default()
}

impl RemoteSttConfig {
    pub fn host(&self) -> String {
        host_of(&self.base_url)
    }
}

/// Which side's key the document carries.
fn api_key_setting(text: &str) -> &'static str {
    // Read again as plain TOML, where no unknown field is refused and no value
    // is quoted back, because the parse error names the field and not the table
    // above it
    let Ok(document) = text.parse::<toml::Table>() else {
        return "stt.remote.api_key";
    };
    let carries = |side: &str| {
        document
            .get(side)
            .and_then(toml::Value::as_table)
            .is_some_and(|table| {
                table.contains_key("api_key")
                    || table
                        .get("remote")
                        .and_then(toml::Value::as_table)
                        .is_some_and(|remote| remote.contains_key("api_key"))
            })
    };
    if carries("tts") && !carries("stt") {
        "tts.remote.api_key"
    } else {
        "stt.remote.api_key"
    }
}

fn api_key_refusal(setting: &str) -> String {
    format!("api_key does not belong in config.toml; set it with: banshee config set {setting}")
}

#[derive(Deserialize, Serialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TtsProvider {
    Local,
    Remote,
}

impl TtsProvider {
    pub fn is_remote(self) -> bool {
        match self {
            TtsProvider::Local => false,
            TtsProvider::Remote => true,
        }
    }
}

/// What the remote speaker asks the server to send. These two are what it can
/// decode, so a third value is refused where a person can read the refusal.
#[derive(Deserialize, Serialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum SpeechFormat {
    #[default]
    Wav,
    Pcm,
}

impl std::fmt::Display for SpeechFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SpeechFormat::Wav => f.write_str("wav"),
            SpeechFormat::Pcm => f.write_str("pcm"),
        }
    }
}

/// Where `[tts] provider = "remote"` sends the text. The key is not here: it
/// lives in the credentials file, and `api_key` in this table is refused.
#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct RemoteTtsConfig {
    /// The `/v1` root, as in `https://api.openai.com/v1`.
    #[serde(deserialize_with = "remote_tts_base_url")]
    pub base_url: String,
    #[serde(deserialize_with = "remote_tts_model")]
    pub model: String,
    /// Empty until the user names one. The remote speaker refuses to start
    /// without it, because the endpoint has no call that lists voices.
    pub voice: String,
    /// Tone and delivery, for a model that reads it.
    pub instructions: String,
    /// What the server is asked for. WAV states its own rate, so it is the one
    /// format that plays right on a server Banshee knows nothing about.
    pub response_format: SpeechFormat,
    /// Under `pcm` this is also the rate the samples are read at, because raw
    /// samples say nothing about themselves.
    pub sample_rate: Option<std::num::NonZero<u32>>,
}

impl Default for RemoteTtsConfig {
    fn default() -> Self {
        Self {
            base_url: "https://api.openai.com/v1".to_string(),
            model: "tts-1".to_string(),
            voice: String::new(),
            instructions: String::new(),
            response_format: SpeechFormat::Wav,
            sample_rate: None,
        }
    }
}

impl RemoteTtsConfig {
    pub fn host(&self) -> String {
        host_of(&self.base_url)
    }
}

#[derive(Deserialize, Serialize, Debug)]
#[serde(default, deny_unknown_fields)]
pub struct STTConfig {
    pub provider: SttProvider,
    pub remote: RemoteSttConfig,
    pub preset: STTPreset,
    /// A Whisper language code, or `auto` to detect it. The English-only build
    /// holds no other language, so `preset = "fast"` reads English whatever
    /// this says.
    #[serde(deserialize_with = "language")]
    pub language: String,
    pub translate: bool,
    #[serde(deserialize_with = "probability")]
    pub vad_threshold: f32,
    pub vocabulary: Vec<String>,
    // Trailing silence that ends an armed-listening answer
    pub endpoint_silence_ms: u64,
}

impl Default for STTConfig {
    fn default() -> Self {
        Self {
            provider: SttProvider::Local,
            remote: RemoteSttConfig::default(),
            preset: STTPreset::Balanced,
            language: "en".to_string(),
            translate: false,
            vad_threshold: 0.5,
            vocabulary: vec!["banshee".to_string()],
            endpoint_silence_ms: 2500,
        }
    }
}

#[derive(Deserialize, Serialize, Debug)]
#[serde(rename_all = "lowercase")]
pub enum TTSFallback {
    System,
    None,
}

#[derive(Deserialize, Serialize, Debug)]
#[serde(default, deny_unknown_fields)]
pub struct TTSConfig {
    pub provider: TtsProvider,
    pub remote: RemoteTtsConfig,
    pub voice: String,
    #[serde(deserialize_with = "rate")]
    pub speed: f32,
    pub fallback: TTSFallback,
}

impl Default for TTSConfig {
    fn default() -> Self {
        Self {
            provider: TtsProvider::Local,
            remote: RemoteTtsConfig::default(),
            voice: "af_sky".to_string(),
            speed: 1.2,
            fallback: TTSFallback::System,
        }
    }
}

#[derive(Deserialize, Serialize, Debug, Default)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub daemon: DaemonConfig,
    pub audio: AudioConfig,
    pub stt: STTConfig,
    pub tts: TTSConfig,
    /// Parsed so a `config.toml` that carries it still loads, and never written
    /// back or reported, so it does not read as a setting.
    #[serde(default, skip_serializing)]
    #[allow(dead_code, reason = "parsed only so an older config still loads")]
    logging: Option<toml::Value>,
}

impl Config {
    pub fn path() -> Result<PathBuf, BansheeError> {
        get_config_path()
            .ok_or_else(|| BansheeError::Other("Failed to get config path".to_string()))
    }

    /// Empty rather than an error when the file is absent, because no file means
    /// every default.
    pub fn read(path: &Path) -> Result<String, BansheeError> {
        if path.exists() {
            Ok(std::fs::read_to_string(path)?)
        } else {
            Ok(String::new())
        }
    }

    /// Parses a config document, and refuses an `api_key` in it.
    pub fn parse(text: &str) -> Result<Self, BansheeError> {
        toml::from_str(text).map_err(|error| {
            // toml renders a parse error with the offending line above it, so
            // the parser's own text would carry the key into stderr, the log and
            // an RPC reply. Every other fault keeps its span, which is how a
            // person finds the byte at fault.
            if error.message().starts_with("unknown field `api_key`") {
                BansheeError::Rejected(api_key_refusal(api_key_setting(text)))
            } else {
                error.into()
            }
        })
    }

    pub fn load() -> Result<Self, BansheeError> {
        let contents = Config::read(&Config::path()?)?;
        Config::parse(&contents)
    }
}

#[cfg(test)]
mod language_tests {
    /// Nothing read this field before, so a config written then can hold any
    /// string. Exiting on one is a daemon launchd restarts for ever.
    #[test]
    fn an_unknown_code_reads_as_english_rather_than_stopping_the_daemon() {
        let config: super::Config =
            toml::from_str("[stt]\nlanguage = \"en-US\"\n").expect("an old config must load");
        assert_eq!(config.stt.language, "en");
    }

    #[test]
    fn a_code_the_engine_knows_is_kept() {
        let config: super::Config = toml::from_str("[stt]\nlanguage = \"hi\"\n").unwrap();
        assert_eq!(config.stt.language, "hi");
    }

    /// `auto` is the config's own word for detect it and is not in the engine's
    /// table, so it has to survive the same check.
    #[test]
    fn auto_survives() {
        let config: super::Config = toml::from_str("[stt]\nlanguage = \"auto\"\n").unwrap();
        assert_eq!(config.stt.language, "auto");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // `hotkey_mode` under `[tts]` is valid TOML, so only `deny_unknown_fields`
    // stands between a misplaced key and a silent default
    #[test]
    fn a_key_under_the_wrong_section_is_rejected() {
        let misplaced = "[tts]\nvoice = \"af_sky\"\nhotkey_mode = \"toggle\"\n";
        let error = toml::from_str::<Config>(misplaced)
            .expect_err("a key in the wrong section must not parse");
        assert!(
            error.to_string().contains("hotkey_mode"),
            "the error must name the offending key: {error}"
        );

        let placed = "[audio]\nhotkey_mode = \"toggle\"\n\n[tts]\nvoice = \"af_sky\"\n";
        let config: Config = toml::from_str(placed).expect("the same key parses under [audio]");
        assert!(matches!(config.audio.hotkey_mode, HotkeyMode::Toggle));
    }

    // The listener matches what this field parses, so an unmatchable binding
    // must fail the config load, not sit silent behind a working-looking file
    #[test]
    fn a_hotkey_the_listener_cannot_match_is_refused() {
        let error = toml::from_str::<Config>("[audio]\nhotkey = \"banana\"\n")
            .expect_err("an unknown key name must not parse");
        assert!(
            error.to_string().contains("RightOption"),
            "the error must list the legal names: {error}"
        );

        let config: Config = toml::from_str("[audio]\nhotkey = \"RightOption\"\n").unwrap();
        assert_eq!(
            config.audio.hotkey,
            crate::binding::Hotkey::Modifier(rdev::Key::AltGr)
        );
    }

    #[test]
    fn a_config_without_the_key_reads_as_local() {
        let config: Config =
            toml::from_str("[stt]\npreset = \"fast\"\n\n[tts]\nvoice = \"af_sky\"\n").unwrap();
        assert_eq!(config.stt.provider, SttProvider::Local);
        assert_eq!(config.tts.provider, TtsProvider::Local);
    }

    #[test]
    fn a_provider_the_daemon_does_not_have_is_refused_and_the_message_names_local() {
        let error = toml::from_str::<Config>("[stt]\nprovider = \"cloud\"\n")
            .expect_err("an unknown listener must not parse");
        assert!(
            error.to_string().contains("local"),
            "the error must list the legal values: {error}"
        );

        let error = toml::from_str::<Config>("[tts]\nprovider = \"cloud\"\n")
            .expect_err("an unknown voice provider must not parse");
        assert!(
            error.to_string().contains("local"),
            "the error must list the legal values: {error}"
        );
    }

    #[test]
    fn a_remote_listener_parses_with_its_table() {
        let config: Config = toml::from_str(
            "[stt]\nprovider = \"remote\"\n\n[stt.remote]\nbase_url = \"https://api.groq.com/openai/v1\"\nmodel = \"whisper-large-v3-turbo\"\n",
        )
        .unwrap();
        assert_eq!(config.stt.provider, SttProvider::Remote);
        assert!(config.stt.provider.is_remote());
        assert_eq!(config.stt.remote.base_url, "https://api.groq.com/openai/v1");
        assert_eq!(config.stt.remote.model, "whisper-large-v3-turbo");
        assert_eq!(config.stt.remote.host(), "api.groq.com");
    }

    /// A pasted URL can carry a user, a password and a port. The window says
    /// "audio goes to {host}", where none of them belong.
    #[test]
    fn only_the_host_survives_a_url_that_carries_a_user_and_a_port() {
        assert_eq!(
            host_of("https://someone:sk-secret@listener.example:8443/v1"),
            "listener.example"
        );
    }

    #[test]
    fn a_base_url_that_is_not_a_url_names_no_host() {
        assert_eq!(host_of("not a url"), "");
    }

    #[test]
    fn the_remote_table_defaults_to_openai() {
        let config: Config = toml::from_str("[stt]\nprovider = \"remote\"\n").unwrap();
        assert_eq!(config.stt.remote.base_url, "https://api.openai.com/v1");
        assert_eq!(config.stt.remote.model, "whisper-1");
        assert_eq!(config.stt.remote.host(), "api.openai.com");
    }

    // toml renders a parse error with the offending line, so passing it through
    // would echo the key.
    #[test]
    fn a_key_in_the_config_file_is_refused_and_the_message_names_the_command() {
        let error = Config::parse("[stt.remote]\napi_key = \"sk-test\"\n")
            .expect_err("the key must not parse from config.toml");
        assert!(
            error
                .to_string()
                .contains("banshee config set stt.remote.api_key"),
            "the error must name the command: {error}"
        );
        assert!(
            !error.to_string().contains("sk-test"),
            "the refusal must not echo the key: {error}"
        );
    }

    // `[stt] api_key` is the other place a person reaches for.
    #[test]
    fn a_key_under_the_stt_table_is_refused_the_same_way() {
        let error = Config::parse("[stt]\napi_key = \"sk-test\"\n")
            .expect_err("the key must not parse from config.toml");
        assert!(
            error
                .to_string()
                .contains("banshee config set stt.remote.api_key"),
            "the error must name the command: {error}"
        );
        assert!(
            !error.to_string().contains("sk-test"),
            "the refusal must not echo the key: {error}"
        );
    }

    /// The window's Server row commits an empty field, and an empty server sends
    /// the audio nowhere, so the refusal lives where the value is read.
    #[test]
    fn an_empty_remote_server_is_refused() {
        let error = Config::parse("[stt.remote]\nbase_url = \"\"\n")
            .expect_err("an empty server must not parse");
        assert!(
            error
                .to_string()
                .contains("stt.remote.base_url needs a value"),
            "{error}"
        );
    }

    #[test]
    fn an_empty_remote_model_is_refused() {
        let error = Config::parse("[stt.remote]\nmodel = \"\"\n")
            .expect_err("an empty model must not parse");
        assert!(
            error.to_string().contains("stt.remote.model needs a value"),
            "{error}"
        );
    }

    // The line and column are the only way a person finds the byte that broke a
    // file they hand-edited.
    #[test]
    fn a_fault_that_is_not_the_key_keeps_the_line_toml_points_at() {
        let error = Config::parse("[audio]\nhotkey = \"banana\"\n")
            .expect_err("an unknown binding must not parse");
        assert!(error.to_string().contains("hotkey = \"banana\""), "{error}");
    }

    #[test]
    fn a_remote_speaker_parses_with_its_table() {
        let config: Config = toml::from_str(
            "[tts]\nprovider = \"remote\"\n\n[tts.remote]\nbase_url = \"https://api.openai.com/v1\"\nmodel = \"gpt-4o-mini-tts\"\nvoice = \"marin\"\ninstructions = \"Calm and even\"\n",
        )
        .unwrap();
        assert_eq!(config.tts.provider, TtsProvider::Remote);
        assert!(config.tts.provider.is_remote());
        assert_eq!(config.tts.remote.model, "gpt-4o-mini-tts");
        assert_eq!(config.tts.remote.voice, "marin");
        assert_eq!(config.tts.remote.instructions, "Calm and even");
        assert_eq!(config.tts.remote.host(), "api.openai.com");
    }

    #[test]
    fn the_remote_speaker_table_defaults_to_openai_with_no_voice() {
        let config: Config = toml::from_str("[tts]\nprovider = \"remote\"\n").unwrap();
        assert_eq!(config.tts.remote.base_url, "https://api.openai.com/v1");
        assert_eq!(config.tts.remote.model, "tts-1");
        assert_eq!(config.tts.remote.voice, "");
        assert_eq!(config.tts.remote.instructions, "");
    }

    #[test]
    fn the_remote_speaker_asks_for_wav_and_names_no_rate_by_default() {
        let config: Config = toml::from_str("[tts]\nprovider = \"remote\"\n").unwrap();
        assert_eq!(config.tts.remote.response_format, SpeechFormat::Wav);
        assert_eq!(config.tts.remote.sample_rate, None);
    }

    #[test]
    fn a_named_format_and_rate_parse() {
        let config: Config =
            toml::from_str("[tts.remote]\nresponse_format = \"pcm\"\nsample_rate = 22050\n")
                .unwrap();
        assert_eq!(config.tts.remote.response_format, SpeechFormat::Pcm);
        assert_eq!(
            config.tts.remote.sample_rate.map(std::num::NonZero::get),
            Some(22050)
        );
    }

    #[test]
    fn a_format_the_speaker_cannot_decode_is_refused_at_parse() {
        let error = Config::parse("[tts.remote]\nresponse_format = \"mp3\"\n")
            .expect_err("a format with no decoder must not parse");
        assert!(error.to_string().contains("wav"), "{error}");
        assert!(error.to_string().contains("pcm"), "{error}");
    }

    #[test]
    fn a_rate_of_zero_is_refused_at_parse() {
        let error = Config::parse("[tts.remote]\nsample_rate = 0\n")
            .expect_err("a rate of zero must not parse");
        assert!(error.to_string().contains("nonzero"), "{error}");
    }

    #[test]
    fn an_empty_remote_speaker_server_is_refused() {
        let error = Config::parse("[tts.remote]\nbase_url = \"\"\n")
            .expect_err("an empty server must not parse");
        assert!(
            error
                .to_string()
                .contains("tts.remote.base_url needs a value"),
            "{error}"
        );
    }

    #[test]
    fn an_empty_remote_speaker_model_is_refused() {
        let error = Config::parse("[tts.remote]\nmodel = \"\"\n")
            .expect_err("an empty model must not parse");
        assert!(
            error.to_string().contains("tts.remote.model needs a value"),
            "{error}"
        );
    }

    // Two tables hold a key, and the refusal is only useful if it names the one
    // the person reached for.
    #[test]
    fn a_speaker_key_in_the_config_file_names_the_speaker_command() {
        for text in [
            "[tts.remote]\napi_key = \"sk-test\"\n",
            "[tts]\napi_key = \"sk-test\"\n",
        ] {
            let error = Config::parse(text).expect_err("the key must not parse from config.toml");
            assert!(
                error
                    .to_string()
                    .contains("banshee config set tts.remote.api_key"),
                "the error must name the speaker's command: {error}"
            );
            assert!(
                !error.to_string().contains("sk-test"),
                "the refusal must not echo the key: {error}"
            );
        }
    }

    #[test]
    fn a_listener_key_in_the_config_file_still_names_the_listener_command() {
        let error = Config::parse("[stt.remote]\napi_key = \"sk-test\"\n")
            .expect_err("the key must not parse from config.toml");
        assert!(
            error
                .to_string()
                .contains("banshee config set stt.remote.api_key"),
            "{error}"
        );
    }
}
