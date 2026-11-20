//! The speech-to-text models Banshee can listen with, each pinned to one file
//! at one revision so an upstream push cannot change what a person downloaded.

mod catalogue;
pub use catalogue::CATALOGUE;

#[derive(Debug, PartialEq, Eq)]
pub struct Licence {
    /// With its version or date, so a revised text needs a fresh acceptance.
    pub name: &'static str,
    pub url: &'static str,
    /// What the person agrees to, in one line.
    pub summary: &'static str,
    /// The person sees the terms and accepts them before the file downloads.
    pub needs_acceptance: bool,
    /// The attribution the licence asks to show wherever the model is offered.
    pub credit: Option<&'static str>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ListeningModel {
    /// What `stt.model` carries. Matched ignoring case.
    pub id: &'static str,
    /// The name a person reads. Only the suggestions have one.
    pub label: Option<&'static str>,
    /// A Hugging Face repository.
    pub repo: &'static str,
    pub revision: &'static str,
    /// The upstream file name, which is also its name in the models directory.
    pub file: &'static str,
    pub bytes: u64,
    pub sha256: &'static str,
    pub licence: &'static Licence,
    /// The codes the model card names, in its order.
    pub languages: &'static [&'static str],
    pub translates: bool,
    pub streams: bool,
    /// Weighs the words to know while it decodes.
    pub takes_vocabulary: bool,
}

impl ListeningModel {
    pub fn url(&self) -> String {
        format!(
            "https://huggingface.co/{}/resolve/{}/{}",
            self.repo, self.revision, self.file
        )
    }

    pub fn english_only(&self) -> bool {
        self.languages == ["en"]
    }

    /// The name a person reads: a model with no label goes by its id.
    pub fn label(&self) -> &'static str {
        self.label.unwrap_or(self.id)
    }

    /// The attribution to show wherever the model is offered.
    pub fn credit(&self) -> Option<&'static str> {
        if self.id == PARAKEET.id {
            Some(
                "Parakeet Unified by NVIDIA. Licensed by NVIDIA Corporation under the NVIDIA Open Model License.",
            )
        } else {
            self.licence.credit
        }
    }
}

pub const DEFAULT_MODEL: &str = "parakeet-unified-en-0.6b";

pub const TURBO_ID: &str = "whisper-large-v3-turbo";

// Each sha256 is of the downloaded file, and the catalogue generator never
// changes one.
const PARAKEET: ListeningModel = ListeningModel {
    id: DEFAULT_MODEL,
    label: Some("Parakeet Unified"),
    repo: "handy-computer/parakeet-unified-en-0.6b-gguf",
    revision: "843b1b175ace04f5a89d379c79e2518c0d065b25",
    file: "parakeet-unified-en-0.6b-Q8_0.gguf",
    bytes: 731_357_568,
    sha256: "4b50b6dd862bf6e346929aaf4f5eaacec003bfa3f56462d6c874b41ef2f38795",
    licence: &catalogue::NVIDIA_OPEN_MODEL,
    languages: &["en"],
    translates: false,
    streams: true,
    takes_vocabulary: false,
};

const TURBO: ListeningModel = ListeningModel {
    id: TURBO_ID,
    label: Some("Whisper turbo"),
    repo: "handy-computer/whisper-large-v3-turbo-gguf",
    revision: "18ada752005f5fddef499256f9a458d0b37a203a",
    file: "whisper-large-v3-turbo-Q5_K_M.gguf",
    bytes: 619_628_096,
    sha256: "06fc9923c09fb3ae878ea12c34d1be3ed18fad039ab14b4b045c7b6c40af56b2",
    licence: &catalogue::MIT,
    languages: &[
        "af", "am", "ar", "as", "az", "ba", "be", "bg", "bn", "bo", "br", "bs", "ca", "cs", "cy",
        "da", "de", "el", "en", "es", "et", "eu", "fa", "fi", "fo", "fr", "gl", "gu", "haw", "ha",
        "he", "hi", "hr", "ht", "hu", "hy", "id", "is", "it", "ja", "jw", "ka", "kk", "km", "kn",
        "ko", "la", "lb", "ln", "lo", "lt", "lv", "mg", "mi", "mk", "ml", "mn", "mr", "ms", "mt",
        "my", "ne", "nl", "nn", "no", "oc", "pa", "pl", "ps", "pt", "ro", "ru", "sa", "sd", "si",
        "sk", "sl", "sn", "so", "sq", "sr", "su", "sv", "sw", "ta", "te", "tg", "th", "tk", "tl",
        "tr", "tt", "uk", "ur", "uz", "vi", "yi", "yo", "yue", "zh",
    ],
    translates: false,
    streams: false,
    takes_vocabulary: true,
};

pub const SUGGESTED: [ListeningModel; 2] = [PARAKEET, TURBO];

pub fn find(id: &str) -> Option<&'static ListeningModel> {
    CATALOGUE
        .iter()
        .find(|model| model.id.eq_ignore_ascii_case(id))
}

/// The model whose file is `file`, matched exactly: it is a name on disk.
pub fn find_file(file: &str) -> Option<&'static ListeningModel> {
    CATALOGUE.iter().find(|model| model.file == file)
}

#[cfg(test)]
mod tests {
    use super::{CATALOGUE, DEFAULT_MODEL, SUGGESTED, find};
    use std::collections::HashSet;

