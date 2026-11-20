use super::*;
use crate::client::{Caller, Process};
use crate::speech_to_text::Route;
use crate::state::RecordingMode;
use crate::test_support::daemon_state as test_state;

fn request(method: &str, params: Option<serde_json::Value>) -> JsonRpcRequest {
    JsonRpcRequest {
        jsonrpc: banshee_common::Version::V2,
        method: method.to_string(),
        params,
        id: Some(serde_json::json!(1)),
    }
}

#[tokio::test]
async fn history_honours_a_limit() {
    let state = crate::test_support::daemon_state_with_history(&["first", "second", "third"]);

    let request = request(BANSHEE_HISTORY, Some(serde_json::json!({ "limit": 1 })));
    let response = dispatch(request, &state, &Caller::unread()).await;

    let JsonRpcResponse::Success { result, .. } = response else {
        panic!("expected success response");
    };
    let history = result["history"].as_array().expect("a history array");
    assert_eq!(history.len(), 1);
    assert_eq!(history[0]["text"], "third");
}

#[tokio::test]
async fn a_flag_that_is_not_a_boolean_is_refused() {
    let state = test_state(std::sync::mpsc::channel().0);

    for (method, flag, params) in [
        (
            BANSHEE_SPEAK,
            "interrupt",
            serde_json::json!({"text": "hi", "interrupt": "true"}),
        ),
        (
            BANSHEE_CONFIGURE,
            "persist",
            serde_json::json!({"settings": {}, "persist": "true"}),
        ),
        (
            BANSHEE_CONNECT_PLAN,
            "disconnect",
            serde_json::json!({"agent": "cursor", "disconnect": "true"}),
        ),
        (
            BANSHEE_CONNECT_APPLY,
            "disconnect",
            serde_json::json!({"agent": "cursor", "disconnect": "true"}),
        ),
        (
            BANSHEE_TURN_ENDED,
            "repeat",
            serde_json::json!({"agent_pid": 7, "repeat": "yes"}),
        ),
    ] {
        let JsonRpcResponse::Error { error, .. } =
            dispatch(request(method, Some(params)), &state, &Caller::unread()).await
        else {
            panic!("a string {flag} must not be read as a flag by {method}");
        };
        assert_eq!(error.code, rpc_code::INVALID_PARAMS, "{method}");
        assert!(error.message.contains(flag), "{method}: {}", error.message);
    }
}

#[tokio::test]
async fn history_filters_by_kind() {
    let state = crate::test_support::daemon_state_with_history(&["dictated"]);
    state.keep(&crate::history::NewEntry::question(
        "Open the pull request?",
        Some("Yes"),
        crate::history::Outcome::Transcribed,
        None,
    ));

    let request = request(
        BANSHEE_HISTORY,
        Some(serde_json::json!({ "kind": "question" })),
    );
    let JsonRpcResponse::Success { result, .. } =
        dispatch(request, &state, &Caller::unread()).await
    else {
        panic!("expected success response");
    };
    let history = result["history"].as_array().expect("a history array");
    assert_eq!(history.len(), 1);
    assert_eq!(history[0]["question"], "Open the pull request?");
}

#[tokio::test]
async fn an_unknown_kind_is_refused() {
    let state = test_state(std::sync::mpsc::channel().0);

    let request = request(
        BANSHEE_HISTORY,
        Some(serde_json::json!({ "kind": "update" })),
    );
    let JsonRpcResponse::Error { error, .. } = dispatch(request, &state, &Caller::unread()).await
    else {
        panic!("an unknown kind must be refused");
    };
    assert_eq!(error.code, rpc_code::INVALID_PARAMS);
    assert!(error.message.contains("kind"), "{}", error.message);
}

async fn listed_rows(state: &Arc<DaemonState>) -> usize {
    let JsonRpcResponse::Success { result, .. } = dispatch(
        request(BANSHEE_HISTORY, Some(serde_json::json!({}))),
        state,
        &Caller::unread(),
    )
    .await
    else {
        panic!("expected success response");
    };
    result["history"].as_array().expect("a history array").len()
}

#[tokio::test]
async fn history_lists_and_clears_while_saving_is_off() {
    let state = crate::test_support::daemon_state_with_history_off(&["kept"]);
    assert_eq!(listed_rows(&state).await, 1);

    let cleared = dispatch(
        request(BANSHEE_CLEAR_HISTORY, Some(serde_json::json!({}))),
        &state,
        &Caller::unread(),
    )
    .await;
    assert!(matches!(cleared, JsonRpcResponse::Success { .. }));
    assert_eq!(listed_rows(&state).await, 0);
}

#[tokio::test]
async fn a_history_limit_that_does_not_fit_is_refused() {
    let state = crate::test_support::daemon_state_with_history(&["first"]);

    let request = request(
        BANSHEE_HISTORY,
        Some(serde_json::json!({ "limit": 4_294_967_296u64 })),
    );
    let JsonRpcResponse::Error { error, .. } = dispatch(request, &state, &Caller::unread()).await
    else {
        panic!("a limit past 32 bits must not truncate");
    };
    assert_eq!(error.code, rpc_code::INVALID_PARAMS);
}

#[tokio::test]
async fn history_takes_an_explicit_zero_literally() {
    let state = crate::test_support::daemon_state_with_history(&["first", "second", "third"]);

    let request = request(BANSHEE_HISTORY, Some(serde_json::json!({ "limit": 0 })));
    let response = dispatch(request, &state, &Caller::unread()).await;

    let JsonRpcResponse::Success { result, .. } = response else {
        panic!("expected success response");
    };
    let history = result["history"].as_array().expect("a history array");
    assert!(
        history.is_empty(),
        "an explicit zero must not mean every row"
    );
}

#[tokio::test]
async fn ask_user_lets_speech_from_the_same_turn_finish() {
    let (commands, command_receiver) = std::sync::mpsc::channel();
    let crate::test_support::HoldingSpeech {
        state, cut_short, ..
    } = crate::test_support::daemon_state_holding_speech(
        std::time::Duration::from_millis(80),
        commands,
    );

    state
        .speech()
        .speak("here is what I would change", false, None)
        .expect("the status starts playing");

    let session_state = Arc::clone(&state);
    std::thread::spawn(move || {
        let Ok(ConsumerCommand::Ask(ask)) = command_receiver.recv() else {
            return;
        };
        session_state.set_recording_mode(RecordingMode::Idle);
        let _ = ask.reply.send(Ok("say it is idle".to_string()));
    });

    let request = request(
        BANSHEE_ASK_USER,
        Some(serde_json::json!({"question": "What should it say?"})),
    );
    let response = dispatch(request, &state, &Caller::unread()).await;

    assert!(
        matches!(response, JsonRpcResponse::Success { .. }),
        "the question is still asked"
    );
    assert!(
        !cut_short.load(std::sync::atomic::Ordering::SeqCst),
        "a status from the same turn must not be cut off mid-sentence"
    );
}

#[tokio::test]
async fn a_line_sent_while_the_question_waits_plays_after_the_answer() {
    let (commands, command_receiver) = std::sync::mpsc::channel();
    let crate::test_support::HoldingSpeech {
        state,
        cut_short,
        spoken,
    } = crate::test_support::daemon_state_holding_speech(
        std::time::Duration::from_millis(300),
        commands,
    );
    state
        .speech()
        .speak("Agent C reports.", false, None)
        .unwrap();

    let listening = std::thread::spawn({
        let state = Arc::clone(&state);
        let spoken = Arc::clone(&spoken);
        move || {
            let Ok(ConsumerCommand::Ask(ask)) =
                command_receiver.recv_timeout(std::time::Duration::from_secs(5))
            else {
                panic!("the ask never reached the listener");
            };
            let heard_before_the_answer = spoken.lock().unwrap().clone();
            state.disarm(ask.session);
            let _ = ask.reply.send(Ok("yes".to_string()));
            heard_before_the_answer
        }
    });
    let asking = spawn_ask(&state);
    wait_for(&state, "the session arms", |state| {
        state.recording_mode() == RecordingMode::Armed
    })
    .await;
    state
        .speech()
        .speak("Agent B reports.", false, None)
        .unwrap();

    asking.await.unwrap();
    assert_eq!(
        listening.join().unwrap(),
        ["Agent C reports.", "Ready to ship?"],
        "a line sent after the question waits for the answer"
    );
    assert!(!cut_short.load(std::sync::atomic::Ordering::SeqCst));
}

#[tokio::test]
async fn a_stalled_status_is_skipped_and_the_question_still_plays() {
    let held = crate::test_support::daemon_state_holding_speech(
        std::time::Duration::from_millis(50),
        std::sync::mpsc::channel().0,
    );
    let speech = held.state.speech();
    speech.speak("Agent C reports.", false, None).unwrap();
    speech
        .speak(crate::test_support::STALLS, false, None)
        .unwrap();
    speech.speak("Agent B reports.", false, None).unwrap();
    let question = speech.speak_and_hold("Ready to ship?").unwrap();

    let asked = question_played(
        &held.state,
        question,
        std::time::Duration::from_secs(5),
        std::time::Duration::from_secs(1),
    )
    .await;

    assert_eq!(
        asked,
        Asked::Played,
        "a stalled status must not cost the question"
    );
    assert_eq!(
        *held.spoken.lock().unwrap(),
        [
            "Agent C reports.",
            crate::test_support::STALLS,
            "Ready to ship?"
        ]
    );
}

#[tokio::test]
async fn a_question_sent_while_the_speaker_switches_plays_after_the_switch() {
    let (state, _spoken) =
        crate::test_support::daemon_state_recording_speech_for(std::sync::mpsc::channel().0);
    let spoken_after = crate::test_support::SpokenLines::default();
    let lines = spoken_after.clone();
    let (build, building, gate) = crate::test_support::gated_builder(move |_| {
        crate::test_support::speaking_selection(
            lines.clone(),
            crate::text_to_speech::Speaker::Fallback,
        )
    });
    state.speech().set_builder(build);
    let speech = Arc::clone(state.speech());
    let switching = std::thread::spawn(move || {
        speech.switch(&crate::config::TTSConfig::default(), false);
    });
    building
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("the switch never began its build");
    let question = state.speech().speak_and_hold("Ready to ship?").unwrap();

    let asking = question_played(
        &state,
        question,
        std::time::Duration::from_secs(5),
        std::time::Duration::from_secs(5),
    );
    tokio::pin!(asking);
    let first = std::future::poll_fn(|cx| std::task::Poll::Ready(asking.as_mut().poll(cx))).await;
    assert!(
        first.is_pending(),
        "answered {first:?} while the question waited for the switch"
    );
    gate.send(()).unwrap();
    switching.join().unwrap();

    assert_eq!(asking.await, Asked::Played);
    assert_eq!(*spoken_after.lock().unwrap(), ["Ready to ship?"]);
}

/// The stall bound is 3 s, and the answer has to come within 1 s of the
/// switch's end: the switch itself wakes the wait.
#[tokio::test]
async fn a_question_whose_new_voice_will_not_start_is_answered_at_the_switch() {
    let (state, _spoken) =
        crate::test_support::daemon_state_recording_speech_for(std::sync::mpsc::channel().0);
    let (build, building, gate) =
        crate::test_support::gated_builder(|_| crate::text_to_speech::Selection {
            backend: Box::new(crate::test_support::Refusing),
            speaker: crate::text_to_speech::Speaker::Fallback,
            fault: None,
        });
    state.speech().set_builder(build);
    let speech = Arc::clone(state.speech());
    let switching = std::thread::spawn(move || {
        speech.switch(&crate::config::TTSConfig::default(), false);
    });
    building
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("the switch never began its build");
    let question = state.speech().speak_and_hold("Ready to ship?").unwrap();

    let asking = question_played(
        &state,
        question,
        std::time::Duration::from_secs(5),
        std::time::Duration::from_secs(3),
    );
    tokio::pin!(asking);
    let first = std::future::poll_fn(|cx| std::task::Poll::Ready(asking.as_mut().poll(cx))).await;
    assert!(
        first.is_pending(),
        "answered {first:?} while the question waited for the switch"
    );
    gate.send(()).unwrap();
    switching.join().unwrap();
    let switched = std::time::Instant::now();

    assert_eq!(asking.await, Asked::Unspoken);
    assert!(
        switched.elapsed() < std::time::Duration::from_secs(1),
        "answered {:?} after the switch",
        switched.elapsed()
    );
}

#[tokio::test]
async fn a_question_that_stalls_ends_on_its_own_budget() {
    let held = crate::test_support::daemon_state_holding_speech(
        std::time::Duration::from_millis(50),
        std::sync::mpsc::channel().0,
    );
    let question = held
        .state
        .speech()
        .speak_and_hold(crate::test_support::STALLS)
        .unwrap();

    let started = std::time::Instant::now();
    let asked = question_played(
        &held.state,
        question,
        std::time::Duration::from_millis(100),
        std::time::Duration::from_secs(5),
    )
    .await;

    assert_eq!(
        asked,
        Asked::Stalled,
        "a stalled backend must not hold the microphone"
    );
    assert!(started.elapsed() < std::time::Duration::from_secs(2));
    held.state.speech().stop();
}

