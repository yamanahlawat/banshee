use super::*;

// A mixer nobody reads: the output device is gone, so no sample is ever pulled
fn dead_mixer() -> rodio::mixer::Mixer {
    let (mixer, _never_read) = rodio::mixer::mixer(CHANNELS, SAMPLE_RATE);
    mixer
}

fn one_second_of_silence() -> impl Iterator<Item = Vec<f32>> + Send + 'static {
    std::iter::once(vec![0.0; SAMPLE_RATE.get() as usize])
}

fn engine_for(voice: &str) -> Option<KokoroEngine> {
    let config = KokoroTTSConfig::new(voice);
    match KokoroEngine::new(&config, 1.0) {
        Ok(engine) => Some(engine),
        Err(_) => {
            eprintln!("{voice} model not installed; skipping");
            None
        }
    }
}

#[test]
fn a_voice_swap_changes_the_style_and_swaps_back() {
    if installed("am_adam").is_err() {
        return;
    }
    let Some(mut engine) = engine_for("af_sky") else {
        return;
    };
    let first = engine.style_fingerprint();
    engine.set_voice("am_adam").expect("an installed voice");
    let second = engine.style_fingerprint();
    assert_ne!(first, second, "the fixture must discriminate");
    engine.set_voice("af_sky").expect("an installed voice");
    assert_eq!(engine.style_fingerprint(), first);
}

#[test]
fn an_uninstalled_voice_is_refused_and_leaves_the_engine_speaking() {
    let Some(mut engine) = engine_for("af_sky") else {
        return;
    };
    let before = engine.style_fingerprint();
    assert!(engine.set_voice("zz_nobody").is_err());
    assert_eq!(
        engine.style_fingerprint(),
        before,
        "a refused swap must not clear the voice"
    );
}

#[test]
fn an_ordinary_utterance_returns_the_engine_to_the_configured_voice() {
    if installed("am_adam").is_err() {
        return;
    }
    let Some(mut engine) = engine_for("af_sky") else {
        return;
    };
    let configured = engine.style_fingerprint();
    engine.ensure_voice("am_adam").expect("an installed voice");
    assert_ne!(
        engine.style_fingerprint(),
        configured,
        "the fixture must discriminate"
    );
    engine.ensure_voice("af_sky").expect("an installed voice");
    assert_eq!(engine.style_fingerprint(), configured);
}

#[test]
fn a_voice_outside_the_installed_set_is_refused_without_naming_a_path() {
    let Some(mut engine) = engine_for("af_sky") else {
        return;
    };
    let models = get_models_path().expect("a home directory");
    let error = engine
        .set_voice("../../../../etc/hosts")
        .unwrap_err()
        .to_string();
    assert!(
        !error.contains(models.to_str().expect("a printable path")),
        "the message must not name the path the daemon built: {error}"
    );
}

#[test]
fn a_voice_this_machine_does_not_hold_is_refused() {
    let held = vec!["af_sky".to_string(), "am_adam".to_string()];
    assert_eq!(voice_to_take("am_adam", &held), Some("am_adam".to_string()));
    assert_eq!(
        voice_to_take("am_nobody", &held),
        None,
        "a voice that is not installed would silence every later reply"
    );
}

#[test]
fn an_utterance_that_names_no_voice_takes_the_configured_one() {
    assert_eq!(voice_for(None, "am_adam"), "am_adam");
    assert_eq!(
        voice_for(Some("af_heart"), "am_adam"),
        "af_heart",
        "a named voice must win over the configured one"
    );
}

#[test]
fn ensure_voice_on_the_loaded_voice_reads_nothing() {
    let Some(mut engine) = engine_for("af_sky") else {
        return;
    };
    let before = engine.voice_ptr();
    engine.ensure_voice("af_sky").expect("already loaded");
    assert_eq!(
        engine.voice_ptr(),
        before,
        "the loaded voice must not be re-read"
    );
}

fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while !done() {
        assert!(
            std::time::Instant::now() < deadline,
            "{what} did not happen within 2s"
        );
        thread::sleep(std::time::Duration::from_millis(10));
    }
}

#[test]
fn a_stopped_utterance_on_a_dead_device_does_not_block_the_next_one() {
    let mixer = dead_mixer();
    let mut first = play(&mixer, one_second_of_silence());
    wait_until("the first sentence is queued", || first.player.len() == 1);
    first.stop();

    let second = play(&mixer, one_second_of_silence());
    wait_until("the second utterance's thread finishes", || {
        second.synth.is_finished()
    });
    assert_eq!(
        second.player.len(),
        1,
        "the second sentence was queued on its own player"
    );
}

