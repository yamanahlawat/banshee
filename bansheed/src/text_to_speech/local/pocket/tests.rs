use std::path::PathBuf;
use std::time::{Duration, Instant};

use banshee_common::POCKET_DIR;

use crate::models::download::pocket_files;
use crate::test_support::ignored_faults;
use crate::text_to_speech::Fault;
use crate::text_to_speech::output::SAMPLE_RATE;

use super::*;

const SAMPLES_PER_FRAME: usize = 1920;

const SAMPLE_LINES: [&str; 8] = [
    "Done. The tests pass and the branch is ready for review.",
    "I found the cause: the server turned the voice description into noise, so I cleared it and the replies came back clean.",
    "Oh, that's great news! I honestly didn't expect the fix to be that simple, but here we are.",
    "Hey, welcome back. How did the meeting go? You looked a little worried this morning.",
    "Hmm, I'm not sure that's right. Can you check the logs before we change anything else?",
    "Take your time. I'll be here when you're ready to pick it up again.",
    "Should I commit it now, or do you want to look at the diff first?",
    "Two tests failed in the config module. They fail on develop too, so my change didn't cause them.",
];

/// Ids from the same `tokenizer.json` read with the `onig` regex backend.
const SAMPLE_TOKENS: [&[u32]; 8] = [
    &[
        1201, 336, 263, 364, 1115, 261, 823, 269, 265, 260, 3935, 277, 1238, 284, 260, 2037, 263,
    ],
    &[
        268, 753, 265, 260, 320, 3244, 265, 1529, 295, 260, 1949, 265, 1483, 2872, 2757, 390, 3288,
        262, 294, 268, 1081, 278, 275, 269, 265, 1152, 2420, 261, 662, 382, 1671, 263,
    ],
    &[
        480, 262, 271, 264, 261, 481, 1569, 682, 268, 260, 1290, 323, 493, 264, 274, 260, 1095,
        265, 1959, 266, 279, 271, 260, 1595, 262, 305, 375, 281, 304, 263,
    ],
    &[
        1446, 262, 281, 1661, 382, 263, 668, 432, 265, 1134, 273, 350, 292, 343, 384, 278, 267,
        260, 415, 2863, 278, 285, 1220, 273, 263,
    ],
    &[
        993, 283, 283, 262, 268, 264, 283, 300, 575, 271, 264, 261, 345, 263, 1235, 270, 927, 265,
        260, 2215, 261, 260, 531, 281, 260, 665, 878, 273, 734, 292,
    ],
    &[
        2807, 312, 361, 263, 268, 264, 335, 279, 375, 348, 270, 264, 280, 1238, 266, 909, 275, 330,
        517, 263,
    ],
    &[
        446, 771, 725, 307, 268, 260, 3115, 275, 372, 262, 311, 299, 270, 347, 266, 384, 309, 265,
        1382, 443, 292,
    ],
    &[
        2995, 1115, 261, 2765, 278, 276, 265, 1709, 960, 712, 1693, 410, 263, 438, 2765, 288, 404,
        1626, 522, 262, 294, 313, 260, 665, 493, 264, 274, 260, 320, 360, 263,
    ],
];

fn pocket_dir() -> PathBuf {
    crate::models::download::models_dir()
        .expect("a home directory")
        .join(POCKET_DIR)
}

/// The engine, or the reason it could not load. Every caller is `#[ignore]`d,
/// so a machine that runs one wants the failure.
fn engine() -> Arc<Mutex<PocketEngine>> {
    let dir = pocket_dir();
    let engine = PocketEngine::new(&dir)
        .unwrap_or_else(|error| panic!("the Pocket files in {dir:?} did not load: {error}"));
    Arc::new(Mutex::new(engine))
}

#[test]
fn the_file_table_pins_both_repos() {
    let files = pocket_files();

    assert_eq!(files.len(), 7);
    for file in &files {
        assert!(
            file.url
                .contains("58a6d00cf13d239b6748cb0769f35c580a8f606c")
                || file
                    .url
                    .contains("a7056f505a4a8ca72d8b5fb1440071d6f1a35739"),
            "{} is not pinned to a revision",
            file.url
        );
        let local = file
            .name
            .strip_prefix("pocket-english_2026-04/")
            .unwrap_or_else(|| panic!("{} is outside the Pocket folder", file.name));
        assert!(
            file.url.ends_with(&format!("/{local}")),
            "{} keeps the remote name {}",
            file.name,
            file.url
        );
    }
    assert_eq!(files.iter().map(|file| file.megabytes).sum::<u64>(), 405);
}

