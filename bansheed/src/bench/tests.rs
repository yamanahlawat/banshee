use super::*;
use crate::text_to_speech::local::kokoro::{MAX_TOKENS, is_letter_spelled, sentences, token_ids};

#[test]
fn no_bench_text_holds_a_word_misaki_spells_out() {
    let g2p = english_g2p();
    for text in [SHORT, MEDIUM, LONG] {
        let mut texts = vec![text.to_string()];
        texts.extend(text_path(text));
        for sentence in texts {
            let (_, tokens) = g2p.g2p(&sentence).unwrap();
            let spelled: Vec<_> = tokens
                .iter()
                .filter(|tk| is_letter_spelled(tk))
                .map(|tk| tk.text.clone())
                .collect();
            assert!(spelled.is_empty(), "spelled out: {spelled:?}");
        }
    }
}

#[test]
fn the_long_text_is_one_sentence_longer_than_one_window() {
    assert_eq!(sentences(LONG).count(), 1);
    let (phonemes, _) = english_g2p().g2p(LONG).unwrap();
    let ids = token_ids(&phonemes).len();
    assert!(
        ids > MAX_TOKENS,
        "{ids} tokens fit one window of {MAX_TOKENS}"
    );
}

#[test]
fn the_short_text_is_one_sentence_and_the_medium_text_is_several() {
    assert_eq!(text_path(SHORT).len(), 1);
    assert!(text_path(MEDIUM).len() > 1);
}

#[test]
fn a_models_folder_without_the_voice_names_the_voice() {
    let (_, config) = defaults();
    let dir = crate::test_support::unique_scratch("bench-models");
    std::fs::write(dir.join(&config.model_name), b"").unwrap();
    assert_eq!(
        missing_file(&dir),
        Some(format!("{} is not in the models folder", config.voice_name))
    );
}

#[test]
fn a_models_folder_with_the_model_and_voice_misses_nothing() {
    let (_, config) = defaults();
    let dir = crate::test_support::unique_scratch("bench-models");
    std::fs::write(dir.join(&config.model_name), b"").unwrap();
    std::fs::write(dir.join(&config.voice_name), b"").unwrap();
    assert_eq!(missing_file(&dir), None);
}

#[test]
fn the_run_facts_name_the_voice_speed_threads_and_espeak() {
    let facts = run_facts();
    let (tts, _) = defaults();
    let expected = [
        format!("voice {}", tts.voice),
        format!("speed {}", tts.speed),
        format!("threads {}", crate::models::kokoro_threads()),
    ];
    for part in &expected {
        assert!(facts.contains(part), "{facts:?} lacks {part:?}");
    }
    assert!(
        facts.ends_with("espeak-ng found") || facts.ends_with("espeak-ng absent"),
        "{facts:?} does not say whether espeak-ng is present"
    );
}
