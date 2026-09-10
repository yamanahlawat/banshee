use std::io::Read;
use std::path::{Path, PathBuf};

use banshee_common::{error::BansheeError, utils::get_credentials_path};
use serde::{Deserialize, Serialize};

// Nested so each key reads as `[stt.remote] api_key`, the way config.toml
// spells it
#[derive(Deserialize, Serialize, Default)]
#[serde(default)]
struct File {
    stt: Side,
    tts: Side,
}

#[derive(Deserialize, Serialize, Default)]
#[serde(default)]
struct Side {
    remote: Remote,
}

#[derive(Deserialize, Serialize, Default)]
#[serde(default)]
struct Remote {
    api_key: Option<String>,
}

/// Which remote side a key belongs to. Nothing is shared between the two but
/// this file and its parser.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteKey {
    Stt,
    Tts,
}

impl RemoteKey {
    pub fn setting(self) -> &'static str {
        match self {
            RemoteKey::Stt => "stt.remote.api_key",
            RemoteKey::Tts => "tts.remote.api_key",
        }
    }

    pub fn of_setting(key: &str) -> Option<RemoteKey> {
        [RemoteKey::Stt, RemoteKey::Tts]
            .into_iter()
            .find(|side| side.setting() == key)
    }

    /// The word each surface uses for this side, so a prompt, a checklist line
    /// and a fix all name it alike.
    pub fn side(self) -> &'static str {
        match self {
            RemoteKey::Stt => "listener",
            RemoteKey::Tts => "speaker",
        }
    }

    /// What a side that will not start says about the key it has not got.
    pub fn no_key(self) -> String {
        format!(
            "no key for the remote {}; set one with: banshee config set {}",
            self.side(),
            self.setting()
        )
    }
}

/// What the daemon reads at startup and what a write changes.
#[derive(Default, PartialEq, Eq)]
pub struct Credentials {
    pub stt_api_key: Option<String>,
    pub tts_api_key: Option<String>,
}

// Hand written, not derived: a derive would put the keys themselves in every
// line that prints a struct holding this one
impl std::fmt::Debug for Credentials {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let held = |key: &Option<String>| match key {
            Some(_) => "<set>",
            None => "<unset>",
        };
        write!(
            formatter,
            "Credentials {{ stt_api_key: {}, tts_api_key: {} }}",
            held(&self.stt_api_key),
            held(&self.tts_api_key)
        )
    }
}

impl Credentials {
    pub fn path() -> Result<PathBuf, BansheeError> {
        get_credentials_path()
            .ok_or_else(|| BansheeError::Other("Failed to get the credentials path".to_string()))
    }

    pub fn load() -> Result<Self, BansheeError> {
        Self::read(&Self::path()?)
    }

    pub fn key(&self, side: RemoteKey) -> Option<&str> {
        match side {
            RemoteKey::Stt => self.stt_api_key.as_deref(),
            RemoteKey::Tts => self.tts_api_key.as_deref(),
        }
    }

    /// A file that will not parse holds no key the engine can use, so it answers
    /// the same as an absent one. The status checklist reports the parse fault.
    pub fn present(side: RemoteKey) -> bool {
        Self::load().is_ok_and(|credentials| credentials.key(side).is_some())
    }

    fn read(path: &Path) -> Result<Self, BansheeError> {
        let file = Self::read_file(path)?;
        let held = |key: Option<String>| key.filter(|key| !key.is_empty());
        Ok(Self {
            stt_api_key: held(file.stt.remote.api_key),
            tts_api_key: held(file.tts.remote.api_key),
        })
    }

