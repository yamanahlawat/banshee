//! What a daemon setting decides, and every client that reads it: which
//! earcons sound and whether a screen chip draws.

use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum FeedbackMode {
    Visual,
    Sound,
    Both,
    #[serde(rename = "none")]
    Off,
}

impl FeedbackMode {
    pub fn word(self) -> &'static str {
        match self {
            FeedbackMode::Visual => "visual",
            FeedbackMode::Sound => "sound",
            FeedbackMode::Both => "both",
            FeedbackMode::Off => "none",
        }
    }

    pub fn draws(self) -> bool {
        matches!(self, FeedbackMode::Visual | FeedbackMode::Both)
    }
}