#[test]
fn the_embedded_voice_is_119_frames_of_1024() {
    let (shape, values) = read_npy(COMPANION).expect("the embedded voice reads");

    assert_eq!(shape, [119, 1024]);
    assert_eq!(values.len(), 121_856);
}

#[test]
fn a_state_manifest_reads_from_bundle_json() {
    let bundle = read_bundle(
        br#"{
            "bundle_name": "english_2026-04",
            "max_token_per_chunk": 50,
            "flow_lm_state_manifest": [
                {"dtype": "float32", "fill": "nan", "index": 0, "input_name": "state_0",
                 "output_name": "out_state_0", "shape": [2, 1, 1000, 16, 64]},
                {"dtype": "float32", "fill": "empty", "index": 1, "input_name": "state_1",
                 "output_name": "out_state_1", "shape": [0]},
                {"dtype": "int64", "fill": "zeros", "index": 2, "input_name": "state_2",
                 "output_name": "out_state_2", "shape": [1]}
            ],
            "mimi_state_manifest": [
                {"dtype": "bool", "fill": "ones", "index": 0, "input_name": "state_0",
                 "output_name": "out_state_0", "shape": [1]}
            ]
        }"#,
    )
    .expect("the bundle reads");

    assert_eq!(bundle.max_token_per_chunk, 50);
    let flow = &bundle.flow_lm_state_manifest;
    assert_eq!(flow.len(), 3);
    assert_eq!(
        (flow[0].input.as_str(), flow[0].output.as_str()),
        ("state_0", "out_state_0")
    );
    assert_eq!(
        (flow[0].dtype, flow[0].fill, flow[0].shape.as_slice()),
        (Dtype::Float32, Fill::Nan, [2, 1, 1000, 16, 64].as_slice())
    );
    assert_eq!(flow[1].fill, Fill::Empty);
    assert_eq!(flow[1].shape, [0]);
    assert_eq!(
        (flow[2].input.as_str(), flow[2].dtype, flow[2].fill),
        ("state_2", Dtype::Int64, Fill::Zeros)
    );
    assert_eq!(
        (
            bundle.mimi_state_manifest[0].dtype,
            bundle.mimi_state_manifest[0].fill
        ),
        (Dtype::Bool, Fill::Ones)
    );
}

/// The frame a sentence ends at, given one EOS logit per frame.
fn ending_frame(mut ending: Ending, eos_logits: impl Fn(usize) -> f32) -> Option<usize> {
    (0..ending.cap).find(|&frame| ending.ends(frame, eos_logits(frame)))
}

#[test]
fn a_sentence_stops_after_its_eos_tail() {
    let quiet = -10.0;
    let heard = -3.9;
    let eos_only_at =
        |at: usize, logit: f32| move |frame: usize| if frame == at { logit } else { quiet };

    assert_eq!(
        ending_frame(Ending::new(30, 2), eos_only_at(3, heard)),
        None,
        "an EOS before frame 6 is not heard"
    );
    assert_eq!(
        ending_frame(Ending::new(30, 2), eos_only_at(6, -4.0)),
        None,
        "a logit at the threshold is not an EOS"
    );
    assert_eq!(
        ending_frame(Ending::new(30, 2), eos_only_at(6, heard)),
        Some(11),
        "four words or fewer run five frames past the EOS"
    );
    assert_eq!(
        ending_frame(Ending::new(30, 4), eos_only_at(6, heard)),
        Some(11)
    );
    assert_eq!(
        ending_frame(Ending::new(30, 5), eos_only_at(6, heard)),
        Some(9),
        "more than four words run three frames past the EOS"
    );
    assert_eq!(
        ending_frame(Ending::new(30, 2), |frame| if frame >= 7 {
            heard
        } else {
            quiet
        }),
        Some(12),
        "the first EOS counts, not a later one"
    );
    assert_eq!(
        Ending::new(17, 5).cap,
        96,
        "(17 / 3 + 2) * 12.5, rounded up"
    );
    assert_eq!(Ending::new(30, 5).cap, 150);
}

#[test]
fn text_with_no_tokens_speaks_nothing() {
    assert!(!speakable("…"));
    assert!(!speakable("🙂"));
    assert!(!speakable(" ?! "));
    assert!(speakable("Done."));
    assert!(speakable("42"));
}

