//! The wire shape of `banshee.cue`, shared between the daemon and every
//! client that reads the event, including the tray, which cannot import the
//! daemon's own modules.

use serde::{Deserialize, Serialize};

/// What a signal is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Target {
    Dictate,
    Mailbox,
    Tell,
    Answer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasonCode {
    NoSpeech,
    EmptyTranscript,
    Silence,
    Closed,
    ListenFailed,
    TypeFailed,
    TranscriptionFailed,
    Starting,
    PipelineBroken,
    SpeechFailed,
    TellFailed,
    TellWarned,
}

/// `text` is what every client shows.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reason {
    pub code: ReasonCode,
    pub text: String,
}

impl Reason {
    pub fn new(code: ReasonCode, device: Option<&str>) -> Self {
        let text = match code {
            ReasonCode::NoSpeech | ReasonCode::EmptyTranscript => match device {
                Some(device) => format!("Nothing heard on {device}"),
                None => "Nothing heard".to_string(),
            },
            ReasonCode::Silence => "No answer heard".to_string(),
            ReasonCode::Closed => "The question was closed".to_string(),
            ReasonCode::ListenFailed => "Could not listen".to_string(),
            ReasonCode::TypeFailed => "Could not type the words".to_string(),
            ReasonCode::TranscriptionFailed => "Could not transcribe".to_string(),
            ReasonCode::Starting => "Banshee is still starting".to_string(),
            ReasonCode::PipelineBroken => "Banshee cannot record".to_string(),
            ReasonCode::SpeechFailed => "The reply did not play".to_string(),
            ReasonCode::TellFailed => "The agent run failed".to_string(),
            ReasonCode::TellWarned => "The agent did not answer".to_string(),
        };
        Reason { code, text }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "cue", rename_all = "snake_case")]
pub enum Signal {
    RecordStart {
        target: Target,
    },
    RecordStop {
        target: Target,
    },
    Ready {
        target: Target,
    },
    /// `target` names the job the failure ends, when one owns it.
    Error {
        reason: Reason,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<Target>,
    },
    Arm,
    Disarm,
    Answered {
        heard: bool,
    },
    Told,
    Cancelled {
        target: Target,
    },
    Onset,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wire_lines_read_as_signals() {
        let lines = [
            (
                r#"{"cue":"record_start","target":"dictate"}"#,
                Signal::RecordStart {
                    target: Target::Dictate,
                },
            ),
            (
                r#"{"cue":"ready","target":"mailbox"}"#,
                Signal::Ready {
                    target: Target::Mailbox,
                },
            ),
            (
                r#"{"cue":"error","reason":{"code":"no_speech","text":"Nothing heard on AirPods"}}"#,
                Signal::Error {
                    reason: Reason::new(ReasonCode::NoSpeech, Some("AirPods")),
                    target: None,
                },
            ),
            (
                r#"{"cue":"error","reason":{"code":"transcription_failed","text":"Could not transcribe"},"target":"answer"}"#,
                Signal::Error {
                    reason: Reason::new(ReasonCode::TranscriptionFailed, None),
                    target: Some(Target::Answer),
                },
            ),
            (
                r#"{"cue":"answered","heard":true}"#,
                Signal::Answered { heard: true },
            ),
            (
                r#"{"cue":"record_stop","target":"answer"}"#,
                Signal::RecordStop {
                    target: Target::Answer,
                },
            ),
            (r#"{"cue":"arm"}"#, Signal::Arm),
            (r#"{"cue":"disarm"}"#, Signal::Disarm),
            (r#"{"cue":"told"}"#, Signal::Told),
            (r#"{"cue":"onset"}"#, Signal::Onset),
            (
                r#"{"cue":"cancelled","target":"dictate"}"#,
                Signal::Cancelled {
                    target: Target::Dictate,
                },
            ),
        ];
        for (line, signal) in lines {
            assert_eq!(
                serde_json::from_str::<Signal>(line).unwrap(),
                signal,
                "{line}"
            );
        }
    }

    #[test]
    fn every_signal_survives_the_wire() {
        let reason = Reason::new(ReasonCode::TellWarned, None);
        for signal in [
            Signal::RecordStart {
                target: Target::Answer,
            },
            Signal::RecordStop {
                target: Target::Tell,
            },
            Signal::Ready {
                target: Target::Dictate,
            },
            Signal::Error {
                reason: reason.clone(),
                target: None,
            },
            Signal::Error {
                reason,
                target: Some(Target::Tell),
            },
            Signal::Arm,
            Signal::Disarm,
            Signal::Answered { heard: false },
            Signal::Told,
            Signal::Cancelled {
                target: Target::Mailbox,
            },
            Signal::Onset,
        ] {
            let line = serde_json::to_string(&signal).unwrap();
            assert_eq!(
                serde_json::from_str::<Signal>(&line).unwrap(),
                signal,
                "{line}"
            );
        }
    }

    #[test]
    fn an_error_no_job_owns_sends_no_target() {
        let line = serde_json::to_string(&Signal::Error {
            reason: Reason::new(ReasonCode::SpeechFailed, None),
            target: None,
        })
        .unwrap();
        assert!(!line.contains("target"), "{line}");
    }
}