#[test]
fn a_sentence_after_stop_is_never_appended() {
    let mixer = dead_mixer();
    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
    let chunks = std::iter::once(vec![0.0; 240]).chain(std::iter::once_with(move || {
        let _ = release_rx.recv();
        vec![0.0; 240]
    }));
    let mut utterance = play(&mixer, chunks);
    wait_until("the first chunk is queued", || utterance.player.len() == 1);
    utterance.stop();
    release_tx.send(()).unwrap();
    wait_until("the thread ends", || utterance.synth.is_finished());
    assert_eq!(utterance.player.len(), 1, "no chunk may follow a stop");
}

#[test]
fn flags_letter_spelled_words_only() {
    let mut spelled = MToken::new("nginx".into(), "NN".into(), " ".into());
    spelled.phonemes = Some("ˈɛn dʒˈiː ˈaɪ ɛn ɛks".into());
    assert!(is_letter_spelled(&spelled));

    let mut resolved = MToken::new("build".into(), "NN".into(), " ".into());
    resolved.phonemes = Some("bˈɪld".into());
    assert!(!is_letter_spelled(&resolved));

    let mut hyphenated = MToken::new("twenty-one".into(), "CD".into(), " ".into());
    hyphenated.phonemes = Some("twˈɛnti wˈʌn".into());
    assert!(!is_letter_spelled(&hyphenated));
}

// Skips unless espeak-ng is installed.
#[test]
fn espeak_resolves_a_letter_spelled_word() {
    let Some(oov) = OovFallback::detect() else {
        eprintln!("espeak-ng not installed; skipping");
        return;
    };
    let word = "kustomize"; // not in misaki gold or our tables
    let mut g2p = G2P::new(Language::EnglishUS);
    let spelled = g2p.g2p(word).unwrap().1;
    assert!(
        spelled.iter().any(is_letter_spelled),
        "expected {word} to start out letter-spelled"
    );

    let phonemes = oov.phonemize(word).expect("espeak should phonemize");
    g2p.lexicon
        .golds
        .insert(word.to_string(), PhonemeEntry::Simple(phonemes));

    let after = g2p.g2p(word).unwrap().1;
    assert!(
        !after.iter().any(is_letter_spelled),
        "espeak phonemes should have resolved {word}"
    );
}

#[test]
fn g2p_output_maps_into_vocab() {
    let g2p = G2P::new(Language::EnglishUS);
    let (phonemes, _) = g2p.g2p("Hello, world!").unwrap();
    let ids: Vec<i64> = phonemes.chars().filter_map(token_id).collect();
    assert!(!ids.is_empty(), "no phonemes mapped for: {phonemes}");
    // Most of the phoneme string should map; a low ratio means vocab drift
    assert!(ids.len() * 2 >= phonemes.chars().count());
}

// Needs the real model on disk: cargo test kokoro_synthesizes -- --ignored
#[test]
#[ignore]
fn kokoro_synthesizes_audible_speech() {
    let config = KokoroTTSConfig::new("af_sky");
    let mut engine = KokoroEngine::new(&config, 1.0).unwrap();
    let samples = engine.synthesize("Kokoro is alive.").unwrap();
    // ~1s of speech at 24kHz, with actual signal in it
    assert!(samples.len() > 10_000, "too few samples: {}", samples.len());
    let peak = samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    assert!(peak > 0.05, "output is near-silence, peak {peak}");
}

// The all-caps form is the one misaki tags NNP and spells out, so it is the one
// espeak would be asked about.
#[test]
fn a_curated_word_is_curated_in_every_casing() {
    let mut g2p = G2P::new(Language::EnglishUS);
    super::super::pronunciation::install_dictionary(&mut g2p);
    for word in ["webhook", "Webhook", "WEBHOOK"] {
        assert!(curated(&g2p.lexicon, word), "{word} must count as curated");
    }
    assert!(!curated(&g2p.lexicon, "Zzzq"));
}

#[test]
fn sentences_split_on_terminators() {
    let chunks: Vec<&str> = sentences("Done with the build. Tests pass! Ready?").collect();
    assert_eq!(
        chunks,
        vec!["Done with the build.", "Tests pass!", "Ready?"]
    );
    assert_eq!(
        sentences("no terminator").collect::<Vec<_>>(),
        vec!["no terminator"]
    );
}