#[tokio::test]
async fn a_line_released_after_the_question_ends_the_wait() {
    let held = crate::test_support::daemon_state_holding_speech(
        std::time::Duration::from_millis(50),
        std::sync::mpsc::channel().0,
    );
    let speech = held.state.speech();
    let question = speech.speak_and_hold("Ready to ship?").unwrap();
    speech
        .speak(crate::test_support::STALLS, false, None)
        .unwrap();
    speech.release();

    let started = std::time::Instant::now();
    let asked = question_played(
        &held.state,
        question,
        std::time::Duration::from_secs(5),
        std::time::Duration::from_secs(5),
    )
    .await;

    assert_eq!(asked, Asked::Played);
    assert!(started.elapsed() < std::time::Duration::from_secs(2));
    assert!(
        !held.cut_short.load(std::sync::atomic::Ordering::SeqCst),
        "a line after the question is not the ask's to skip"
    );
    speech.stop();
}

#[tokio::test]
async fn an_interrupt_that_drops_the_question_is_an_error_not_a_silent_listen() {
    let (commands, command_receiver) = std::sync::mpsc::channel();
    let state = crate::test_support::daemon_state_holding_speech(
        std::time::Duration::from_millis(300),
        commands,
    )
    .state;
    state
        .speech()
        .speak("Agent C reports.", false, None)
        .unwrap();

    let asking = spawn_ask(&state);
    wait_for(&state, "the session arms", |state| {
        state.recording_mode() == RecordingMode::Armed
    })
    .await;
    state
        .speech()
        .speak("Agent B interrupts.", true, None)
        .unwrap();

    let response = tokio::time::timeout(std::time::Duration::from_secs(5), asking)
        .await
        .expect("an ask with nobody to answer must not wait on a listen")
        .unwrap();
    let JsonRpcResponse::Error { error, .. } = response else {
        panic!("a question nobody heard must not open the microphone");
    };
    assert_eq!(error.message, "The question was not spoken.");
    assert!(command_receiver.try_recv().is_err());
}

#[tokio::test]
async fn a_barge_in_before_the_question_still_takes_the_answer() {
    let (commands, command_receiver) = std::sync::mpsc::channel();
    let state = crate::test_support::daemon_state_holding_speech(
        std::time::Duration::from_millis(300),
        commands,
    )
    .state;
    state
        .speech()
        .speak("Agent C reports.", false, None)
        .unwrap();

    let listening = std::thread::spawn({
        let state = Arc::clone(&state);
        move || {
            let Ok(ConsumerCommand::Ask(ask)) =
                command_receiver.recv_timeout(std::time::Duration::from_secs(5))
            else {
                panic!("the answer must still be taken");
            };
            state.disarm(ask.session);
            let _ = ask.reply.send(Ok("ship it".to_string()));
        }
    });
    let asking = spawn_ask(&state);
    wait_for(&state, "the session arms", |state| {
        state.recording_mode() == RecordingMode::Armed
    })
    .await;
    assert!(state.record_start(), "the press holds to answer");

    let response = asking.await.unwrap();
    listening.join().unwrap();
    assert!(
        matches!(response, JsonRpcResponse::Success { .. }),
        "{response:?}"
    );
}

#[tokio::test]
async fn another_agent_speaks_only_after_the_answer_closes() {
    let (commands, command_receiver) = std::sync::mpsc::channel();
    let (state, spoken) = crate::test_support::daemon_state_recording_speech_for(commands);

    // Stand-in for the consumer thread: another agent speaks while it listens
    let session_state = Arc::clone(&state);
    let listening = std::thread::spawn(move || {
        let Ok(ConsumerCommand::Ask(ask)) = command_receiver.recv() else {
            panic!("the ask never reached the listener");
        };
        session_state
            .speech()
            .speak("Agent B reports.", false, None)
            .unwrap();
        let heard_while_listening = session_state.speech().is_speaking();
        session_state.disarm(ask.session);
        let _ = ask.reply.send(Ok("yes".to_string()));
        heard_while_listening
    });

    let request = request(
        BANSHEE_ASK_USER,
        Some(serde_json::json!({"question": "Ready to ship?"})),
    );
    dispatch(request, &state, &Caller::unread()).await;

    assert!(
        !listening.join().unwrap(),
        "speech while the answer listens deafens the microphone"
    );
    assert_eq!(
        *spoken.lock().unwrap(),
        ["Ready to ship?", "Agent B reports."]
    );
}

#[tokio::test]
async fn ask_user_returns_the_scoped_answer() {
    let (commands, command_receiver) = std::sync::mpsc::channel();
    let state = test_state(commands);

    // Stand-in for the consumer thread: answer and disarm like a session
    let session_state = Arc::clone(&state);
    std::thread::spawn(move || {
        let Ok(ConsumerCommand::Ask(ask)) = command_receiver.recv() else {
            return;
        };
        assert_eq!(session_state.recording_mode(), RecordingMode::Armed);
        session_state.set_recording_mode(RecordingMode::Idle);
        let _ = ask.reply.send(Ok("yes, ship it".to_string()));
    });

    let request = request(
        BANSHEE_ASK_USER,
        Some(serde_json::json!({"question": "Ready to ship?"})),
    );
    let response = dispatch(request, &state, &Caller::unread()).await;

    let JsonRpcResponse::Success { result, .. } = response else {
        panic!("expected success response");
    };
    assert_eq!(result["text"], "yes, ship it");
    assert_eq!(state.recording_mode(), RecordingMode::Idle);
}

#[tokio::test]
async fn a_second_ask_waits_for_the_first_answer_and_then_listens() {
    let (commands, command_receiver) = std::sync::mpsc::channel();
    let state = test_state(commands);
    let (answers, answer_next) = std::sync::mpsc::channel::<&'static str>();
    let listener = answer_in_turn(&state, command_receiver, answer_next);

    let first = first_armed(&state, "Ready to ship?").await;
    let second = spawn_ask(&state);
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert!(
        !second.is_finished(),
        "the second ask must wait, not refuse"
    );

    answers.send("first answer").unwrap();
    assert_eq!(answer_of(first).await, "first answer");
    answers.send("second answer").unwrap();
    assert_eq!(answer_of(second).await, "second answer");
    drop(answers);
    listener.join().unwrap();
}

#[tokio::test]
async fn waiting_asks_play_in_arrival_order_and_one_whose_caller_leaves_never_plays() {
    let (commands, command_receiver) = std::sync::mpsc::channel();
    let crate::test_support::HoldingSpeech { state, spoken, .. } =
        crate::test_support::daemon_state_holding_speech(
            std::time::Duration::from_millis(10),
            commands,
        );
    let (answers, answer_next) = std::sync::mpsc::channel::<&'static str>();
    let listener = answer_in_turn(&state, command_receiver, answer_next);

    let first = first_armed(&state, "First question").await;
    let second = spawn_question(&state, "Second question");
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    let gone = spawn_question(&state, "Gone question");
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    let third = spawn_question(&state, "Third question");
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    gone.abort();

    answers.send("one").unwrap();
    answer_of(first).await;
    answers.send("two").unwrap();
    assert_eq!(answer_of(second).await, "two");
    answers.send("three").unwrap();
    assert_eq!(answer_of(third).await, "three");
    assert_eq!(
        *spoken.lock().unwrap(),
        ["First question", "Second question", "Third question"]
    );
    drop(answers);
    listener.join().unwrap();
}

#[tokio::test]
async fn a_waiting_ask_answers_a_broken_microphone_at_once() {
    let (commands, command_receiver) = std::sync::mpsc::channel();
    let state = test_state(commands);
    let (answers, answer_next) = std::sync::mpsc::channel::<&'static str>();
    let listener = answer_in_turn(&state, command_receiver, answer_next);

    let first = first_armed(&state, "Ready to ship?").await;
    let waiting = spawn_ask(&state);
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    break_the_microphone_for(&state, waiting).await;
    assert_eq!(state.recording_mode(), RecordingMode::Armed);

    answers.send("first answer").unwrap();
    answer_of(first).await;
    drop(answers);
    listener.join().unwrap();
}

#[tokio::test]
async fn an_ask_waiting_out_a_dictation_answers_a_broken_microphone_at_once() {
    let state = test_state(std::sync::mpsc::channel().0);
    state.set_recording_mode(RecordingMode::PushToTalk);

    let waiting = spawn_ask(&state);
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    break_the_microphone_for(&state, waiting).await;
}

#[tokio::test]
async fn an_ask_waits_until_the_dictation_leaves_the_ring() {
    let (commands, command_receiver) = std::sync::mpsc::channel();
    let state = test_state(commands);
    let (take_the_ring, ring_wanted) = std::sync::mpsc::channel::<()>();
    let consumer = std::thread::spawn({
        let state = Arc::clone(&state);
        move || {
            let Ok(ConsumerCommand::Stream(_)) =
                command_receiver.recv_timeout(std::time::Duration::from_secs(5))
            else {
                panic!("the press never reached the consumer");
            };
            let Ok(ConsumerCommand::Transcribe(ring, _)) =
                command_receiver.recv_timeout(std::time::Duration::from_secs(5))
            else {
                panic!("the dictation never reached the consumer");
            };
            ring_wanted.recv().unwrap();
            drop(ring);
            let Ok(ConsumerCommand::Ask(ask)) =
                command_receiver.recv_timeout(std::time::Duration::from_secs(5))
            else {
                panic!("the ask never reached the listener");
            };
            state.disarm(ask.session);
            let _ = ask.reply.send(Ok("after the dictation".to_string()));
        }
    });
    assert!(state.record_start());
    let asking = spawn_ask(&state);
    state.record_stop();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert_eq!(
        state.recording_mode(),
        RecordingMode::Idle,
        "the question must not open the microphone while the dictation is still in the ring"
    );

    take_the_ring.send(()).unwrap();
    assert_eq!(answer_of(asking).await, "after the dictation");
    consumer.join().unwrap();
}

#[tokio::test]
async fn an_ask_during_dictation_waits_until_the_dictation_ends() {
    let (commands, command_receiver) = std::sync::mpsc::channel();
    let state = test_state(commands);
    let (answers, answer_next) = std::sync::mpsc::channel::<&'static str>();
    let listener = answer_in_turn(&state, command_receiver, answer_next);
    state.set_recording_mode(RecordingMode::PushToTalk);

    let asking = spawn_ask(&state);
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert!(!asking.is_finished(), "the ask must wait out the dictation");
    assert_eq!(state.recording_mode(), RecordingMode::PushToTalk);

    state.set_recording_mode(RecordingMode::Idle);
    answers.send("after dictation").unwrap();
    assert_eq!(answer_of(asking).await, "after dictation");
    drop(answers);
    listener.join().unwrap();
}

#[tokio::test]
async fn stop_replies_ok_and_signals_shutdown() {
    let state = test_state(std::sync::mpsc::channel().0);

    let request = JsonRpcRequest {
        jsonrpc: banshee_common::Version::V2,
        method: BANSHEE_STOP.to_string(),
        params: None,
        id: Some(serde_json::json!(1)),
    };
    let response = dispatch(request, &state, &Caller::unread()).await;

    let JsonRpcResponse::Success { result, .. } = response else {
        panic!("expected success response");
    };
    assert_eq!(result["ok"], true);
    // notify_one stores a permit, so a later notified() must resolve
    tokio::time::timeout(Duration::from_millis(100), state.shutdown().notified())
        .await
        .expect("shutdown was not signaled");
    assert_eq!(state.exit_code(), 0, "a stop must not be started again");
}

#[tokio::test]
async fn a_restart_replies_then_asks_to_exit_with_failure() {
    let state = test_state(std::sync::mpsc::channel().0);

    let response = dispatch(request(BANSHEE_RESTART, None), &state, &Caller::unread()).await;

    let JsonRpcResponse::Success { result, .. } = response else {
        panic!("expected success response, got {response:?}");
    };
    assert_eq!(result["ok"], true);
    tokio::time::timeout(Duration::from_millis(100), state.shutdown().notified())
        .await
        .expect("shutdown was not signaled");
    assert_eq!(
        state.exit_code(),
        75,
        "EX_TEMPFAIL, so the supervisor starts the daemon again"
    );
}

// "No microphone" and "busy" send the caller to different fixes, so the
// codes must not collapse into one another.
#[tokio::test]
async fn recording_rpcs_report_the_cause_not_a_busy_mic() {
    use crate::state::Mic;

    let causes = [
        (
            Mic::Faulted(RecordingError::Microphone("no device".to_string())),
            -32000,
            None,
        ),
        (
            Mic::Faulted(RecordingError::Model("missing file".to_string())),
            -32002,
            None,
        ),
        (
            Mic::Faulted(RecordingError::KeyFile("it does not parse".to_string())),
            -32008,
            None,
        ),
        (
            Mic::Faulted(RecordingError::Provider(
                "the remote listener refused the key".to_string(),
            )),
            rpc_code::PROVIDER,
            Some(("remote listener", "the message names the failing side")),
        ),
        (
            Mic::Starting,
            rpc_code::MICROPHONE,
            Some((
                "still starting",
                "the reason must say it is not broken, only late",
            )),
        ),
    ];
    let methods = [
        (BANSHEE_RECORD_START, None),
        (BANSHEE_RECORD_TOGGLE, None),
        (
            BANSHEE_ASK_USER,
            Some(serde_json::json!({"question": "ready?"})),
        ),
    ];

    for (mic, expected, says) in causes {
        for (method, params) in &methods {
            let state = test_state(std::sync::mpsc::channel().0);
            state.set_mic(mic.clone());

            let JsonRpcResponse::Error { error, .. } =
                dispatch(request(method, params.clone()), &state, &Caller::unread()).await
            else {
                panic!("{method} on {mic:?} must be refused");
            };
            assert_eq!(error.code, expected, "{method} on {mic:?}");
            if let Some((says, why)) = says {
                assert!(
                    error.message.contains(says),
                    "{method} on {mic:?}: {why}: {}",
                    error.message
                );
            }
            // The armed lock must not be taken by a refused question
            assert_eq!(state.recording_mode(), RecordingMode::Idle);
        }
    }
}