    #[test]
    fn a_download_url_pins_the_revision() {
        assert_eq!(
            SUGGESTED[0].url(),
            "https://huggingface.co/handy-computer/parakeet-unified-en-0.6b-gguf/resolve/843b1b175ace04f5a89d379c79e2518c0d065b25/parakeet-unified-en-0.6b-Q8_0.gguf"
        );
    }

    #[test]
    fn find_answers_a_known_id_and_refuses_any_other() {
        assert_eq!(
            find(DEFAULT_MODEL).map(|model| model.id),
            Some(DEFAULT_MODEL)
        );
        assert_eq!(
            find("whisper-large-v3-turbo").map(|model| model.file),
            Some("whisper-large-v3-turbo-Q5_K_M.gguf")
        );
        assert_eq!(find("ggml-large-v3-turbo-q5_0.bin"), None);
    }

    #[test]
    fn an_id_matches_whatever_its_case() {
        assert_eq!(
            find("qwen3-asr-0.6b").map(|model| model.id),
            Some("Qwen3-ASR-0.6B")
        );
        assert_eq!(
            find("PARAKEET-unified-en-0.6b").map(|model| model.id),
            Some(DEFAULT_MODEL)
        );
    }

    #[test]
    fn a_model_is_english_only_when_its_card_names_english_alone() {
        let english_only = |id| find(id).unwrap().english_only();
        assert!(english_only(DEFAULT_MODEL));
        assert!(!english_only("whisper-large-v3-turbo"));
        assert!(
            !english_only("gigaam-v3-ctc"),
            "Russian alone is not English"
        );
        assert!(!english_only("Breeze-ASR-25"));
    }

    #[test]
    fn the_catalogue_holds_73_speech_models() {
        assert_eq!(CATALOGUE.len(), 73);
    }

    /// A file is matched back to its model by name, so files must be unique too.
    #[test]
    fn every_id_is_unique() {
        let ids: HashSet<&str> = CATALOGUE.iter().map(|model| model.id).collect();
        let files: HashSet<&str> = CATALOGUE.iter().map(|model| model.file).collect();
        assert_eq!(ids.len(), CATALOGUE.len());
        assert_eq!(files.len(), CATALOGUE.len());
    }

    #[test]
    fn the_suggestions_match_the_pins() {
        let pins: Vec<_> = SUGGESTED
            .iter()
            .map(|model| {
                (
                    model.id,
                    model.repo,
                    model.revision,
                    model.file,
                    model.bytes,
                    model.sha256,
                    model.licence.name,
                )
            })
            .collect();
        assert_eq!(
            pins,
            [
                (
                    "parakeet-unified-en-0.6b",
                    "handy-computer/parakeet-unified-en-0.6b-gguf",
                    "843b1b175ace04f5a89d379c79e2518c0d065b25",
                    "parakeet-unified-en-0.6b-Q8_0.gguf",
                    731_357_568,
                    "4b50b6dd862bf6e346929aaf4f5eaacec003bfa3f56462d6c874b41ef2f38795",
                    "NVIDIA Open Model License (2025-10-24)",
                ),
                (
                    "whisper-large-v3-turbo",
                    "handy-computer/whisper-large-v3-turbo-gguf",
                    "18ada752005f5fddef499256f9a458d0b37a203a",
                    "whisper-large-v3-turbo-Q5_K_M.gguf",
                    619_628_096,
                    "06fc9923c09fb3ae878ea12c34d1be3ed18fad039ab14b4b045c7b6c40af56b2",
                    "MIT",
                ),
            ]
        );
        assert_eq!(&CATALOGUE[..2], &SUGGESTED[..]);
    }

    /// The licences read by hand, and which of them ask for acceptance.
    #[test]
    fn every_catalogue_licence_is_a_known_one() {
        let known = [
            ("MIT", false),
            ("Apache-2.0", false),
            ("CC-BY-4.0", false),
            ("CC-BY-SA-4.0", false),
            ("NVIDIA Open Model License (2025-10-24)", false),
            ("FunASR Model License 1.1", true),
            (
                "Health AI Developer Foundations Terms of Use (2024-11-15)",
                true,
            ),
            ("OpenMDW-1.1", false),
            ("CC-BY-NC-4.0", true),
            ("CC-BY-NC-SA-4.0", true),
            ("Moonshine AI Community License (2025-06-15)", true),
        ];
        for model in CATALOGUE {
            let licence = model.licence;
            assert!(
                known.contains(&(licence.name, licence.needs_acceptance)),
                "{} is under {licence:?}",
                model.id
            );
            assert!(licence.url.starts_with("https://"), "{licence:?}");
            assert!(!licence.summary.is_empty(), "{licence:?}");
        }
        assert_eq!(
            find("canary-1b").map(|model| model.licence.name),
            Some("CC-BY-NC-4.0")
        );
    }

    #[test]
    fn the_moonshine_summary_names_registration_and_the_revenue_limit() {
        let summary = find("moonshine-tiny-ja").unwrap().licence.summary;
        assert!(summary.contains("registration"), "{summary}");
        assert!(summary.contains("USD 1M"), "{summary}");
        assert!(summary.contains("affiliates"), "{summary}");
        assert_eq!(
            find("moonshine-tiny-ja").unwrap().licence.credit,
            Some("Powered by Moonshine AI")
        );
    }
}
