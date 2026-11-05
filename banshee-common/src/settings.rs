//! Every setting a person can write, defined once: its key, what it takes,
//! what an absent key reads as, and the line that describes it. The daemon's
//! `Config` stays the parser and validator.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Domain {
    Toggle,
    Choice {
        values: &'static [&'static str],
    },
    /// `max` is the largest value `Config` takes, or `None` where only the
    /// largest integer a file holds stops it.
    Number {
        min: f64,
        max: Option<f64>,
        whole: bool,
    },
    Words,
    Text {
        empty_allowed: bool,
    },
    Url,
    Hotkey,
    ListeningModel,
    Voice,
    Secret,
}

impl Domain {
    /// The JSON a command-line word stands for. `null` removes the key in every domain.
    pub fn typed(&self, word: &str) -> serde_json::Value {
        if word == "null" {
            return serde_json::Value::Null;
        }
        match self {
            // A word that is not JSON stays text, so the parse names the fault
            Domain::Toggle | Domain::Number { .. } | Domain::Words => serde_json::from_str(word)
                .unwrap_or_else(|_| serde_json::Value::String(word.to_string())),
            _ => serde_json::Value::String(word.to_string()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Setting {
    pub key: &'static str,
    pub domain: Domain,
    /// JSON text of the value an absent key reads as. `None` for a secret.
    pub default: Option<&'static str>,
    /// The key decides what leaves this machine.
    pub sends_out: bool,
    pub description: &'static str,
}

const fn setting(
    key: &'static str,
    domain: Domain,
    default: &'static str,
    description: &'static str,
) -> Setting {
    Setting {
        key,
        domain,
        default: Some(default),
        sends_out: false,
        description,
    }
}

const fn sending(setting: Setting) -> Setting {
    Setting {
        sends_out: true,
        ..setting
    }
}

const fn secret(key: &'static str, description: &'static str) -> Setting {
    Setting {
        key,
        domain: Domain::Secret,
        default: None,
        sends_out: true,
        description,
    }
}

const PROVIDERS: &[&str] = &["local", "remote"];
const WHOLE_FROM_ZERO: Domain = Domain::Number {
    min: 0.0,
    max: None,
    whole: true,
};

pub const SETTINGS: &[Setting] = &[
    setting(
        "daemon.save_history",
        Domain::Toggle,
        "true",
        "keep dictations and agent questions with your answers in ~/.banshee/banshee.db; off keeps nothing new, and what is kept stays until cleared",
    ),
    setting(
        "audio.input_device",
        Domain::Text {
            empty_allowed: true,
        },
        "\"default\"",
        "\"default\" = follow the OS; otherwise match a device name",
    ),
    setting(
        "audio.hotkey",
        Domain::Hotkey,
        "\"RightOption\"",
        "F1-F12, a lone modifier, or a chord like \"Ctrl+Alt+D\"",
    ),
    setting(
        "audio.hotkey_mode",
        Domain::Choice {
            values: &["hold", "toggle"],
        },
        "\"hold\"",
        "hold = record while the hotkey is down | toggle = tap to start, tap to stop",
    ),
    setting(
        "audio.barge_in",
        Domain::Choice {
            values: &["stop", "none"],
        },
        "\"stop\"",
        "stop = the record hotkey cuts off whatever Banshee is saying | none",
    ),
    setting(
        "audio.keep_open",
        Domain::Toggle,
        "false",
        "true = hold the microphone open between uses",
    ),
    setting(
        "feedback.mode",
        Domain::Choice {
            values: &["visual", "sound", "both", "none"],
        },
        "\"both\"",
        "visual | sound | both | none",
    ),
    setting(
        "feedback.speak_problems",
        Domain::Toggle,
        "true",
        "false = never say a new problem aloud",
    ),
    sending(setting(
        "stt.provider",
        Domain::Choice { values: PROVIDERS },
        "\"local\"",
        "local | remote = send each utterance to the server in stt.remote.base_url",
    )),
    sending(setting(
        "stt.remote.base_url",
        Domain::Url,
        "\"https://api.openai.com/v1\"",
        "an OpenAI-compatible /v1 root; https, or http on your own network",
    )),
    setting(
        "stt.remote.model",
        Domain::Text {
            empty_allowed: false,
        },
        "\"whisper-1\"",
        "the model the remote server names",
    ),
    secret(
        "stt.remote.api_key",
        "The key for the remote listener, kept in credentials.toml",
    ),
    setting(
        "stt.model",
        Domain::ListeningModel,
        "\"parakeet-unified-en-0.6b\"",
        "the id of the local model that transcribes speech",
    ),
    setting(
        "stt.translate",
        Domain::Toggle,
        "false",
        "true answers in English, for a model or server that translates",
    ),
    setting(
        "stt.vad_threshold",
        Domain::Number {
            min: 0.0,
            max: Some(1.0),
            whole: false,
        },
        "0.5",
        "0.0 to 1.0; higher means stricter speech detection",
    ),
    setting(
        "stt.vocabulary",
        Domain::Words,
        "[]",
        "words the model keeps mangling, for example [\"clippy\", \"tokio\"]",
    ),
    setting(
        "stt.endpoint_silence_ms",
        WHOLE_FROM_ZERO,
        "2500",
        "trailing silence that ends a spoken answer",
    ),
    sending(setting(
        "tts.provider",
        Domain::Choice { values: PROVIDERS },
        "\"local\"",
        "local | remote = send each reply's text to the server in tts.remote.base_url",
    )),
    setting(
        "tts.fallback",
        Domain::Choice {
            values: &["local", "none"],
        },
        "\"local\"",
        "local = tts.voice speaks a failed remote reply | none",
    ),
    setting(
        "tts.voice",
        Domain::Voice,
        "\"companion\"",
        "companion, or any voice from the Kokoro voices directory",
    ),
    sending(setting(
        "tts.remote.base_url",
        Domain::Url,
        "\"https://api.openai.com/v1\"",
        "an OpenAI-compatible /v1 root; https, or http on your own network",
    )),
    setting(
        "tts.remote.model",
        Domain::Text {
            empty_allowed: false,
        },
        "\"tts-1\"",
        "the model the remote server names",
    ),
    setting(
        "tts.remote.voice",
        Domain::Text {
            empty_allowed: true,
        },
        "\"\"",
        "a voice the remote server names; the remote speaker needs one to start",
    ),
    setting(
        "tts.remote.instructions",
        Domain::Text {
            empty_allowed: true,
        },
        "\"\"",
        "tone and delivery, for a model that reads it",
    ),
    setting(
        "tts.remote.response_format",
        Domain::Choice {
            values: &["wav", "pcm"],
        },
        "\"wav\"",
        "wav | pcm; what the server is asked to send",
    ),
    setting(
        "tts.remote.sample_rate",
        Domain::Number {
            min: 1.0,
            max: Some(4_294_967_295.0),
            whole: true,
        },
        "null",
        "samples per second; only pcm needs it",
    ),
    secret(
        "tts.remote.api_key",
        "The key for the remote speaker, kept in credentials.toml",
    ),
    setting(
        "models.unload_after",
        WHOLE_FROM_ZERO,
        "5",
        "idle minutes before local models unload; 0 = never",
    ),
];

pub fn find(key: &str) -> Option<&'static Setting> {
    SETTINGS.iter().find(|setting| setting.key == key)
}

#[cfg(test)]
mod tests {
    use super::find;

    #[test]
    fn a_word_is_typed_by_its_setting() {
        assert_eq!(
            find("audio.input_device").unwrap().domain.typed("12"),
            serde_json::json!("12")
        );
        assert_eq!(
            find("stt.vad_threshold").unwrap().domain.typed("0.6"),
            serde_json::json!(0.6)
        );
        assert_eq!(
            find("audio.keep_open").unwrap().domain.typed("true"),
            serde_json::json!(true)
        );
        assert_eq!(
            find("stt.vocabulary").unwrap().domain.typed(r#"["tokio"]"#),
            serde_json::json!(["tokio"])
        );
        assert_eq!(
            find("audio.hotkey").unwrap().domain.typed("null"),
            serde_json::Value::Null
        );
    }
}