#[tokio::test]
async fn record_start_and_stop_drive_push_to_talk() {
    let (commands, command_receiver) = std::sync::mpsc::channel();
    let state = test_state(commands);

    let start = request(
        BANSHEE_RECORD_START,
        Some(serde_json::json!({"dictate": true})),
    );
    let JsonRpcResponse::Success { .. } = dispatch(start, &state, &Caller::unread()).await else {
        panic!("expected success response");
    };
    assert_eq!(state.recording_mode(), RecordingMode::PushToTalk);

    // A second start must be refused while recording
    let again = request(BANSHEE_RECORD_START, None);
    let JsonRpcResponse::Error { error, .. } = dispatch(again, &state, &Caller::unread()).await
    else {
        panic!("expected error response");
    };
    assert_eq!(error.code, rpc_code::BUSY);
    assert_eq!(state.recording_mode(), RecordingMode::PushToTalk);

    let stop = request(BANSHEE_RECORD_STOP, None);
    let JsonRpcResponse::Success { .. } = dispatch(stop, &state, &Caller::unread()).await else {
        panic!("expected success response");
    };
    assert_eq!(state.recording_mode(), RecordingMode::Idle);
    let Ok(ConsumerCommand::Stream(_)) = command_receiver.try_recv() else {
        panic!("expected the start to stream");
    };
    let Ok(ConsumerCommand::Transcribe(..)) = command_receiver.try_recv() else {
        panic!("expected a transcribe command");
    };
}

/// Starts a recording over the socket with `params`, lets its first sound
/// arrive, stops it, and gives back every signal the pair sent.
async fn recorded_with(params: Option<serde_json::Value>) -> Vec<crate::audio::cues::Signal> {
    let (commands, command_receiver) = std::sync::mpsc::channel();
    let state = test_state(commands);
    let mut signals = state.cues().subscribe_signals();

    let start = request(BANSHEE_RECORD_START, params);
    let JsonRpcResponse::Success { .. } = dispatch(start, &state, &Caller::unread()).await else {
        panic!("the start must be accepted");
    };
    state.heard_sound();
    state.announce_sound();
    let stop = request(BANSHEE_RECORD_STOP, None);
    let JsonRpcResponse::Success { .. } = dispatch(stop, &state, &Caller::unread()).await else {
        panic!("the stop must be accepted");
    };
    let Ok(ConsumerCommand::Stream(_)) = command_receiver.try_recv() else {
        panic!("the start must stream");
    };
    let Ok(ConsumerCommand::Transcribe(..)) = command_receiver.try_recv() else {
        panic!("the stop must send the audio to be transcribed");
    };
    let mut sent = Vec::new();
    while let Ok(signal) = signals.try_recv() {
        sent.push(signal);
    }
    sent
}

fn a_dictation() -> [crate::audio::cues::Signal; 2] {
    use crate::audio::cues::{Signal, Target};
    [
        Signal::RecordStart {
            target: Target::Dictate,
        },
        Signal::RecordStop {
            target: Target::Dictate,
        },
    ]
}

#[tokio::test]
async fn a_record_start_with_no_flag_dictates() {
    assert_eq!(recorded_with(None).await, a_dictation());
}

#[tokio::test]
async fn the_dictate_flag_still_dictates() {
    assert_eq!(
        recorded_with(Some(serde_json::json!({"dictate": true}))).await,
        a_dictation()
    );
}

#[tokio::test]
async fn record_toggle_starts_then_stops_and_says_which() {
    let (commands, command_receiver) = std::sync::mpsc::channel();
    let state = test_state(commands);

    let first = request(BANSHEE_RECORD_TOGGLE, None);
    let JsonRpcResponse::Success { result, .. } = dispatch(first, &state, &Caller::unread()).await
    else {
        panic!("expected success response");
    };
    assert_eq!(result["recording"], serde_json::Value::Bool(true));
    assert_eq!(state.recording_mode(), RecordingMode::PushToTalk);

    let second = request(BANSHEE_RECORD_TOGGLE, None);
    let JsonRpcResponse::Success { result, .. } = dispatch(second, &state, &Caller::unread()).await
    else {
        panic!("expected success response");
    };
    assert_eq!(result["recording"], serde_json::Value::Bool(false));
    assert_eq!(state.recording_mode(), RecordingMode::Idle);
    let Ok(ConsumerCommand::Stream(_)) = command_receiver.try_recv() else {
        panic!("expected the start to stream");
    };
    let Ok(ConsumerCommand::Transcribe(..)) = command_receiver.try_recv() else {
        panic!("expected a transcribe command");
    };
}

#[tokio::test]
async fn record_stop_while_idle_is_a_no_op() {
    let (commands, command_receiver) = std::sync::mpsc::channel();
    let state = test_state(commands);

    let stop = request(BANSHEE_RECORD_STOP, None);
    let JsonRpcResponse::Success { result, .. } = dispatch(stop, &state, &Caller::unread()).await
    else {
        panic!("expected success response");
    };
    assert_eq!(result["ok"], true);
    assert_eq!(state.recording_mode(), RecordingMode::Idle);
    assert!(command_receiver.try_recv().is_err(), "no command expected");
}

// One payload, so a subscriber's first read and a poller's read cannot
// disagree about what the daemon reports.
#[tokio::test]
async fn subscribe_answers_with_the_status_payload() {
    let state = test_state(std::sync::mpsc::channel().0);
    state.set_recording_mode(RecordingMode::PushToTalk);

    let JsonRpcResponse::Success { result: polled, .. } =
        dispatch(request(BANSHEE_STATUS, None), &state, &Caller::unread()).await
    else {
        panic!("expected success response");
    };
    let JsonRpcResponse::Success {
        result: subscribed, ..
    } = dispatch(request(BANSHEE_SUBSCRIBE, None), &state, &Caller::unread()).await
    else {
        panic!("expected success response");
    };

    assert_eq!(polled["recording"], true, "the fixture must discriminate");
    // Every key but the clock, which advances between the two calls
    assert!(polled["uptime_seconds"].is_number());
    for (key, value) in polled.as_object().expect("an object") {
        if key != "uptime_seconds" {
            assert_eq!(value, &subscribed[key], "'{key}' differs");
        }
    }
    assert_eq!(
        polled.as_object().map(serde_json::Map::len),
        subscribed.as_object().map(serde_json::Map::len),
        "one payload carries a key the other does not"
    );
}

// The two agree by construction, and nothing but this would notice if they stopped.
#[test]
fn a_pushed_change_agrees_with_what_status_reports() {
    let state = test_state(std::sync::mpsc::channel().0);
    state.set_recording_mode(RecordingMode::PushToTalk);
    // Both device fields carry a value, or `null == null` passes for them
    state.set_audio_device(Some("MacBook Pro Microphone".to_string()));
    state.set_missing_device(Some("Yeti Nano".to_string()));
    state.set_mic(crate::state::Mic::Open);

    let status = status_payload(&state);
    let live = live_state(&state);

    assert_eq!(live["recording"], true, "the fixture must discriminate");
    assert_eq!(live["microphone_open"], true);
    assert_eq!(live["audio_device"], "MacBook Pro Microphone");
    assert_eq!(live["missing_device"], "Yeti Nano");
    for key in live.as_object().expect("live_state is an object").keys() {
        assert_eq!(live[key], status[key], "'{key}' disagrees with status");
    }
}

// The tray takes its problem off the push, so the push ranks the list the
// status reply does.
#[test]
fn the_state_push_carries_the_blockers() {
    let state = test_state(std::sync::mpsc::channel().0);
    state.set_listening(crate::state::Listening::Missing(
        banshee_common::listening::find(banshee_common::listening::DEFAULT_MODEL).unwrap(),
    ));

    let status = status_payload(&state);
    let live = live_state(&state);

    assert!(
        status["blockers"][0]["id"].is_string(),
        "the fixture must discriminate"
    );
    assert_eq!(live["blockers"][0]["id"], status["blockers"][0]["id"]);
    assert_eq!(live["blockers"], status["blockers"]);
}

// Between uses the device stays named, so only this says whether it is open.
// No client could tell a use from `audio.keep_open` by the other flags.
#[test]
fn every_push_says_whether_the_microphone_is_open_and_a_move_wakes_it() {
    let state = test_state(std::sync::mpsc::channel().0);
    state.set_audio_device(Some("MacBook Pro Microphone".to_string()));
    let mut devices = state.device_changes();
    devices.mark_unchanged();

    state.set_mic(crate::state::Mic::Closed);
    assert_eq!(live_state(&state)["microphone_open"], false);
    assert!(
        !devices.has_changed().unwrap(),
        "closed to closed is no move"
    );

    state.set_mic(crate::state::Mic::Open);
    assert_eq!(live_state(&state)["microphone_open"], true);
    assert!(devices.has_changed().unwrap(), "the open wakes every push");
    devices.mark_unchanged();

    state.set_mic(crate::state::Mic::Closed);
    assert_eq!(live_state(&state)["microphone_open"], false);
    assert!(devices.has_changed().unwrap(), "and so does the close");
    assert_eq!(
        live_state(&state)["audio_device"],
        "MacBook Pro Microphone",
        "the device the next use opens stays named"
    );
}

// A refusal changes what the microphone does, so the push carries it. Each
// read goes through the setter, and status reads on every reply, so only a
// move may wake the push.
#[test]
fn a_refused_microphone_is_pushed_and_only_a_move_wakes_it() {
    let state = test_state(std::sync::mpsc::channel().0);
    let mut devices = state.device_changes();
    devices.mark_unchanged();
    assert_eq!(live_state(&state)["microphone_refused"], false);

    state.set_microphone_refusal(None);
    assert!(!devices.has_changed().unwrap(), "a repeat read is no move");

    state.set_microphone_refusal(Some(crate::state::Refusal::Denied));
    assert_eq!(live_state(&state)["microphone_refused"], true);
    assert!(
        devices.has_changed().unwrap(),
        "the refusal wakes every push"
    );
    devices.mark_unchanged();

    state.set_microphone_refusal(Some(crate::state::Refusal::Denied));
    assert!(!devices.has_changed().unwrap(), "a repeat read is no move");

    state.set_microphone_refusal(None);
    assert_eq!(live_state(&state)["microphone_refused"], false);
    assert!(devices.has_changed().unwrap(), "and so does the grant");
}

#[test]
fn status_carries_the_config_the_daemon_parsed() {
    let state = test_state(std::sync::mpsc::channel().0);
    let mut config = crate::config::Config::default();
    config.stt.model = "whisper-large-v3-turbo".to_string();
    config.stt.vocabulary = vec!["Tauri".to_string(), "Svelte".to_string()];
    config.tts.voice = "af_sky".to_string();
    state.set_config(std::sync::Arc::new(config));

    let status = status_payload(&state);
    assert_eq!(status["config"]["stt"]["model"], "whisper-large-v3-turbo");
    assert_eq!(
        status["config"]["stt"]["vocabulary"],
        serde_json::json!(["Tauri", "Svelte"])
    );
    assert_eq!(status["config"]["tts"]["voice"], "af_sky");
    assert!(status["config"]["audio"]["hotkey"].is_string());
}

/// The window builds its key capture from this list rather than a table of its
/// own, which would drift from the parser.
#[test]
fn status_reports_the_modifiers_the_parser_binds() {
    let state = test_state(std::sync::mpsc::channel().0);
    let reported = status_payload(&state)["bindable_modifiers"].clone();
    let expected = serde_json::to_value(crate::binding::bindable_modifiers()).unwrap();

    assert_eq!(reported, expected);
    for refused in ["Shift", "LeftShift", "RightShift", "CapsLock"] {
        assert!(
            !reported.as_array().unwrap().iter().any(|n| n == refused),
            "the parser refuses {refused}, so the window must never offer it"
        );
    }
}

#[test]
fn status_reports_nothing_pending_on_a_fresh_daemon() {
    let state = test_state(std::sync::mpsc::channel().0);
    assert_eq!(status_payload(&state)["pending"], serde_json::json!([]));
}

/// A model applied without `persist` reloads the listener and leaves the file
/// config where it was, so the flag follows the listener. The catalogue knows
/// it before any model loads.
#[test]
fn translates_follows_the_listener_in_use() {
    let state = test_state(std::sync::mpsc::channel().0);
    let status = status_payload(&state);
    assert_eq!(status["stt_translates"], false, "Parakeet, loaded or not");
    assert_eq!(
        status.get("english_only"),
        None,
        "the window reads languages"
    );

    state.listening_model_loaded(banshee_common::listening::find("Breeze-ASR-25"));
    assert_eq!(status_payload(&state)["stt_translates"], true);
}