#[test]
fn a_prompt_is_prepared_the_way_the_reference_does() {
    assert_eq!(prepare("  hello there  ").as_deref(), Some("Hello there."));
    assert_eq!(
        prepare("line one\nline two").as_deref(),
        Some("Line one line two.")
    );
    assert_eq!(prepare("one\r\ntwo").as_deref(), Some("One two."));
    assert_eq!(prepare("two  spaces").as_deref(), Some("Two spaces."));
    assert_eq!(
        prepare("three   spaces").as_deref(),
        Some("Three  spaces."),
        "one pass of the reference's replace leaves a double space"
    );
    assert_eq!(prepare("ends with 42").as_deref(), Some("Ends with 42."));
    assert_eq!(prepare("Done!").as_deref(), Some("Done!"));
    assert_eq!(
        prepare("Is it \"done\"").as_deref(),
        Some("Is it \"done\""),
        "a closing quote is not alphanumeric, so no period follows it"
    );
    assert_eq!(prepare(""), None);
    assert_eq!(prepare("  \n "), None);
}

#[test]
fn boundaries_fall_after_each_run_of_end_tokens() {
    let ends = HashSet::from([9]);

    assert_eq!(boundaries(&[1, 9, 9, 2, 3, 9], &ends), [0, 3, 6]);
    assert_eq!(boundaries(&[9, 1], &ends), [0, 1, 2]);
    assert_eq!(boundaries(&[1, 2], &ends), [0, 2]);
    assert_eq!(boundaries(&[], &ends), [0, 0]);
}

#[test]
fn segments_pack_greedily_within_the_limit() {
    let segments = |list: &[(usize, &str)]| {
        list.iter()
            .map(|&(count, text)| (count, text.to_string()))
            .collect::<Vec<_>>()
    };

    assert_eq!(
        pack(segments(&[(20, "A."), (20, "B."), (20, "C.")]), 50),
        segments(&[(40, "A. B."), (20, "C.")])
    );
    assert_eq!(
        pack(segments(&[(60, "Long."), (10, "Short.")]), 50),
        segments(&[(60, "Long."), (10, "Short.")]),
        "a segment over the limit stands alone"
    );
    assert_eq!(
        pack(segments(&[(25, "A."), (25, "B.")]), 50),
        segments(&[(50, "A. B.")]),
        "the limit itself fits"
    );
    assert_eq!(pack(segments(&[(0, "")]), 50), segments(&[]));
}

#[test]
fn a_voice_other_than_the_companion_is_refused() {
    let refused = refuse_other_voices(Some("af_sky")).expect_err("af_sky is not Pocket's");
    assert!(matches!(refused, BansheeError::Rejected(_)), "{refused:?}");
    assert!(refuse_other_voices(Some(COMPANION_VOICE)).is_ok());
    assert!(refuse_other_voices(None).is_ok());
}

#[test]
#[ignore = "needs the Pocket files"]
fn the_companion_speaks_a_line() {
    let engine = engine();
    let line = "Banshee speaks with the companion voice.";

    for pass in ["first after load", "second"] {
        let started = Instant::now();
        let mut chunks = chunks(Arc::clone(&engine), line, ignored_faults());
        let first = chunks.next().expect("the line speaks");
        let first_at = started.elapsed();
        let rest: Vec<Chunk> = chunks.collect();
        let took = started.elapsed();

        let samples = first.samples.len() + rest.iter().map(|c| c.samples.len()).sum::<usize>();
        let seconds = samples as f64 / f64::from(SAMPLE_RATE.get());
        eprintln!(
            "{pass}: first chunk {:.0} ms, {seconds:.2} s of audio in {:.2} s = {:.1}x real time",
            first_at.as_secs_f64() * 1e3,
            took.as_secs_f64(),
            seconds / took.as_secs_f64()
        );
        assert!(seconds > 1.0, "{seconds} s of audio");
        assert_eq!(first.samples.len(), 2 * SAMPLES_PER_FRAME);
        assert!(
            !rest.is_empty(),
            "the first chunk came before the sentence ended"
        );
    }
}

#[test]
#[ignore = "needs the Pocket files"]
fn a_stop_ends_at_the_next_chunk() {
    let engine = engine();
    let at_load = lock(&engine).runs();
    let mut chunks = chunks(
        Arc::clone(&engine),
        "This first sentence runs long enough to need several chunks. A second one follows it.",
        ignored_faults(),
    );

    chunks.next().expect("the first chunk");
    let after_the_first = lock(&engine).runs();
    drop(chunks);
    std::thread::sleep(Duration::from_millis(300));

    assert_eq!(
        after_the_first - at_load,
        2 + 2 * (1 + DECODING_STEPS) + 1,
        "the conditioner, the prompt pass, two frames and one decode, nothing ahead"
    );
    assert_eq!(
        lock(&engine).runs(),
        after_the_first,
        "nothing ran after the stop"
    );
}