    fn read_file(path: &Path) -> Result<File, BansheeError> {
        match std::fs::read_to_string(path) {
            // A TOML parse error quotes the line it stumbled on, so the
            // parser's own text would carry a key into a status reply
            Ok(text) => toml::from_str(&text).map_err(|_| {
                BansheeError::Other(format!(
                    "{} does not parse; fix it or delete it and set the keys again",
                    path.display()
                ))
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(File::default()),
            Err(error) => Err(error.into()),
        }
    }

    /// Every key given, in one write. `banshee config remote` sets both sides
    /// at once, and two writes leave a window where one side is on file and the
    /// other is not. An empty value removes the key on file.
    pub fn set_many(keys: &[(RemoteKey, &str)]) -> Result<(), BansheeError> {
        Self::write_many(&Self::path()?, keys)
    }

    fn write_many(path: &Path, keys: &[(RemoteKey, &str)]) -> Result<(), BansheeError> {
        let mut file = Self::read_file(path)?;
        for (side, value) in keys {
            let stored = Some(*value)
                .filter(|key| !key.is_empty())
                .map(str::to_string);
            match side {
                RemoteKey::Stt => file.stt.remote.api_key = stored,
                RemoteKey::Tts => file.tts.remote.api_key = stored,
            }
        }
        let rendered =
            toml::to_string(&file).map_err(|error| BansheeError::Other(error.to_string()))?;
        banshee_common::utils::write_atomically(path, rendered.as_bytes(), Some(0o600))?;
        Ok(())
    }
}

const REDACTED: &str = "<redacted>";
/// What a key opens with. `sk-` covers Anthropic's `sk-ant-` keys as well.
const KEY_PREFIXES: [&str; 4] = ["sk-", "gsk_", "xai-", "AIza"];
const KEY_TAIL: usize = 8;
const SHORTEST_KEY: usize = 8;

/// Every key-shaped run replaced, before text a server sent reaches a log or a
/// person. Two rules: the key the caller holds, whatever it looks like, and a
/// token that opens like a key, for a key the server itself put in the answer.
pub fn redacted(text: &str, api_key: &str) -> String {
    // A local server that wants no key takes any stand-in. A stand-in shorter
    // than a key matches inside ordinary words, and an empty one matches
    // between every character.
    let held = if api_key.len() >= SHORTEST_KEY {
        text.replace(api_key, REDACTED)
    } else {
        text.to_string()
    };
    let mut out = String::with_capacity(held.len());
    let mut rest = held.as_str();
    while let Some(key) = next_key(rest) {
        out.push_str(&rest[..key.start]);
        out.push_str(REDACTED);
        rest = &rest[key.end..];
    }
    out.push_str(rest);
    out
}

/// The next run that opens with a key prefix and carries enough after it to be
/// a key. A prefix inside a word opens nothing, so `task-oriented` keeps its
/// letters, and a run too short to be a key is left to be read.
fn next_key(text: &str) -> Option<std::ops::Range<usize>> {
    text.char_indices()
        .filter(|(at, _)| starts_a_word(text, *at))
        .find_map(|(at, _)| {
            let prefix = KEY_PREFIXES
                .iter()
                .find(|prefix| text[at..].starts_with(**prefix))?;
            let opens = at + prefix.len();
            // Every key character is one byte, so this count is also a length
            let tail = text[opens..]
                .chars()
                .take_while(|character| is_key_char(*character))
                .count();
            (tail >= KEY_TAIL).then_some(at..opens + tail)
        })
}

fn starts_a_word(text: &str, at: usize) -> bool {
    text[..at]
        .chars()
        .next_back()
        .is_none_or(|before| !before.is_alphanumeric() && before != '_')
}

fn is_key_char(character: char) -> bool {
    character.is_ascii_alphanumeric() || character == '-' || character == '_'
}

/// A cap can split a key and leave a head no rule here recognises.
pub fn without_a_split_key(text: &str) -> &str {
    match text
        .char_indices()
        .rev()
        .find(|(_, character)| !is_key_char(*character))
    {
        // The whole character, because every character outside the alphabet
        // may be more than one byte
        Some((at, character)) => &text[..at + character.len_utf8()],
        None => "",
    }
}

/// Text a server sent, as one line with single spaces. A body of its own lines
/// could otherwise forge a `banshee:` line in the daemon log, and a reason of
/// its own lines could break the status line it is read in.
pub fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// How much of a refused answer is read. Nothing past the fault it names is of
/// use, so a server that answers a refusal with megabytes fills no memory with
/// them.
pub const BODY_LIMIT: u64 = 8 * 1024;

/// Whether a refused answer's body is read at all. A key-refusal body is the
/// likeliest place for a server to echo the key it refused, and it names no
/// fault the status does not. Both remote clients ask this, so the two sides
/// cannot drift apart on the one answer that must not.
pub fn may_read_body(status: reqwest::StatusCode) -> bool {
    !matches!(status.as_u16(), 401 | 403)
}

pub fn read_body(response: reqwest::blocking::Response) -> String {
    let mut body = Vec::new();
    let read = response.take(BODY_LIMIT).read_to_end(&mut body);
    let text = String::from_utf8_lossy(&body);
    // A read that stopped early, at the cap or at a fault, can have stopped
    // inside a key
    let whole = if read.is_ok() && (body.len() as u64) < BODY_LIMIT {
        text.as_ref()
    } else {
        without_a_split_key(&text)
    };
    whole.to_string()
}

/// Why a request never reached a server. The host is the whole of what it
/// names: nothing a caller sent is in a reason a person reads.
pub fn describe_send_error(host: &str, error: &reqwest::Error) -> String {
    if error.is_timeout() {
        format!("{host} did not answer in time")
    } else if error.is_connect() {
        format!("{host} could not be reached")
    } else {
        format!("the request to {host} failed: {error}")
    }
}

#[cfg(test)]
mod tests {
    use super::{Credentials, RemoteKey, redacted};
    use std::os::unix::fs::PermissionsExt;

    fn scratch(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("banshee-{name}-{}.toml", std::process::id()))
    }

    #[test]
    fn a_missing_file_holds_no_key() {
        let path = scratch("absent");
        let _ = std::fs::remove_file(&path);
        assert_eq!(Credentials::read(&path).unwrap(), Credentials::default());
    }

    // `banshee config remote` sets both sides at once, so one write has to
    // carry both: a write that kept only the last pair would leave the other
    // side unset with nothing to say so.
    #[test]
    fn one_write_sets_both_keys_and_the_file_is_owner_only() {
        let path = scratch("both");
        Credentials::write_many(
            &path,
            &[
                (RemoteKey::Stt, "sk-listener"),
                (RemoteKey::Tts, "sk-speaker"),
            ],
        )
        .unwrap();
        let held = Credentials::read(&path).unwrap();
        assert_eq!(held.key(RemoteKey::Stt), Some("sk-listener"));
        assert_eq!(held.key(RemoteKey::Tts), Some("sk-speaker"));
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(
            mode, 0o600,
            "the key file must be readable by its owner alone"
        );
        let _ = std::fs::remove_file(&path);
    }

    // A write reads the file first, so a careless one would drop the other side.
    #[test]
    fn removing_one_key_leaves_the_other_on_file() {
        let path = scratch("one-removed");
        Credentials::write_many(
            &path,
            &[
                (RemoteKey::Stt, "sk-listener"),
                (RemoteKey::Tts, "sk-speaker"),
            ],
        )
        .unwrap();
        Credentials::write_many(&path, &[(RemoteKey::Tts, "")]).unwrap();
        let held = Credentials::read(&path).unwrap();
        assert_eq!(held.key(RemoteKey::Stt), Some("sk-listener"));
        assert_eq!(held.key(RemoteKey::Tts), None);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn the_debug_rendering_says_which_keys_are_there_and_not_what_they_are() {
        let set = Credentials {
            stt_api_key: Some("sk-listener".to_string()),
            tts_api_key: Some("sk-speaker".to_string()),
        };
        let rendering = format!("{set:?}");
        assert!(!rendering.contains("sk-listener"), "{rendering}");
        assert!(!rendering.contains("sk-speaker"), "{rendering}");
        assert_eq!(
            rendering,
            "Credentials { stt_api_key: <set>, tts_api_key: <set> }"
        );
        assert_eq!(
            format!("{:?}", Credentials::default()),
            "Credentials { stt_api_key: <unset>, tts_api_key: <unset> }"
        );
    }

    #[test]
    fn a_file_that_does_not_parse_names_the_path_and_not_the_key() {
        let path = scratch("unparsable");
        std::fs::write(&path, "[tts.remote]\napi_key = sk-live-SECRET123\n").unwrap();
        let error = Credentials::read(&path).unwrap_err().to_string();
        assert!(
            !error.contains("sk-live-SECRET123"),
            "the key must not be in the error: {error}"
        );
        assert!(error.contains(&path.display().to_string()), "{error}");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn the_key_the_caller_holds_is_replaced_wherever_it_appears() {
        let key = "kokoro-9f3c2ab7d14e5b6079f3";
        let body = format!(r#"{{"detail":"{key} is not a model. Call /v1/models"}}"#);
        let hidden = redacted(&body, key);
        assert!(!hidden.contains("kokoro-9f3c"), "{hidden}");
        assert!(hidden.contains("<redacted>"), "{hidden}");
        assert!(
            hidden.contains("is not a model. Call /v1/models"),
            "{hidden}"
        );
    }

    // A key a server echoes need not be the key that was sent to it, so the
    // prefix rule is the one that catches it.
    #[test]
    fn a_key_the_caller_never_held_is_replaced_by_its_prefix() {
        for token in [
            "sk-4f9c2be71a3d05e8",
            "gsk_8Hn2Qv6Lp0Rt4Ws9",
            "sk-ant-api03-Zb7Yc1Xd",
            "xai-3Kp9Mn2Qr7Ts5Vw1",
            "AIzaSyD9fK2mQ7pR4tX1bN",
        ] {
            let body = format!(
                r#"{{"detail":{{"error":"Invalid model name passed in model={token}."}}}}"#
            );
            let hidden = redacted(&body, "sk-test");
            assert!(!hidden.contains(token), "{token} survived: {hidden}");
            assert!(hidden.contains("<redacted>"), "{hidden}");
            assert!(hidden.contains("Invalid model name"), "{hidden}");

            let said = format!("le modèle {token} est introuvable");
            let hidden = redacted(&said, "sk-test");
            assert!(!hidden.contains(token), "{token} survived: {hidden}");
            assert!(hidden.starts_with("le modèle "), "{hidden}");
            assert!(hidden.ends_with(" est introuvable"), "{hidden}");
        }
    }

    // The message a server sends is what a person acts on, so redaction that
    // eats a model name or the link to open is worse than none.
    #[test]
    fn redaction_leaves_a_message_a_model_name_and_a_url_alone() {
        for kept in [
            r#"{"error":{"message":"The model `canopylabs/orpheus-v1-english` requires terms acceptance. Please have the org admin accept the terms at https://console.groq.com/playground?model=canopylabs%2Forpheus-v1-english"}}"#,
            r#"{"error":{"message":"unknown field `instructions` in request body"}}"#,
            r#"{"detail":"a task-oriented model refuses this"}"#,
            r#"{"detail":"sk-1234567 is too short to be a key"}"#,
            // A server answers in the language it is set to
            r#"{"detail":"le modèle « caché » est introuvable"}"#,
            r#"{"detail":"モデルが見つかりません"}"#,
        ] {
            assert_eq!(redacted(kept, "sk-test"), kept);
        }
    }

    // A server that wants no key at all takes any stand-in, and a stand-in
    // short enough matches inside ordinary words.
    #[test]
    fn a_stand_in_too_short_to_be_a_key_replaces_nothing() {
        let said = "a model that cannot be reached";
        for stand_in in ["a", "no", "none", "local", "kokoro-"] {
            assert_eq!(redacted(said, stand_in), said, "{stand_in}");
        }
        // One character longer, and it is long enough to be a key
        let key = "kokoro-9";
        let held = format!("the key {key} is not for this model");
        let hidden = redacted(&held, key);
        assert!(!hidden.contains(key), "{hidden}");
        assert!(hidden.contains("<redacted>"), "{hidden}");
    }

    // A read cap can split a key and leave its head behind, which no rule here
    // can recognise.
    #[test]
    fn a_body_cut_mid_key_loses_the_run_the_cut_left() {
        let cut = "Invalid model name passed in model=sk-pro";
        let kept = super::without_a_split_key(cut);
        assert!(!kept.contains("sk-pro"), "{kept}");
        assert!(kept.starts_with("Invalid model name"), "{kept}");
        // A cut that fell outside a run keeps every word it read
        let whole = "the upstream model is down; ";
        assert_eq!(super::without_a_split_key(whole), whole);
        assert_eq!(super::without_a_split_key("sk-proj7Qm4Xb2v"), "");
    }

    // A server's error page is the least ASCII text the daemon reads, and the
    // cut keeps whole characters: every character outside the key alphabet is
    // one the cut may end on.
    #[test]
    fn a_body_cut_in_another_language_keeps_whole_characters() {
        let kept = super::without_a_split_key("modèle");
        assert!(kept.ends_with('è'), "{kept}");
        assert!(!kept.contains('l'), "{kept}");
        assert_eq!(super::without_a_split_key("boom 😀abc"), "boom 😀");
        // What a cap leaves when it cuts inside a character: the lossy read
        // ends on the replacement character
        let cut = String::from_utf8_lossy(&"café".as_bytes()[.."café".len() - 1]);
        let kept = super::without_a_split_key(&cut);
        assert!(kept.ends_with('\u{fffd}'), "{kept}");
        assert!(kept.starts_with("caf"), "{kept}");
    }

    // A body a server wrote must not be able to forge a line of its own in the
    // daemon log.
    #[test]
    fn text_a_server_sent_becomes_one_line() {
        let forged = "no such model\nbanshee: the listener is fine\r\n\tand idle";
        let line = super::one_line(forged);
        assert!(!line.chars().any(char::is_control), "{line}");
        assert!(!line.contains("  "), "{line}");
        assert!(line.starts_with("no such model banshee:"), "{line}");
        assert!(line.ends_with("and idle"), "{line}");

        // A space that is not ASCII is still a space, and the words it
        // separates keep their characters
        let spaced = super::one_line("modèle\u{00a0}introuvable\n\tcafé");
        assert_eq!(spaced.split(' ').count(), 3, "{spaced}");
        assert!(spaced.ends_with("café"), "{spaced}");
        assert!(spaced.starts_with("modèle introuvable"), "{spaced}");
    }

    // A reason never carries the body, so this rule is the only place a test
    // can hold it: the two statuses that refuse the key are the two whose body
    // nothing reads.
    #[test]
    fn a_refused_key_reads_no_body_at_all() {
        for refused in [401, 403] {
            assert!(
                !super::may_read_body(reqwest::StatusCode::from_u16(refused).unwrap()),
                "{refused} may echo the key it refused"
            );
        }
        for read in [400, 404, 429, 500, 503] {
            assert!(
                super::may_read_body(reqwest::StatusCode::from_u16(read).unwrap()),
                "{read} names a fault worth logging"
            );
        }
    }

    #[test]
    fn each_side_names_its_own_setting() {
        assert_eq!(RemoteKey::Stt.setting(), "stt.remote.api_key");
        assert_eq!(RemoteKey::Tts.setting(), "tts.remote.api_key");
        assert_eq!(
            RemoteKey::of_setting("tts.remote.api_key"),
            Some(RemoteKey::Tts)
        );
        assert_eq!(
            RemoteKey::of_setting("stt.remote.api_key"),
            Some(RemoteKey::Stt)
        );
        assert_eq!(RemoteKey::of_setting("tts.voice"), None);
    }
}