#[test]
fn the_window_lists_the_suggestions_then_the_downloaded_models() {
    use banshee_common::listening::find;
    let dir = crate::test_support::unique_scratch("listening-models");
    let breeze = find("Breeze-ASR-25").unwrap();
    crate::test_support::write_empty(&dir, &[breeze.file]);
    let accepted = crate::models::licences::Accepted::at(dir.join("accepted-licences"));

    let listed = listening_models("parakeet-unified-en-0.6b", &dir, &accepted);
    let ids: Vec<&str> = listed
        .iter()
        .map(|one| one["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        ids,
        [
            "parakeet-unified-en-0.6b",
            "whisper-large-v3-turbo",
            "Breeze-ASR-25"
        ]
    );

    let parakeet = &listed[0];
    assert_eq!(parakeet["label"], "Parakeet Unified");
    assert_eq!(parakeet["megabytes"], 731);
    assert_eq!(parakeet["downloaded"], false);
    assert_eq!(parakeet["suggested"], true);
    assert_eq!(parakeet["languages"], serde_json::json!(["en"]));
    assert_eq!(parakeet["streams"], true);
    assert!(
        parakeet["credit"]
            .as_str()
            .unwrap()
            .contains("Licensed by NVIDIA Corporation under the NVIDIA Open Model License"),
        "{parakeet}"
    );
    assert_eq!(
        parakeet["licence"].get("url"),
        None,
        "the window opens a licence by id"
    );
    assert_eq!(parakeet["accept_command"], serde_json::Value::Null);

    assert_eq!(listed[1]["label"], "Whisper turbo");
    assert_eq!(listed[1]["streams"], false);
    assert_eq!(listed[1]["credit"], serde_json::Value::Null);

    let breeze = &listed[2];
    assert_eq!(breeze["label"], "Breeze-ASR-25");
    assert_eq!(breeze["downloaded"], true);
    assert_eq!(breeze["suggested"], false);
    assert_eq!(breeze["languages"], serde_json::json!(["zh", "en"]));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Set through the CLI, so the window has no other way to learn why its
/// download is refused.
#[test]
fn a_configured_model_waiting_on_its_licence_is_listed_with_the_command_that_accepts_it() {
    let dir = crate::test_support::unique_scratch("listening-models-licence");
    let mut accepted = crate::models::licences::Accepted::at(dir.join("accepted-licences"));

    let listed = listening_models("canary-1b", &dir, &accepted);
    let canary = listed.last().unwrap();
    assert_eq!(canary["id"], "canary-1b");
    assert_eq!(canary["downloaded"], false);
    assert_eq!(canary["licence"]["name"], "CC-BY-NC-4.0");
    assert!(
        canary["licence"]["summary"]
            .as_str()
            .unwrap()
            .contains("non-commercial"),
        "{canary}"
    );
    assert_eq!(
        canary["accept_command"],
        "banshee setup --model canary-1b --accept-licence"
    );

    accepted.add("CC-BY-NC-4.0").unwrap();
    let listed = listening_models("canary-1b", &dir, &accepted);
    assert_eq!(
        listed.last().unwrap()["accept_command"],
        serde_json::Value::Null
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// whisper-base sorts after canary-1b in the catalogue, so catalogue order
/// alone would put the configured model before it.
#[test]
fn the_configured_model_comes_after_the_downloaded_ones() {
    let dir = crate::test_support::unique_scratch("listening-models-order");
    let base = banshee_common::listening::find("whisper-base").unwrap();
    crate::test_support::write_empty(&dir, &[base.file]);
    let accepted = crate::models::licences::Accepted::at(dir.join("accepted-licences"));

    let listed = listening_models("canary-1b", &dir, &accepted);
    let ids: Vec<&str> = listed
        .iter()
        .map(|one| one["id"].as_str().unwrap())
        .collect();
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        ids,
        [
            "parakeet-unified-en-0.6b",
            "whisper-large-v3-turbo",
            "whisper-base",
            "canary-1b"
        ]
    );
}

/// The licence gate sits in the download path, so a file already here asks
/// for nothing.
#[test]
fn a_downloaded_model_asks_for_no_acceptance() {
    let dir = crate::test_support::unique_scratch("listening-models-here");
    let canary = banshee_common::listening::find("canary-1b").unwrap();
    crate::test_support::write_empty(&dir, &[canary.file]);
    let accepted = crate::models::licences::Accepted::at(dir.join("accepted-licences"));

    let listed = listening_models("canary-1b", &dir, &accepted);
    let _ = std::fs::remove_dir_all(&dir);
    let entry = listed.iter().find(|one| one["id"] == "canary-1b").unwrap();
    assert_eq!(entry["downloaded"], true);
    assert_eq!(entry["accept_command"], serde_json::Value::Null);
}

#[test]
fn status_carries_the_listening_models() {
    let state = test_state(std::sync::mpsc::channel().0);
    let status = status_payload(&state);
    assert_eq!(status["stt_models"][0]["id"], "parakeet-unified-en-0.6b");
    assert_eq!(status["stt_models"][1]["id"], "whisper-large-v3-turbo");
}

#[test]
fn status_reads_the_licence_record_the_machine_names() {
    let mut config = crate::config::Config::default();
    config.stt.model = "canary-1b".to_string();
    let state = crate::test_support::daemon_state_speaking(
        config,
        crate::text_to_speech::Speaker::Fallback,
        std::sync::mpsc::channel().0,
    );
    let canary = |status: serde_json::Value| {
        status["stt_models"]
            .as_array()
            .unwrap()
            .iter()
            .find(|one| one["id"] == "canary-1b")
            .unwrap()["accept_command"]
            .clone()
    };
    assert_eq!(
        canary(status_payload(&state)),
        "banshee setup --model canary-1b --accept-licence"
    );

    crate::models::licences::Accepted::at(state.licences().to_path_buf())
        .add("CC-BY-NC-4.0")
        .unwrap();

    assert_eq!(canary(status_payload(&state)), serde_json::Value::Null);
}

#[test]
fn stt_model_is_the_id_of_the_model_in_use() {
    let state = test_state(std::sync::mpsc::channel().0);
    assert_eq!(
        status_payload(&state)["stt_model"],
        "parakeet-unified-en-0.6b"
    );
}

#[test]
fn a_remote_listener_reports_no_model_and_translates() {
    let mut config = crate::config::Config::default();
    config.stt.provider = crate::config::Provider::Remote;
    let state = crate::test_support::daemon_state_speaking(
        config,
        crate::text_to_speech::Speaker::Fallback,
        std::sync::mpsc::channel().0,
    );
    let status = status_payload(&state);
    assert_eq!(status["stt_model"], serde_json::Value::Null);
    assert_eq!(status["stt_translates"], true);
}

#[test]
fn status_says_nothing_leaves_the_machine_while_every_provider_is_local() {
    let remote = remote_report(
        &Route::Local,
        &crate::text_to_speech::SpeakerSpec::of(&crate::config::TTSConfig::default()),
        |_| false,
        true,
    );
    for side in ["stt", "tts"] {
        assert_eq!(remote[side]["remote"], false);
        assert_eq!(remote[side]["host"], serde_json::Value::Null);
        assert!(remote[side]["key_present"].is_boolean());
    }
}

// A copy of the listener's key read into the speaker's object would answer the
// same bool for both sides, so each side is read through a closure that answers
// for one side only. No test touches the real credentials file.
#[test]
fn each_side_reports_its_own_key() {
    for held in [
        crate::credentials::RemoteKey::Stt,
        crate::credentials::RemoteKey::Tts,
    ] {
        let report = remote_report(
            &Route::Local,
            &crate::text_to_speech::SpeakerSpec::of(&crate::config::TTSConfig::default()),
            |side| side == held,
            true,
        );
        assert_eq!(
            report["stt"]["key_present"],
            held == crate::credentials::RemoteKey::Stt,
            "a key for {held:?} only: {report}"
        );
        assert_eq!(
            report["tts"]["key_present"],
            held == crate::credentials::RemoteKey::Tts,
            "a key for {held:?} only: {report}"
        );
    }
}

/// A config whose speaker is the server, with `voice` named in its table.
fn remote_speaker(voice: &str) -> crate::config::Config {
    let mut config = crate::config::Config::default();
    config.tts.provider = crate::config::Provider::Remote;
    config.tts.remote.base_url = "https://api.openai.com/v1".to_string();
    config.tts.remote.voice = voice.to_string();
    config
}

// The speaker the daemon built says this, and a voice name says nothing about
// it: a missing key leaves the config naming a voice and the local voice
// speaking every reply, and a live `[tts]` write leaves the voice held either
// way.
#[test]
fn the_reply_says_which_speaker_started_and_not_which_voice_is_held() {
    for speaker in [
        crate::text_to_speech::Speaker::Fallback,
        crate::text_to_speech::Speaker::Configured("marin".to_string()),
    ] {
        let started = speaker.started();
        let state = crate::test_support::daemon_state_speaking(
            remote_speaker("marin"),
            speaker,
            std::sync::mpsc::channel().0,
        );
        state.set_tts_voice(Some("marin".to_string()));
        let remote = &status_payload(&state)["remote"];
        assert_eq!(remote["tts"]["speaker_started"], started, "{remote}");
        assert_eq!(remote["tts"]["remote"], true);
        assert_eq!(remote["tts"]["host"], "api.openai.com");
        assert_eq!(remote["stt"]["remote"], false);
    }
}

// The window marks the voice the backend loaded, so a remote speaker's voice
// has to be the one reported. Kokoro's catalogue is still the list.
#[tokio::test]
async fn list_voices_names_no_current_voice_until_one_is_set_and_then_the_one_held() {
    let state = test_state(std::sync::mpsc::channel().0);
    let JsonRpcResponse::Success { result, .. } = dispatch(
        request(BANSHEE_LIST_VOICES, None),
        &state,
        &Caller::unread(),
    )
    .await
    else {
        panic!("expected success response");
    };
    assert!(result["current"].is_null());
    assert!(result["voices"].is_array(), "{result}");

    state.set_tts_voice(Some("marin".to_string()));

    let JsonRpcResponse::Success { result, .. } = dispatch(
        request(BANSHEE_LIST_VOICES, None),
        &state,
        &Caller::unread(),
    )
    .await
    else {
        panic!("expected success response");
    };
    assert_eq!(result["current"], "marin");
    assert!(
        result["voices"]
            .as_array()
            .expect("a voices array")
            .iter()
            .any(|voice| voice["id"] == "af_sky"),
        "the local catalogue stays the list: {result}"
    );
}

async fn listed_voices(state: &Arc<DaemonState>) -> Vec<serde_json::Value> {
    let JsonRpcResponse::Success { result, .. } =
        dispatch(request(BANSHEE_LIST_VOICES, None), state, &Caller::unread()).await
    else {
        panic!("expected success response");
    };
    result["voices"].as_array().expect("a voices array").clone()
}

#[tokio::test]
async fn the_companion_is_downloaded_only_when_all_seven_files_are_there() {
    let state = test_state(std::sync::mpsc::channel().0);
    let files = crate::models::download::pocket_files();
    let (last, rest) = files.split_last().unwrap();
    crate::test_support::write_empty(
        state.models_dir(),
        &rest
            .iter()
            .map(|file| file.name.as_str())
            .collect::<Vec<_>>(),
    );

    let companion = listed_voices(&state).await.remove(0);
    assert_eq!(companion["id"], banshee_common::COMPANION_VOICE);
    assert_eq!(companion["downloaded"], false, "{} is missing", last.name);

    crate::test_support::write_empty(state.models_dir(), &[&last.name]);
    let companion = listed_voices(&state).await.remove(0);
    assert_eq!(companion["downloaded"], true);
}

#[test]
fn status_names_the_host_a_remote_listener_sends_audio_to() {
    let mut config = crate::config::Config::default();
    config.stt.provider = crate::config::Provider::Remote;
    config.stt.remote.base_url = "https://api.groq.com/openai/v1".to_string();
    let state = crate::test_support::daemon_state_speaking(
        config,
        crate::text_to_speech::Speaker::Fallback,
        std::sync::mpsc::channel().0,
    );
    let remote = &status_payload(&state)["remote"];
    assert_eq!(remote["stt"]["remote"], true);
    assert_eq!(remote["stt"]["host"], "api.groq.com");
}

#[test]
fn the_report_names_the_listener_in_force() {
    let state = test_state(std::sync::mpsc::channel().0);
    state.install_listener(
        Route::Remote {
            base_url: "https://api.example.test/v1".into(),
            model: "whisper-1".into(),
        },
        crate::state::Listening::Remote,
    );
    let status = status_payload(&state);
    assert_eq!(status["remote"]["stt"]["remote"], true);
    assert_eq!(status["remote"]["stt"]["host"], "api.example.test");
}

#[test]
fn live_state_reports_armed_while_a_question_waits() {
    let state = test_state(std::sync::mpsc::channel().0);

    for (name, mode, armed, recording) in [
        ("idle at the start", RecordingMode::Idle, false, false),
        ("armed", RecordingMode::Armed, true, true),
        ("push to talk", RecordingMode::PushToTalk, false, true),
        ("idle again", RecordingMode::Idle, false, false),
    ] {
        state.set_recording_mode(mode);
        let live = live_state(&state);
        assert_eq!(live["armed"], serde_json::json!(armed), "{name}");
        assert_eq!(live["recording"], serde_json::json!(recording), "{name}");
    }
}

#[test]
fn live_state_reports_transcribing() {
    let state = test_state(std::sync::mpsc::channel().0);
    assert_eq!(live_state(&state)["transcribing"], serde_json::json!(false));
    state.set_transcribing(true);
    assert_eq!(live_state(&state)["transcribing"], serde_json::json!(true));
    state.set_transcribing(false);
    assert_eq!(live_state(&state)["transcribing"], serde_json::json!(false));
}

#[test]
fn setting_transcribing_wakes_a_subscriber() {
    let state = test_state(std::sync::mpsc::channel().0);
    let mut changes = state.subscribe_transcribing();
    state.set_transcribing(true);
    assert!(changes.has_changed().expect("the sender outlives this"));
    assert!(*changes.borrow_and_update());
}

#[tokio::test]
async fn a_second_download_is_refused_while_one_runs() {
    let state = test_state(std::sync::mpsc::channel().0);
    // Something to fetch, or the call answers "nothing to do" and never
    // reaches the slot at all
    state.set_wanted_downloads(vec![crate::models::download::Download {
        megabytes: 1,
        name: "no-such-model-9f3a.bin".to_string(),
        url: "https://example.invalid/no-such-model-9f3a.bin".to_string(),
        sha256: None,
        model: None,
    }]);
    let slot = state.start_downloading().expect("the slot starts free");

    let JsonRpcResponse::Error { error, .. } = dispatch(
        request(BANSHEE_DOWNLOAD_MODELS, None),
        &state,
        &Caller::unread(),
    )
    .await
    else {
        panic!("expected the busy error");
    };
    assert_eq!(error.code, rpc_code::DOWNLOAD_RUNNING);
    assert!(
        !state.config_path().exists(),
        "a refused download chooses no feedback mode"
    );

    drop(slot);
    assert!(state.start_downloading().is_some(), "the slot came back");
}

// With nothing else wanted, only the named model can reach the busy slot.
#[tokio::test]
async fn a_download_by_model_fetches_that_model_and_refuses_an_unknown_id() {
    let state = test_state(std::sync::mpsc::channel().0);
    state.set_wanted_downloads(Vec::new());
    let slot = state.start_downloading().expect("the slot starts free");

    let JsonRpcResponse::Error { error, .. } = dispatch(
        request(
            BANSHEE_DOWNLOAD_MODELS,
            Some(serde_json::json!({"model": "moonshine-tiny"})),
        ),
        &state,
        &Caller::unread(),
    )
    .await
    else {
        panic!("expected the busy error");
    };
    assert_eq!(error.code, rpc_code::DOWNLOAD_RUNNING);
    drop(slot);

    let JsonRpcResponse::Error { error, .. } = dispatch(
        request(
            BANSHEE_DOWNLOAD_MODELS,
            Some(serde_json::json!({"model": "no-such-model-9f3a"})),
        ),
        &state,
        &Caller::unread(),
    )
    .await
    else {
        panic!("expected an unknown id to be refused");
    };
    assert_eq!(error.code, rpc_code::INVALID_PARAMS);
    assert!(
        error.message.contains("no-such-model-9f3a"),
        "{}",
        error.message
    );
}

#[cfg(target_os = "macos")]
fn slow_voiceover() -> bool {
    std::thread::sleep(std::time::Duration::from_millis(200));
    true
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn a_first_download_writes_the_mode_into_the_daemons_own_config_after_the_reply() {
    let state = Arc::new(DaemonState::new(
        Arc::new(crate::config::Config::default()),
        crate::test_support::seeded_history(&[]),
        crate::test_support::quiet_speech(),
        crate::text_to_speech::Speaker::Fallback,
        std::sync::mpsc::channel().0,
        crate::audio::cues::Cues::silent(),
        crate::state::Machine {
            voiceover: slow_voiceover,
            ..crate::test_support::scratch_machine("api-first-run")
        },
    ));
    // A port nothing listens on, so the fetch fails at once and off the network
    state.set_wanted_downloads(vec![crate::models::download::Download {
        megabytes: 1,
        name: "no-such-model-4c1d.bin".to_string(),
        url: "http://127.0.0.1:9/no-such-model-4c1d.bin".to_string(),
        sha256: None,
        model: None,
    }]);

    let JsonRpcResponse::Success { .. } = dispatch(
        request(BANSHEE_DOWNLOAD_MODELS, None),
        &state,
        &Caller::unread(),
    )
    .await
    else {
        panic!("expected the download to start");
    };
    assert!(
        !state.config_path().exists(),
        "the reply does not wait on the VoiceOver read"
    );

    let written = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if let Ok(written) = std::fs::read_to_string(state.config_path()) {
                return written;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the mode lands in the config the daemon state names");
    assert_eq!(
        crate::config::Config::parse(&written)
            .unwrap()
            .feedback
            .mode,
        Some(crate::config::FeedbackMode::Both),
        "{written}"
    );
}

#[tokio::test]
async fn downloading_nothing_reports_an_empty_list() {
    let state = test_state(std::sync::mpsc::channel().0);
    state.set_wanted_downloads(Vec::new());

    let JsonRpcResponse::Success { result, .. } = dispatch(
        request(BANSHEE_DOWNLOAD_MODELS, None),
        &state,
        &Caller::unread(),
    )
    .await
    else {
        panic!("expected success response");
    };
    assert_eq!(result["ok"], true);
    assert_eq!(result["downloading"].as_array().unwrap().len(), 0);
    assert!(
        state.start_downloading().is_some(),
        "a call that fetched nothing must not hold the slot"
    );
}

#[tokio::test]
async fn an_unknown_agent_slug_is_refused_by_plan_and_apply() {
    let state = test_state(std::sync::mpsc::channel().0);

    let JsonRpcResponse::Error {
        error: plan_error, ..
    } = dispatch(
        request(
            BANSHEE_CONNECT_PLAN,
            Some(serde_json::json!({"agent": "notatool", "disconnect": false})),
        ),
        &state,
        &Caller::unread(),
    )
    .await
    else {
        panic!("expected an error response");
    };
    assert!(
        plan_error.message.contains("notatool"),
        "{}",
        plan_error.message
    );

    let JsonRpcResponse::Error {
        error: apply_error, ..
    } = dispatch(
        request(
            BANSHEE_CONNECT_APPLY,
            Some(serde_json::json!({"agent": "notatool", "disconnect": false})),
        ),
        &state,
        &Caller::unread(),
    )
    .await
    else {
        panic!("expected an error response");
    };
    assert!(
        apply_error.message.contains("notatool"),
        "{}",
        apply_error.message
    );
}

#[test]
fn connect_apply_carries_the_codex_note_and_no_note_for_claude_code() {
    assert_eq!(
        apply_reply(connect::Agent::Codex, 2),
        serde_json::json!({
            "applied": 2,
            "note": "Codex runs this hook only after you trust it: open Codex and run /hooks.",
        })
    );
    assert_eq!(
        apply_reply(connect::Agent::ClaudeCode, 1),
        serde_json::json!({"applied": 1})
    );
}

#[tokio::test]
async fn disconnect_true_is_refused_before_the_agent_is_resolved() {
    let state = test_state(std::sync::mpsc::channel().0);
    let refusal =
        "Disconnect is not available yet. Remove Banshee from the agent's config by hand.";

    for request in [
        request(
            BANSHEE_CONNECT_PLAN,
            Some(serde_json::json!({"agent": "cursor", "disconnect": true})),
        ),
        request(
            BANSHEE_CONNECT_APPLY,
            Some(serde_json::json!({"agent": "cursor", "disconnect": true})),
        ),
        request(
            BANSHEE_CONNECT_PLAN,
            Some(serde_json::json!({"agent": "notatool", "disconnect": true})),
        ),
        request(
            BANSHEE_CONNECT_APPLY,
            Some(serde_json::json!({"agent": "notatool", "disconnect": true})),
        ),
    ] {
        let response = dispatch(request, &state, &Caller::unread()).await;
        let JsonRpcResponse::Error { error, .. } = response else {
            panic!("expected an error response");
        };
        assert_eq!(error.message, refusal);
    }
}

struct VoiceCapture(Arc<std::sync::Mutex<Vec<Option<String>>>>);

struct RecordedUtterance;

impl crate::text_to_speech::ActiveUtterance for RecordedUtterance {
    fn is_finished(&mut self) -> bool {
        true
    }
    fn stop(&mut self) {}
}

impl crate::text_to_speech::TtsBackend for VoiceCapture {
    fn start(
        &self,
        _text: &str,
        voice: Option<&str>,
    ) -> Result<Box<dyn crate::text_to_speech::ActiveUtterance>, banshee_common::error::BansheeError>
    {
        self.0.lock().unwrap().push(voice.map(str::to_string));
        Ok(Box::new(RecordedUtterance))
    }
}

#[tokio::test]
async fn speak_passes_the_voice_parameter_to_the_backend() {
    let captured = Arc::new(std::sync::Mutex::new(Vec::new()));
    let speech =
        crate::text_to_speech::SpeechPlayer::new(Box::new(VoiceCapture(Arc::clone(&captured))));
    let state = Arc::new(DaemonState::new(
        Arc::new(crate::config::Config::default()),
        crate::test_support::seeded_history(&[]),
        speech,
        crate::text_to_speech::Speaker::Fallback,
        std::sync::mpsc::channel().0,
        crate::audio::cues::Cues::silent(),
        crate::test_support::scratch_machine("api-models"),
    ));

    let response = dispatch(
        request(
            BANSHEE_SPEAK,
            Some(serde_json::json!({"text": "hello", "voice": "am_adam"})),
        ),
        &state,
        &Caller::unread(),
    )
    .await;

    assert!(matches!(response, JsonRpcResponse::Success { .. }));
    assert_eq!(*captured.lock().unwrap(), vec![Some("am_adam".to_string())]);
}

/// Silence answers `{"text": ""}`; a listener that failed answers an error, so
/// the agent never mistakes one for the other.
#[tokio::test]
async fn ask_user_answers_an_error_when_the_listen_failed() {
    let (commands, command_receiver) = std::sync::mpsc::channel();
    let state = test_state(commands);

    let session_state = Arc::clone(&state);
    std::thread::spawn(move || {
        let Ok(ConsumerCommand::Ask(ask)) = command_receiver.recv() else {
            return;
        };
        session_state.set_recording_mode(RecordingMode::Idle);
        let _ = ask
            .reply
            .send(Err("the remote listener refused the key".to_string()));
    });

    let request = request(
        BANSHEE_ASK_USER,
        Some(serde_json::json!({"question": "Ready to ship?"})),
    );
    let response = dispatch(request, &state, &Caller::unread()).await;

    let JsonRpcResponse::Error { error, .. } = response else {
        panic!("expected error response");
    };
    assert_eq!(error.code, rpc_code::LISTENING_FAILED);
    assert!(
        error.message.contains("refused the key"),
        "{}",
        error.message
    );
}

#[test]
fn the_last_error_rides_in_status_and_in_the_pushed_state() {
    let state = test_state(std::sync::mpsc::channel().0);
    assert_eq!(
        status_payload(&state)["last_error"],
        serde_json::Value::Null
    );
    assert_eq!(live_state(&state)["last_error"], serde_json::Value::Null);

    state.set_last_error(Some("the remote listener refused the key".to_string()));
    assert_eq!(
        status_payload(&state)["last_error"],
        "the remote listener refused the key"
    );
    assert_eq!(
        live_state(&state)["last_error"],
        "the remote listener refused the key"
    );

    state.set_last_error(None);
    assert_eq!(live_state(&state)["last_error"], serde_json::Value::Null);
}

// A reader that only polls status must see the reason too, not only a
// subscriber.
#[test]
fn a_failed_reply_is_reported_in_status_and_in_the_pushed_state() {
    let state = test_state(std::sync::mpsc::channel().0);
    assert_eq!(
        status_payload(&state)["last_speech_error"],
        serde_json::Value::Null
    );
    assert_eq!(
        crate::api::live_state(&state)["last_speech_error"],
        serde_json::Value::Null
    );

    state.set_last_speech_error(Some("the remote speaker refused the key".to_string()));
    assert_eq!(
        status_payload(&state)["last_speech_error"],
        "the remote speaker refused the key"
    );
    assert_eq!(
        crate::api::live_state(&state)["last_speech_error"],
        "the remote speaker refused the key"
    );
    // The listener's field is untouched, so a client can tell the two apart
    assert_eq!(
        status_payload(&state)["last_error"],
        serde_json::Value::Null
    );
}

/// Every key a value holds, as a path list, so a comparison states which key
/// is missing rather than that two blobs differ.
fn key_paths(value: &serde_json::Value, at: &str, into: &mut Vec<String>) {
    if let serde_json::Value::Object(fields) = value {
        for (name, field) in fields {
            let path = format!("{at}.{name}");
            into.push(path.clone());
            key_paths(field, &path, into);
        }
    }
}

fn paths_of(value: &serde_json::Value) -> Vec<String> {
    let mut paths = Vec::new();
    key_paths(value, "config", &mut paths);
    paths.sort();
    paths
}

// A window mock stands in for a real reply in every test that reads it, so a
// mock missing a key the daemon writes lets a branch that reads that key pass
// on a shape no daemon sends. Every mock that carries a `config` is listed, so
// a new one that skips the table is the thing to notice.
#[test]
fn the_window_mocks_carry_every_config_key_the_reply_writes() {
    for (name, body) in [
        (
            "remote.json",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../banshee-app/ui/src/mocks/remote.json"
            )),
        ),
        (
            "remote-speech.json",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../banshee-app/ui/src/mocks/remote-speech.json"
            )),
        ),
        (
            "ready.json",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../banshee-app/ui/src/mocks/ready.json"
            )),
        ),
        (
            "permissions.json",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../banshee-app/ui/src/mocks/permissions.json"
            )),
        ),
        (
            "pending-cues.json",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../banshee-app/ui/src/mocks/pending-cues.json"
            )),
        ),
    ] {
        one_mock_carries_every_config_key(name, body);
    }
}

