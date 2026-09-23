use super::*;
use banshee_common::TurnVerdict::{Pass, Speak};

const AGENT: u32 = 4242;

#[test]
fn a_turn_that_spoke_passes() {
    let turns = SpokenTurns::default();
    turns.spoke(AGENT);
    assert_eq!(turns.turn_ended(AGENT), Pass);
}

#[test]
fn a_silent_turn_is_sent_back_once() {
    let turns = SpokenTurns::default();
    assert_eq!(turns.turn_ended(AGENT), Speak);
    assert_eq!(
        turns.turn_ended(AGENT),
        Pass,
        "the reminder never repeats in one turn"
    );
    assert_eq!(
        turns.turn_ended(AGENT),
        Speak,
        "the next silent turn is sent back again"
    );
}

#[test]
fn speaking_after_a_reminder_ends_the_turn() {
    let turns = SpokenTurns::default();
    assert_eq!(turns.turn_ended(AGENT), Speak);
    turns.spoke(AGENT);
    assert_eq!(turns.turn_ended(AGENT), Pass);
    assert_eq!(
        turns.turn_ended(AGENT),
        Speak,
        "the speech belonged to the turn before"
    );
}

#[test]
fn one_agent_speaking_does_not_pass_another() {
    let turns = SpokenTurns::default();
    turns.spoke(AGENT);
    assert_eq!(turns.turn_ended(AGENT + 1), Speak);
    assert_eq!(turns.turn_ended(AGENT), Pass);
}

#[test]
fn the_turn_after_a_reminded_turn_is_reminded_when_silent() {
    let turns = SpokenTurns::default();
    assert_eq!(turns.turn_ended(AGENT), Speak);
    turns.spoke(AGENT);
    assert_eq!(turns.repeated_stop(AGENT), Pass);
    assert_eq!(
        turns.turn_ended(AGENT),
        Speak,
        "the speech belonged to the reminded turn"
    );
}

#[test]
fn a_repeated_stop_closes_a_turn_that_ignored_the_reminder() {
    let turns = SpokenTurns::default();
    assert_eq!(turns.turn_ended(AGENT), Speak);
    assert_eq!(turns.repeated_stop(AGENT), Pass);
    assert_eq!(
        turns.turn_ended(AGENT),
        Speak,
        "the next silent turn is sent back"
    );
}