#[test]
#[ignore = "needs the Pocket files"]
fn tokens_match_the_onig_tokenizer() {
    let tokenizer = Tokenizer::from_file(pocket_dir().join("tokenizer.json"))
        .unwrap_or_else(|error| panic!("tokenizer.json did not load: {error}"));

    for (line, expected) in SAMPLE_LINES.iter().zip(SAMPLE_TOKENS) {
        assert_eq!(token_ids(&tokenizer, line).unwrap(), expected, "{line}");
    }
}

fn splitter() -> Splitter {
    let dir = pocket_dir();
    let bundle = read_bundle(&std::fs::read(dir.join("bundle.json")).expect("bundle.json"))
        .expect("bundle.json reads");
    let tokenizer = Tokenizer::from_file(dir.join("tokenizer.json"))
        .unwrap_or_else(|error| panic!("tokenizer.json did not load: {error}"));
    Splitter::new(tokenizer, bundle.max_token_per_chunk).expect("the splitter builds")
}

/// The expected chunks are the reference's `_split_into_best_sentences`, run
/// with sentencepiece on the bundle's `tokenizer.model`.
#[test]
#[ignore = "needs the Pocket files"]
fn text_splits_the_way_the_reference_does() {
    let splitter = splitter();
    assert_eq!(splitter.limit, 50);
    assert_eq!(splitter.sentence_ends, HashSet::from([263, 682, 799, 292]));
    assert_eq!(splitter.clause_ends, HashSet::from([262, 1230, 3244]));

    assert_eq!(
        splitter
            .split("Done. The tests pass. Ready for review.")
            .unwrap(),
        ["Done. The tests pass. Ready for review."],
        "short sentences pack into one chunk"
    );
    assert_eq!(
        splitter
            .split(
                "I found the cause in the config loader, the server had turned the voice \
                 description into noise, so I cleared the instructions field and restarted the \
                 daemon, then I ran the whole test suite twice, and every reply came back clean \
                 on the first try, which was a relief after such a long afternoon."
            )
            .unwrap(),
        [
            "I found the cause in the config loader, the server had turned the voice description \
             into noise, so I cleared the instructions field and restarted the daemon,",
            "then I ran the whole test suite twice, and every reply came back clean on the first \
             try, which was a relief after such a long afternoon.",
        ],
        "a sentence over the limit splits at its commas, then packs"
    );
    assert_eq!(
        splitter.split("hello there\nthis is  a test").unwrap(),
        ["Hello there this is a test."]
    );
    assert!(splitter.split("").unwrap().is_empty());
    assert!(splitter.split("   \n  ").unwrap().is_empty());
}

#[test]
#[ignore = "needs the Pocket files"]
fn a_run_with_no_punctuation_splits_between_words_within_the_limit() {
    let splitter = splitter();
    let run = vec!["then the build ran again"; 40].join(" and ");
    let tokens = |text: &str| token_ids(&splitter.tokenizer, text).unwrap().len();
    assert!(tokens(&run) > 200, "the run is {} tokens", tokens(&run));

    let chunks = splitter.split(&run).unwrap();

    assert!(chunks.len() >= 5, "{} chunks", chunks.len());
    for chunk in &chunks {
        assert!(tokens(chunk) <= 50, "{} tokens in {chunk:?}", tokens(chunk));
    }
    assert_eq!(
        chunks.join(" ").split_whitespace().collect::<Vec<_>>(),
        prepare(&run)
            .unwrap()
            .split_whitespace()
            .collect::<Vec<_>>(),
        "every word survives, in order"
    );
}

#[test]
#[ignore = "needs the Pocket files"]
fn a_run_on_reply_plays_to_the_end() {
    let engine = engine();
    let run = vec!["then the build ran again"; 40].join(" and ");
    let (faults, reported) = std::sync::mpsc::channel();

    let samples: usize = chunks(Arc::clone(&engine), &run, Faults::new(faults))
        .map(|chunk| chunk.samples.len())
        .sum();

    let failures: Vec<String> = reported
        .try_iter()
        .filter_map(|fault| match fault {
            Fault::Failed(reason) => Some(reason),
            _ => None,
        })
        .collect();
    let seconds = samples as f64 / f64::from(SAMPLE_RATE.get());
    eprintln!("the run-on reply played {seconds:.1} s");
    assert_eq!(failures, Vec::<String>::new());
    assert!(seconds > 30.0, "{seconds} s");
}