/// The `banshee.state_changed` mocks, listed once so a fifth cannot be added to
/// one of the two tests below and not the other.
const LIVE_MOCKS: [(&str, &str); 4] = [
    (
        "recording.json",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../banshee-app/ui/src/mocks/recording.json"
        )),
    ),
    (
        "armed.json",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../banshee-app/ui/src/mocks/armed.json"
        )),
    ),
    (
        "transcribing.json",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../banshee-app/ui/src/mocks/transcribing.json"
        )),
    ),
    (
        "speaking.json",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../banshee-app/ui/src/mocks/speaking.json"
        )),
    ),
];

// The window feeds these to `reduceLive` as a `banshee.state_changed` push, so
// a mock short of a key lets a branch pass on a payload no daemon sends.
#[test]
fn the_window_mocks_carry_every_live_key_the_push_writes() {
    let state = test_state(std::sync::mpsc::channel().0);
    let written: Vec<String> = live_state(&state)
        .as_object()
        .expect("an object")
        .keys()
        .cloned()
        .collect();

    for (name, body) in LIVE_MOCKS {
        let mock: serde_json::Value = serde_json::from_str(body).expect("the mock parses");
        let carried = mock.as_object().expect("an object");
        let missing: Vec<&String> = written
            .iter()
            .filter(|key| !carried.contains_key(*key))
            .collect();
        let invented: Vec<&String> = carried
            .keys()
            .filter(|key| !written.contains(key))
            .collect();
        assert!(
            missing.is_empty() && invented.is_empty(),
            "{name} is missing {missing:?} and carries {invented:?}, which no push does"
        );
    }
}

// Each live mock names the activity the daemon would rank for its own flags,
// so the window is never handed a word the ranking would not have chosen.
#[test]
fn each_live_mock_names_the_activity_its_flags_rank() {
    for (name, body) in LIVE_MOCKS {
        let mock: serde_json::Value = serde_json::from_str(body).expect("the mock parses");
        assert_eq!(
            mock["activity"],
            serde_json::json!(banshee_common::Activity::of(&mock).word()),
            "{name} names an activity its own flags do not rank"
        );
    }
}

fn one_mock_carries_every_config_key(name: &str, body: &str) {
    let mock: serde_json::Value = serde_json::from_str(body).expect("the mock parses");
    let answered = paths_of(&serde_json::to_value(crate::config::Config::default()).unwrap());
    let carried = paths_of(&mock["config"]);

    let missing: Vec<&String> = answered
        .iter()
        .filter(|key| !carried.contains(key))
        .collect();
    let invented: Vec<&String> = carried
        .iter()
        .filter(|key| !answered.contains(key))
        .collect();
    assert!(
        missing.is_empty() && invented.is_empty(),
        "{name} is missing {missing:?} and carries {invented:?}, which no reply does"
    );
}

// The daemon answers its socket from `claim()`, long before the microphone is
// open. Saying nothing of that gap leaves a client reporting a healthy daemon
// that cannot record.
#[tokio::test]
async fn status_says_the_pipeline_is_opening_until_it_stands() {
    let state = crate::test_support::daemon_state_before_the_pipeline();

    let opening = dispatch(request(BANSHEE_STATUS, None), &state, &Caller::unread()).await;
    let JsonRpcResponse::Success { result, .. } = opening else {
        panic!("status answers");
    };
    assert_eq!(result["pipeline"], "opening");

    state.set_mic(crate::state::Mic::Open);
    let open = dispatch(request(BANSHEE_STATUS, None), &state, &Caller::unread()).await;
    let JsonRpcResponse::Success { result, .. } = open else {
        panic!("status answers");
    };
    assert_eq!(result["pipeline"], "open");
}

// The window draws its word from this. A daemon that answers "ready" while it
// cannot record sends a person to a hotkey that plays the error cue.
#[test]
fn anything_that_stops_banshee_working_is_not_ready() {
    use crate::state::Pipeline;

    let blocker = banshee_common::Blocker {
        kind: banshee_common::BlockerKind::Permission,
        role: None,
        remedy: None,
        id: "accessibility".to_string(),
        name: "Accessibility is not granted".to_string(),
        consequence: "dictation cannot type".to_string(),
        fix: "grant it".to_string(),
        command: None,
    };
    let broken = Pipeline::Broken(RecordingError::Microphone("no device".to_string()));

    for (name, blockers, pipeline, ready) in [
        (
            "an open pipeline with nothing blocking",
            vec![],
            Pipeline::Open,
            true,
        ),
        ("a pipeline still opening", vec![], Pipeline::Opening, false),
        ("a broken pipeline", vec![], broken, false),
        ("a blocker", vec![blocker], Pipeline::Open, false),
    ] {
        assert_eq!(super::ready(&blockers, &pipeline), ready, "{name}");
    }
}

#[tokio::test]
async fn an_ask_dropped_while_its_question_waits_never_speaks_it() {
    let crate::test_support::HoldingSpeech { state, spoken, .. } =
        crate::test_support::daemon_state_holding_speech(
            std::time::Duration::from_millis(300),
            std::sync::mpsc::channel().0,
        );
    state
        .speech()
        .speak("Agent C reports.", false, None)
        .unwrap();
    let asking = spawn_ask(&state);
    wait_for(&state, "the session arms", |state| {
        state.recording_mode() == RecordingMode::Armed
    })
    .await;

    asking.abort();
    wait_for(&state, "the session closes", |state| {
        state.recording_mode() == RecordingMode::Idle
    })
    .await;
    wait_for(&state, "the speech ends", |state| {
        !state.speech().is_speaking()
    })
    .await;

    assert_eq!(
        *spoken.lock().unwrap(),
        ["Agent C reports."],
        "a question with nobody to answer it must not play"
    );
}

// An agent that dies mid-question leaves the microphone armed for the rest of
// the session, and every other ask waits behind it until it ends. Dropping
// the call has to close the session the way a stop does.
#[tokio::test]
async fn a_dropped_ask_closes_the_session_it_armed() {
    let (commands, command_receiver) = std::sync::mpsc::channel();
    let state = test_state(commands);

    let asking = spawn_ask(&state);

    // The session is armed once the consumer has the command
    let ask = tokio::task::spawn_blocking(move || command_receiver.recv())
        .await
        .expect("the blocking read")
        .expect("the consumer is asked");
    assert_eq!(state.recording_mode(), RecordingMode::Armed);

    asking.abort();

    // Generous next to the 30 ms the consumer polls at: a bound for the test,
    // not a limit the daemon promises.
    for _ in 0..100 {
        if state.recording_mode() == RecordingMode::Idle {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(
        state.recording_mode(),
        RecordingMode::Idle,
        "the microphone belongs to nobody once the caller is gone"
    );
    drop(ask);
}

fn spawn_ask(state: &Arc<DaemonState>) -> tokio::task::JoinHandle<JsonRpcResponse> {
    spawn_question(state, "Ready to ship?")
}

fn spawn_question(
    state: &Arc<DaemonState>,
    question: &'static str,
) -> tokio::task::JoinHandle<JsonRpcResponse> {
    spawn_question_from(state, question, Caller::unread())
}

fn spawn_question_from(
    state: &Arc<DaemonState>,
    question: &'static str,
    caller: Caller,
) -> tokio::task::JoinHandle<JsonRpcResponse> {
    let state = Arc::clone(state);
    tokio::spawn(async move {
        let asked = request(
            BANSHEE_ASK_USER,
            Some(serde_json::json!({ "question": question })),
        );
        dispatch(asked, &state, &caller).await
    })
}

async fn first_armed(
    state: &Arc<DaemonState>,
    question: &'static str,
) -> tokio::task::JoinHandle<JsonRpcResponse> {
    let first = spawn_question(state, question);
    wait_for(state, "the first ask arms", |state| {
        state.recording_mode() == RecordingMode::Armed
    })
    .await;
    first
}

/// Breaks the microphone, and expects the waiting ask to answer with that
/// error at once rather than when the microphone would have come free.
async fn break_the_microphone_for(
    state: &DaemonState,
    waiting: tokio::task::JoinHandle<JsonRpcResponse>,
) {
    state.set_mic(crate::state::Mic::Faulted(RecordingError::Microphone(
        "no device".to_string(),
    )));
    let response = tokio::time::timeout(std::time::Duration::from_secs(1), waiting)
        .await
        .expect("the waiting ask must answer as soon as the microphone breaks")
        .unwrap();
    let JsonRpcResponse::Error { error, .. } = response else {
        panic!("expected the microphone error, got {response:?}");
    };
    assert_eq!(error.code, rpc_code::MICROPHONE);
}

/// Answers each ask with the next line sent on `answers`, and ends the session
/// the way the consumer does.
fn answer_in_turn(
    state: &Arc<DaemonState>,
    commands: std::sync::mpsc::Receiver<ConsumerCommand>,
    answers: std::sync::mpsc::Receiver<&'static str>,
) -> std::thread::JoinHandle<()> {
    let state = Arc::clone(state);
    std::thread::spawn(move || {
        for answer in answers {
            let Ok(ConsumerCommand::Ask(ask)) =
                commands.recv_timeout(std::time::Duration::from_secs(5))
            else {
                panic!("the ask never reached the listener");
            };
            state.disarm(ask.session);
            let _ = ask.reply.send(Ok(answer.to_string()));
        }
    })
}

async fn answer_of(asking: tokio::task::JoinHandle<JsonRpcResponse>) -> String {
    let response = tokio::time::timeout(std::time::Duration::from_secs(5), asking)
        .await
        .expect("the ask never answered")
        .unwrap();
    let JsonRpcResponse::Success { result, .. } = response else {
        panic!("expected an answer, got {response:?}");
    };
    result["text"].as_str().unwrap_or_default().to_string()
}

/// Polls the state until it holds, or the test fails. Generous next to the
/// intervals the daemon polls at: a bound for the test, not a promise.
async fn wait_for(
    state: &crate::state::DaemonState,
    what: &str,
    holds: impl Fn(&crate::state::DaemonState) -> bool,
) {
    for _ in 0..200 {
        if holds(state) {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("{what} did not happen within 2s");
}

#[test]
fn a_dropped_ask_leaves_a_recording_someone_else_started() {
    let state = test_state(std::sync::mpsc::channel().0);
    let session = EndsTheSession {
        state: &state,
        session: state.arm_for_ask().expect("the ask arms"),
    };
    state.set_recording_mode(RecordingMode::PushToTalk);

    drop(session);

    assert_eq!(state.recording_mode(), RecordingMode::PushToTalk);
}

fn processes(pid: u32) -> Option<Process> {
    let (path, parent) = match pid {
        40 => (
            "/Applications/Banshee.app/Contents/MacOS/banshee-mcp-shim",
            41,
        ),
        41 => ("/usr/local/bin/claude", 2),
        50 => ("/usr/local/bin/codex", 2),
        60 => ("/usr/local/bin/banshee", 2),
        _ => return None,
    };
    let path = std::path::PathBuf::from(path);
    Some(Process {
        name: path.file_name()?.to_string_lossy().into_owned(),
        path: Some(path),
        parent,
    })
}

/// The shim that Claude, process 41, runs.
const SHIM: Caller = Caller::read_by(40, processes);
const CODEX: Caller = Caller::read_by(50, processes);

/// Asks once from `caller`, and answers the way the consumer does, keeping the
/// row with the asker the command carries. Gives back the question as it was
/// published while the answer was listened for, and the rows.
async fn ask_and_answer(caller: &Caller) -> (crate::state::Question, Vec<crate::history::Entry>) {
    let (commands, command_receiver) = std::sync::mpsc::channel();
    let state = test_state(commands);
    let published = state.subscribe_question();
    let consumer = std::thread::spawn({
        let state = Arc::clone(&state);
        move || {
            let Ok(ConsumerCommand::Ask(ask)) =
                command_receiver.recv_timeout(std::time::Duration::from_secs(5))
            else {
                panic!("the ask never reached the listener");
            };
            let shown = published.borrow().clone();
            state.keep(
                &NewEntry::question(&ask.question, Some("yes"), Outcome::Transcribed, None)
                    .with_client(ask.client.as_deref()),
            );
            state.disarm(ask.session);
            let _ = ask.reply.send(Ok("yes".to_string()));
            shown
        }
    });
    dispatch(
        request(
            BANSHEE_ASK_USER,
            Some(serde_json::json!({"question": "Ship it?"})),
        ),
        &state,
        caller,
    )
    .await;
    let shown = consumer.join().unwrap();
    (shown, state.history(None, None).unwrap())
}

#[tokio::test]
async fn a_question_publishes_and_keeps_the_name_of_who_asked() {
    let (published, rows) = ask_and_answer(&SHIM).await;
    assert_eq!(published.text, "Ship it?");
    assert_eq!(published.client.as_deref(), Some("claude"));
    assert_eq!(rows.last().unwrap().client.as_deref(), Some("claude"));
}

const BANSHEE: Caller = Caller::read_by(60, processes);

#[tokio::test]
async fn a_socket_dictation_logs_as_a_recording() {
    let state = test_state(std::sync::mpsc::channel().0);
    dispatch(
        request(BANSHEE_RECORD_START, Some(serde_json::json!({}))),
        &state,
        &BANSHEE,
    )
    .await;
    assert_eq!(state.recording_mode(), RecordingMode::PushToTalk);
}

#[tokio::test]
async fn a_hold_to_answer_over_the_socket_is_not_logged_as_a_recording() {
    let (commands, command_receiver) = std::sync::mpsc::channel();
    let state = test_state(commands);
    let (answers, answer_next) = std::sync::mpsc::channel::<&'static str>();
    let listener = answer_in_turn(&state, command_receiver, answer_next);
    let asking = spawn_question_from(&state, "Ready to ship?", SHIM);
    wait_for(&state, "the ask arms", |state| {
        state.recording_mode() == RecordingMode::Armed
    })
    .await;

    dispatch(
        request(BANSHEE_RECORD_START, Some(serde_json::json!({}))),
        &state,
        &BANSHEE,
    )
    .await;
    assert_eq!(state.recording_mode(), RecordingMode::ArmedHold);

    state.record_stop();
    answers.send("yes").unwrap();
    answer_of(asking).await;
    drop(answers);
    listener.join().unwrap();
}

#[tokio::test]
async fn configuring_the_keep_open_setting_over_the_socket_succeeds_for_a_named_caller() {
    let state = test_state(std::sync::mpsc::channel().0);
    let JsonRpcResponse::Success { result, .. } = dispatch(
        request(
            BANSHEE_CONFIGURE,
            Some(serde_json::json!({"settings": {"audio.keep_open": true}})),
        ),
        &state,
        &BANSHEE,
    )
    .await
    else {
        panic!("expected success response");
    };
    assert!(
        result["applied"]
            .as_array()
            .unwrap()
            .contains(&"audio.keep_open".into())
    );
}

#[tokio::test]
async fn a_question_that_is_never_put_keeps_the_name_too() {
    let state = test_state(std::sync::mpsc::channel().0);
    state.set_mic(crate::state::Mic::Starting);
    dispatch(
        request(
            BANSHEE_ASK_USER,
            Some(serde_json::json!({"question": "Deploy now?"})),
        ),
        &state,
        &SHIM,
    )
    .await;
    let rows = state.history(None, None).unwrap();
    assert_eq!(rows.last().unwrap().outcome, Outcome::Unasked);
    assert_eq!(rows.last().unwrap().client.as_deref(), Some("claude"));
}

#[tokio::test]
async fn a_question_from_an_unread_caller_carries_no_name() {
    let (published, rows) = ask_and_answer(&Caller::unread()).await;
    assert_eq!(published.text, "Ship it?");
    assert_eq!(published.client, None);
    assert_eq!(rows.last().unwrap().client, None);
}

#[tokio::test]
async fn the_second_of_two_waiting_questions_shows_its_own_asker() {
    let (commands, command_receiver) = std::sync::mpsc::channel();
    let state = test_state(commands);
    let (answers, answer_next) = std::sync::mpsc::channel::<&'static str>();
    let listener = answer_in_turn(&state, command_receiver, answer_next);
    let published = state.subscribe_question();

    let first = spawn_question_from(&state, "First question", SHIM);
    wait_for(&state, "the first ask arms", |state| {
        state.recording_mode() == RecordingMode::Armed
    })
    .await;
    let second = spawn_question_from(&state, "Second question", CODEX);
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert_eq!(published.borrow().client.as_deref(), Some("claude"));

    answers.send("one").unwrap();
    answer_of(first).await;
    wait_for(&state, "the second question is published", |_| {
        published.borrow().text == "Second question"
    })
    .await;
    assert_eq!(published.borrow().client.as_deref(), Some("codex"));

    answers.send("two").unwrap();
    answer_of(second).await;
    drop(answers);
    listener.join().unwrap();
}

#[tokio::test]
async fn speech_counts_for_the_shims_agent_and_ignores_a_sent_agent_pid() {
    let state = test_state(std::sync::mpsc::channel().0);
    dispatch(
        request(
            BANSHEE_SPEAK,
            Some(serde_json::json!({"text": "hi", "agent_pid": 7})),
        ),
        &state,
        &SHIM,
    )
    .await;
    for (agent, verdict) in [(41, "pass"), (7, "speak")] {
        let JsonRpcResponse::Success { result, .. } = dispatch(
            request(
                BANSHEE_TURN_ENDED,
                Some(serde_json::json!({ "agent_pid": agent })),
            ),
            &state,
            &Caller::unread(),
        )
        .await
        else {
            panic!("expected success response");
        };
        assert_eq!(result["verdict"], verdict, "agent {agent}");
    }
}

#[tokio::test]
async fn a_speech_from_an_agent_counts_for_that_agent_only() {
    let state = test_state(std::sync::mpsc::channel().0);
    dispatch(
        request(BANSHEE_SPEAK, Some(serde_json::json!({"text": "hi"}))),
        &state,
        &SHIM,
    )
    .await;
    assert_eq!(
        state.spoken().turn_ended(41),
        banshee_common::TurnVerdict::Pass
    );
    assert_eq!(
        state.spoken().turn_ended(42),
        banshee_common::TurnVerdict::Speak
    );
}

#[tokio::test]
async fn a_question_that_cannot_be_asked_is_kept_as_unasked() {
    let state = test_state(std::sync::mpsc::channel().0);
    state.set_mic(crate::state::Mic::Starting);
    let response = dispatch(
        request(
            BANSHEE_ASK_USER,
            Some(serde_json::json!({"question": "Deploy now?"})),
        ),
        &state,
        &Caller::unread(),
    )
    .await;
    let JsonRpcResponse::Error { error, .. } = response else {
        panic!("expected the microphone refusal");
    };
    let rows = state.history(None, None).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].kind, crate::history::Kind::Question);
    assert_eq!(rows[0].question.as_deref(), Some("Deploy now?"));
    assert_eq!(rows[0].text, None);
    assert_eq!(rows[0].outcome, crate::history::Outcome::Unasked);
    assert_eq!(rows[0].reason.as_deref(), Some(error.message.as_str()));
}

#[tokio::test]
async fn an_unasked_question_keeps_the_words_the_agent_sent() {
    let state = test_state(std::sync::mpsc::channel().0);
    state.set_mic(crate::state::Mic::Starting);
    dispatch(
        request(
            BANSHEE_ASK_USER,
            Some(serde_json::json!({"question": "Run `rm -rf /` now?"})),
        ),
        &state,
        &Caller::unread(),
    )
    .await;
    let rows = state.history(None, None).unwrap();
    assert_eq!(rows[0].question.as_deref(), Some("Run `rm -rf /` now?"));
}

#[tokio::test]
async fn a_question_is_spoken_clean_and_reaches_the_listener_as_sent() {
    let (commands, command_receiver) = std::sync::mpsc::channel();
    let crate::test_support::HoldingSpeech { state, spoken, .. } =
        crate::test_support::daemon_state_holding_speech(
            std::time::Duration::from_millis(10),
            commands,
        );
    let (heard, question) = std::sync::mpsc::channel();

    let session_state = Arc::clone(&state);
    std::thread::spawn(move || {
        let Ok(ConsumerCommand::Ask(ask)) = command_receiver.recv() else {
            return;
        };
        let _ = heard.send(ask.question.clone());
        session_state.set_recording_mode(RecordingMode::Idle);
        let _ = ask.reply.send(Ok("no".to_string()));
    });

    dispatch(
        request(
            BANSHEE_ASK_USER,
            Some(serde_json::json!({"question": "Run `rm -rf /` now?"})),
        ),
        &state,
        &Caller::unread(),
    )
    .await;
    assert_eq!(question.recv().unwrap(), "Run `rm -rf /` now?");
    assert_eq!(*spoken.lock().unwrap(), ["Run now?"]);
}

#[tokio::test]
async fn history_keeps_the_raw_question_the_card_shows_clean() {
    let (commands, command_receiver) = std::sync::mpsc::channel();
    let crate::test_support::HoldingSpeech { state, .. } =
        crate::test_support::daemon_state_holding_speech(
            std::time::Duration::from_millis(10),
            commands,
        );
    let published = state.subscribe_question();
    let (shown, on_the_card) = std::sync::mpsc::channel();

    let session_state = Arc::clone(&state);
    std::thread::spawn(move || {
        let Ok(ConsumerCommand::Ask(ask)) = command_receiver.recv() else {
            return;
        };
        let _ = shown.send(published.borrow().text.clone());
        session_state.keep(&NewEntry::question(
            &ask.question,
            Some("no"),
            Outcome::Transcribed,
            None,
        ));
        session_state.set_recording_mode(RecordingMode::Idle);
        let _ = ask.reply.send(Ok("no".to_string()));
    });

    dispatch(
        request(
            BANSHEE_ASK_USER,
            Some(serde_json::json!({"question": "Run \u{202E}now?"})),
        ),
        &state,
        &Caller::unread(),
    )
    .await;

    assert_eq!(on_the_card.recv().unwrap(), "Run \u{202E}now?");
    let rows = state.history(None, None).unwrap();
    assert_eq!(rows[0].question.as_deref(), Some("Run \u{202E}now?"));
}

#[tokio::test]
async fn every_exit_after_the_arm_ends_the_published_question() {
    // A refused line publishes and ends within one poll, so only the end is seen
    let (commands, _consumer) = std::sync::mpsc::channel();
    let held = crate::test_support::daemon_state_holding_speech(
        std::time::Duration::from_millis(10),
        commands,
    );
    let refused = held.state.subscribe_question();
    spawn_question(&held.state, crate::test_support::REFUSED)
        .await
        .unwrap();
    assert_eq!(*refused.borrow(), crate::state::Question::default());
    assert!(refused.has_changed().unwrap());

    let (commands, _consumer) = std::sync::mpsc::channel();
    let held = crate::test_support::daemon_state_holding_speech(
        std::time::Duration::from_millis(10),
        commands,
    );
    let mut stalled = held.state.subscribe_question();
    let asking = spawn_question_from(&held.state, crate::test_support::STALLS, SHIM);
    stalled.changed().await.unwrap();
    assert_eq!(
        stalled.borrow_and_update().text,
        crate::test_support::STALLS
    );
    asking.await.unwrap();
    assert_eq!(*stalled.borrow(), crate::state::Question::default());
    assert!(stalled.has_changed().unwrap());

    let (commands, command_receiver) = std::sync::mpsc::channel();
    let state = test_state(commands);
    let mut dropped = state.subscribe_question();
    let asking = spawn_question_from(&state, "Ready to ship?", SHIM);
    let _ask = tokio::task::spawn_blocking(move || command_receiver.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(dropped.borrow_and_update().text, "Ready to ship?");
    asking.abort();
    wait_for(&state, "the dropped ask ends the question", |state| {
        *state.subscribe_question().borrow() == crate::state::Question::default()
    })
    .await;
    assert!(dropped.has_changed().unwrap());
}

#[tokio::test]
async fn the_question_is_published_before_it_is_spoken() {
    let (commands, _consumer) = std::sync::mpsc::channel();
    let crate::test_support::HoldingSpeech { state, spoken, .. } =
        crate::test_support::daemon_state_holding_speech(
            std::time::Duration::from_millis(10),
            commands,
        );
    state
        .speech()
        .speak(crate::test_support::STALLS, false, None)
        .unwrap();
    let mut question = state.subscribe_question();
    let asking = spawn_ask(&state);

    tokio::time::timeout(std::time::Duration::from_secs(2), question.changed())
        .await
        .expect("the question was never published")
        .unwrap();
    assert_eq!(question.borrow().text, "Ready to ship?");
    assert_eq!(
        *spoken.lock().unwrap(),
        [crate::test_support::STALLS],
        "the card is up while the question waits behind a line"
    );
    asking.abort();
    state.speech().stop();
}

#[tokio::test]
async fn a_question_refused_before_the_pipeline_stands_still_counts() {
    let state = test_state(std::sync::mpsc::channel().0);
    state.set_mic(crate::state::Mic::Starting);
    let response = dispatch(
        request(
            BANSHEE_ASK_USER,
            Some(serde_json::json!({"question": "ready?"})),
        ),
        &state,
        &SHIM,
    )
    .await;
    let JsonRpcResponse::Error { error, .. } = response else {
        panic!("expected the microphone refusal");
    };
    assert_eq!(error.code, rpc_code::MICROPHONE);
    assert_eq!(
        state.spoken().turn_ended(41),
        banshee_common::TurnVerdict::Pass
    );
}

#[tokio::test]
async fn a_request_missing_its_text_counts_for_nobody() {
    let state = test_state(std::sync::mpsc::channel().0);
    for method in [BANSHEE_SPEAK, BANSHEE_ASK_USER] {
        dispatch(request(method, Some(serde_json::json!({}))), &state, &SHIM).await;
    }
    assert_eq!(
        state.spoken().turn_ended(41),
        banshee_common::TurnVerdict::Speak
    );
}

#[tokio::test]
async fn an_agent_pid_that_is_not_a_process_id_is_refused() {
    let state = test_state(std::sync::mpsc::channel().0);
    let response = dispatch(
        request(
            BANSHEE_TURN_ENDED,
            Some(serde_json::json!({"agent_pid": -1})),
        ),
        &state,
        &Caller::unread(),
    )
    .await;
    let JsonRpcResponse::Error { error, .. } = response else {
        panic!("expected an error response");
    };
    assert_eq!(error.code, rpc_code::INVALID_PARAMS);
}

#[tokio::test]
async fn turn_ended_with_no_repeat_answers_the_records_verdict() {
    let state = test_state(std::sync::mpsc::channel().0);
    let JsonRpcResponse::Success { result, .. } = dispatch(
        request(
            BANSHEE_TURN_ENDED,
            Some(serde_json::json!({"agent_pid": 7})),
        ),
        &state,
        &Caller::unread(),
    )
    .await
    else {
        panic!("expected success response");
    };
    assert_eq!(result["verdict"], "speak");
}

#[tokio::test]
async fn turn_ended_needs_the_agent() {
    let state = test_state(std::sync::mpsc::channel().0);
    let response = dispatch(
        request(BANSHEE_TURN_ENDED, Some(serde_json::json!({}))),
        &state,
        &Caller::unread(),
    )
    .await;
    let JsonRpcResponse::Error { error, .. } = response else {
        panic!("expected an error response");
    };
    assert_eq!(error.code, rpc_code::INVALID_PARAMS);
}

#[tokio::test]
async fn turn_ended_with_repeat_true_answers_pass() {
    let state = test_state(std::sync::mpsc::channel().0);
    let JsonRpcResponse::Success { result, .. } = dispatch(
        request(
            BANSHEE_TURN_ENDED,
            Some(serde_json::json!({"agent_pid": 7, "repeat": true})),
        ),
        &state,
        &Caller::unread(),
    )
    .await
    else {
        panic!("expected success response");
    };
    assert_eq!(result["verdict"], "pass", "even on a silent turn");
}

#[test]
fn the_report_names_the_speaker_in_force() {
    let state = test_state(std::sync::mpsc::channel().0);
    state.speech().set_builder(crate::test_support::here(
        &crate::test_support::SpokenLines::default(),
        "alloy",
    ));
    let tts = crate::config::TTSConfig {
        provider: crate::config::Provider::Remote,
        remote: crate::config::RemoteTtsConfig {
            base_url: "https://api.example.test/v1".to_string(),
            voice: "alloy".to_string(),
            ..Default::default()
        },
        ..Default::default()
    };
    assert!(state.switch_speaker(&tts, false));
    let status = status_payload(&state);
    assert_eq!(status["remote"]["tts"]["remote"], true);
    assert_eq!(status["remote"]["tts"]["host"], "api.example.test");
}

#[tokio::test]
async fn a_configure_reply_names_the_keys_still_waiting() {
    let state = test_state(std::sync::mpsc::channel().0);
    let reply = dispatch(
        request(
            BANSHEE_CONFIGURE,
            Some(serde_json::json!({
                "settings": {"stt.model": "whisper-large-v3-turbo"},
                "persist": true
            })),
        ),
        &state,
        &Caller::unread(),
    )
    .await;
    let JsonRpcResponse::Success { result, .. } = reply else {
        panic!("a legal write succeeds")
    };
    assert_eq!(result["waiting"], serde_json::json!(["stt.model"]));
    assert!(result.get("restart_required").is_none());
}

/// Keeps `entry` and answers its id.
fn kept_row(state: &DaemonState, entry: &NewEntry) -> i64 {
    state.keep(entry);
    state.history(None, None).unwrap().last().unwrap().id
}

const A_TENTH_OF_A_SECOND: [f32; 1600] = [0.25; 1600];

async fn refusal(
    state: &Arc<DaemonState>,
    method: &str,
    params: serde_json::Value,
) -> (i32, String) {
    let JsonRpcResponse::Error { error, .. } =
        dispatch(request(method, Some(params)), state, &Caller::unread()).await
    else {
        panic!("{method} must be refused");
    };
    (error.code, error.message)
}

/// Answers the first retry the consumer gets with `answer`, and hands back
/// the command's row id and samples.
fn retry_consumer(
    commands: std::sync::mpsc::Receiver<ConsumerCommand>,
    answer: Result<&str, &str>,
) -> std::thread::JoinHandle<(i64, Vec<f32>)> {
    let answer = answer.map(str::to_string).map_err(str::to_string);
    std::thread::spawn(move || {
        let Ok(ConsumerCommand::Retry(retry)) = commands.recv() else {
            panic!("the consumer gets a retry");
        };
        let _ = retry.reply.send(answer);
        (retry.id, retry.samples)
    })
}

#[tokio::test]
async fn history_audio_answers_the_path_of_a_kept_row() {
    let state = test_state(std::sync::mpsc::channel().0);
    let id = kept_row(
        &state,
        &NewEntry::dictation("hello").with_audio(&A_TENTH_OF_A_SECOND),
    );

    let response = dispatch(
        request(BANSHEE_HISTORY_AUDIO, Some(serde_json::json!({"id": id}))),
        &state,
        &Caller::unread(),
    )
    .await;

    let JsonRpcResponse::Success { result, .. } = response else {
        panic!("a kept row answers its audio");
    };
    let path = std::path::PathBuf::from(result["path"].as_str().unwrap());
    assert_eq!(Some(path.clone()), state.audio_path(id));
    assert!(path.is_file());
}

#[tokio::test]
async fn history_audio_refuses_a_row_without_audio() {
    let state = test_state(std::sync::mpsc::channel().0);
    let id = kept_row(&state, &NewEntry::dictation("hello"));

    let refused = refusal(&state, BANSHEE_HISTORY_AUDIO, serde_json::json!({"id": id})).await;

    assert_eq!(
        refused,
        (
            rpc_code::INVALID_PARAMS,
            "No audio is kept for this row.".to_string()
        )
    );
}

#[tokio::test]
async fn history_audio_refuses_a_missing_or_non_integer_id() {
    let state = test_state(std::sync::mpsc::channel().0);
    for method in [BANSHEE_HISTORY_AUDIO, BANSHEE_RETRY] {
        for params in [
            serde_json::json!({}),
            serde_json::json!({"id": "1"}),
            serde_json::json!({"id": 1.5}),
        ] {
            let (code, message) = refusal(&state, method, params.clone()).await;
            assert_eq!(code, rpc_code::INVALID_PARAMS, "{method} {params}");
            assert!(message.contains("'id'"), "{method} {params}: {message}");
        }
    }
}

#[tokio::test]
async fn an_unknown_row_is_refused_by_name() {
    let state = test_state(std::sync::mpsc::channel().0);
    for method in [BANSHEE_HISTORY_AUDIO, BANSHEE_RETRY] {
        let refused = refusal(&state, method, serde_json::json!({"id": 41})).await;
        assert_eq!(
            refused,
            (rpc_code::INVALID_PARAMS, "No row has id 41.".to_string()),
            "{method}"
        );
    }
}

#[tokio::test]
async fn retry_refuses_a_question_row() {
    let state = test_state(std::sync::mpsc::channel().0);
    let id = kept_row(
        &state,
        &NewEntry::question("Ship it?", Some("Yes"), Outcome::Transcribed, None)
            .with_audio(&A_TENTH_OF_A_SECOND),
    );

    let refused = refusal(&state, BANSHEE_RETRY, serde_json::json!({"id": id})).await;

    assert_eq!(
        refused,
        (
            rpc_code::INVALID_PARAMS,
            "Only a dictation can be retried.".to_string()
        )
    );
}

#[tokio::test]
async fn retry_refuses_a_row_without_audio() {
    let state = test_state(std::sync::mpsc::channel().0);
    let id = kept_row(&state, &NewEntry::dictation("hello"));

    let refused = refusal(&state, BANSHEE_RETRY, serde_json::json!({"id": id})).await;

    assert_eq!(
        refused,
        (
            rpc_code::INVALID_PARAMS,
            "No audio is kept for this row.".to_string()
        )
    );
}

#[tokio::test]
async fn a_retry_whose_model_will_not_load_answers_the_consumers_reason() {
    let (commands, consumer) = std::sync::mpsc::channel();
    let state = test_state(commands);
    let id = kept_row(
        &state,
        &NewEntry::dictation("hello").with_audio(&A_TENTH_OF_A_SECOND),
    );
    state.set_mic(crate::state::Mic::Faulted(RecordingError::Model(
        "missing file".to_string(),
    )));
    let consumer = retry_consumer(consumer, Err("The listening model would not load."));

    let refused = refusal(&state, BANSHEE_RETRY, serde_json::json!({"id": id})).await;

    assert_eq!(
        refused,
        (
            rpc_code::LISTENING_FAILED,
            "The listening model would not load.".to_string()
        )
    );
    assert_eq!(consumer.join().unwrap().0, id);
}

#[tokio::test]
async fn a_retry_with_no_consumer_answers_at_the_send() {
    let state = test_state(std::sync::mpsc::channel().0);
    let id = kept_row(
        &state,
        &NewEntry::dictation("hello").with_audio(&A_TENTH_OF_A_SECOND),
    );

    let refused = refusal(&state, BANSHEE_RETRY, serde_json::json!({"id": id})).await;

    assert_eq!(
        refused,
        (
            rpc_code::INTERNAL,
            "Audio pipeline is not running.".to_string()
        )
    );
}

#[tokio::test]
async fn retry_replaces_the_text_and_answers_the_row() {
    let (commands, consumer) = std::sync::mpsc::channel();
    let state = test_state(commands);
    let id = kept_row(
        &state,
        &NewEntry::failed_dictation("The server did not answer.").with_audio(&A_TENTH_OF_A_SECOND),
    );
    let consumer = retry_consumer(consumer, Ok("hello"));

    let response = dispatch(
        request(BANSHEE_RETRY, Some(serde_json::json!({"id": id}))),
        &state,
        &Caller::unread(),
    )
    .await;

    let JsonRpcResponse::Success { result, .. } = response else {
        panic!("a kept dictation retries: {response:?}");
    };
    assert_eq!(result["row"]["id"], id);
    assert_eq!(result["row"]["outcome"], "transcribed");
    assert_eq!(result["row"]["text"], "hello");
    assert!(result["row"]["reason"].is_null());
    let row = state.history_entry(id).unwrap().unwrap();
    assert_eq!(row.text.as_deref(), Some("hello"));
    let (asked_for, samples) = consumer.join().unwrap();
    assert_eq!(asked_for, id);
    assert_eq!(samples.len(), A_TENTH_OF_A_SECOND.len());
    assert!(
        samples
            .iter()
            .all(|sample| (sample - 0.25).abs() <= 1.0 / f32::from(i16::MAX)),
        "the consumer hears the kept audio"
    );
}

#[tokio::test]
async fn a_failed_retry_leaves_the_row_unchanged() {
    let (commands, consumer) = std::sync::mpsc::channel();
    let state = test_state(commands);
    let id = kept_row(
        &state,
        &NewEntry::failed_dictation("The server did not answer.").with_audio(&A_TENTH_OF_A_SECOND),
    );
    let before = serde_json::to_value(state.history_entry(id).unwrap()).unwrap();
    let _consumer = retry_consumer(consumer, Err("The model returned no text."));

    let refused = refusal(&state, BANSHEE_RETRY, serde_json::json!({"id": id})).await;

    assert_eq!(
        refused,
        (
            rpc_code::LISTENING_FAILED,
            "The model returned no text.".to_string()
        )
    );
    let after = serde_json::to_value(state.history_entry(id).unwrap()).unwrap();
    assert_eq!(before, after);
}

#[tokio::test]
async fn a_retry_needs_no_microphone() {
    for fault in [
        RecordingError::Microphone("no device".to_string()),
        RecordingError::MicrophoneRefused(crate::state::Refusal::Denied),
    ] {
        let (commands, consumer) = std::sync::mpsc::channel();
        let state = test_state(commands);
        let id = kept_row(
            &state,
            &NewEntry::failed_dictation("The server did not answer.")
                .with_audio(&A_TENTH_OF_A_SECOND),
        );
        state.set_mic(crate::state::Mic::Faulted(fault.clone()));
        let consumer = retry_consumer(consumer, Ok("hello"));

        let response = dispatch(
            request(BANSHEE_RETRY, Some(serde_json::json!({"id": id}))),
            &state,
            &Caller::unread(),
        )
        .await;

        let JsonRpcResponse::Success { result, .. } = response else {
            panic!("{fault:?} must not refuse a retry: {response:?}");
        };
        assert_eq!(result["row"]["text"], "hello", "{fault:?}");
        assert_eq!(consumer.join().unwrap().0, id);
    }
}

#[tokio::test]
async fn a_retry_whose_row_was_cleared_meanwhile_names_the_row() {
    let (commands, consumer) = std::sync::mpsc::channel();
    let state = test_state(commands);
    let id = kept_row(
        &state,
        &NewEntry::failed_dictation("The server did not answer.").with_audio(&A_TENTH_OF_A_SECOND),
    );
    let clearing = Arc::clone(&state);
    std::thread::spawn(move || {
        let Ok(ConsumerCommand::Retry(retry)) = consumer.recv() else {
            panic!("the consumer gets a retry");
        };
        clearing.clear_history().unwrap();
        let _ = retry.reply.send(Ok("hello".to_string()));
    });

    let refused = refusal(&state, BANSHEE_RETRY, serde_json::json!({"id": id})).await;

    assert_eq!(
        refused,
        (rpc_code::INVALID_PARAMS, format!("No row has id {id}."))
    );
}

#[test]
fn the_live_state_carries_settings_changes() {
    let state = test_state(std::sync::mpsc::channel().0);
    assert_eq!(live_state(&state)["settings_changes"], 0);

    state.bump_settings_changes();

    assert_eq!(
        live_state(&state)["settings_changes"],
        state.settings_changes()
    );
    assert_eq!(state.settings_changes(), 1);
}
