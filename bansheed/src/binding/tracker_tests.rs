use super::*;
use rdev::Button;

fn press(key: Key) -> EventType {
    EventType::KeyPress(key)
}

fn release(key: Key) -> EventType {
    EventType::KeyRelease(key)
}

fn tracker(text: &str, mode: HotkeyMode) -> HotkeyTracker {
    HotkeyTracker::new(hotkey(text).unwrap(), mode)
}

#[test]
fn hold_mode_records_between_press_and_release() {
    let mut t = tracker("F5", HotkeyMode::Hold);
    assert_eq!(t.on_event(&press(Key::F5)), Some(HotkeyAction::Start));
    assert_eq!(
        t.on_event(&press(Key::F5)),
        None,
        "auto-repeat must not restart"
    );
    assert_eq!(t.on_event(&release(Key::F5)), Some(HotkeyAction::Stop));
}

#[test]
fn toggle_mode_emits_toggle_and_lets_the_daemon_resolve_it() {
    let mut t = tracker("F5", HotkeyMode::Toggle);
    assert_eq!(t.on_event(&press(Key::F5)), Some(HotkeyAction::Toggle));
    assert_eq!(t.on_event(&release(Key::F5)), None);
    assert_eq!(t.on_event(&press(Key::F5)), Some(HotkeyAction::Toggle));
}

#[test]
fn a_chord_fires_only_with_its_modifiers_down() {
    let mut t = tracker("Ctrl+Alt+D", HotkeyMode::Hold);
    assert_eq!(
        t.on_event(&press(Key::KeyD)),
        None,
        "a bare D is typing, not the hotkey"
    );
    assert_eq!(
        t.on_event(&release(Key::KeyD)),
        None,
        "typing must not stop a session another caller opened"
    );
    t.on_event(&press(Key::ControlLeft));
    assert_eq!(
        t.on_event(&press(Key::KeyD)),
        None,
        "half the chord is still typing"
    );
    t.on_event(&release(Key::KeyD));
    t.on_event(&press(Key::Alt));
    assert_eq!(t.on_event(&press(Key::KeyD)), Some(HotkeyAction::Start));
    assert_eq!(
        t.on_event(&release(Key::KeyD)),
        Some(HotkeyAction::Stop),
        "the main key alone ends it, whatever order the chord unwinds"
    );
}

#[test]
fn a_bare_key_press_does_not_toggle() {
    let mut t = tracker("Ctrl+Alt+D", HotkeyMode::Toggle);
    t.on_event(&press(Key::ControlLeft));
    t.on_event(&press(Key::Alt));
    assert_eq!(t.on_event(&press(Key::KeyD)), Some(HotkeyAction::Toggle));
    t.on_event(&release(Key::KeyD));
    t.on_event(&release(Key::ControlLeft));
    t.on_event(&release(Key::Alt));
    assert_eq!(
        t.on_event(&press(Key::KeyD)),
        None,
        "typing d mid-session must not end the dictation"
    );
}

// Presence is checked, absence is not: a decision, not an accident
#[test]
fn extra_modifiers_do_not_block_the_chord() {
    let mut t = tracker("Ctrl+Alt+D", HotkeyMode::Hold);
    t.on_event(&press(Key::ControlLeft));
    t.on_event(&press(Key::Alt));
    t.on_event(&press(Key::MetaLeft));
    assert_eq!(t.on_event(&press(Key::KeyD)), Some(HotkeyAction::Start));
}

#[test]
fn a_released_twin_does_not_clear_its_held_pair() {
    let mut t = tracker("Ctrl+D", HotkeyMode::Hold);
    t.on_event(&press(Key::ControlLeft));
    t.on_event(&press(Key::ControlRight));
    t.on_event(&release(Key::ControlRight));
    assert_eq!(
        t.on_event(&press(Key::KeyD)),
        Some(HotkeyAction::Start),
        "LeftControl is still down"
    );
}

#[test]
fn a_lone_modifier_used_as_a_modifier_cancels_instead_of_stopping() {
    let cases = [
        (
            "a letter typed with it",
            vec![press(Key::KeyE), release(Key::KeyE)],
            "RightOption+E typed a letter; it must not dictate",
        ),
        (
            "another modifier",
            vec![press(Key::MetaLeft), release(Key::MetaLeft)],
            "another modifier also makes it a chord",
        ),
        (
            "a mouse click",
            vec![EventType::ButtonPress(Button::Left)],
            "Option+click is a gesture, not a dictation",
        ),
    ];

    for (name, during, why) in cases {
        let mut t = tracker("RightOption", HotkeyMode::Hold);
        assert_eq!(
            t.on_event(&press(Key::AltGr)),
            Some(HotkeyAction::Start),
            "{name}"
        );
        for event in &during {
            assert_eq!(t.on_event(event), None, "{name}");
        }
        assert_eq!(
            t.on_event(&release(Key::AltGr)),
            Some(HotkeyAction::Cancel),
            "{name}: {why}"
        );
        // A clean hold still transcribes
        assert_eq!(
            t.on_event(&press(Key::AltGr)),
            Some(HotkeyAction::Start),
            "{name}"
        );
        assert_eq!(
            t.on_event(&release(Key::AltGr)),
            Some(HotkeyAction::Stop),
            "{name}"
        );
    }
}

#[test]
fn an_orphan_release_is_not_a_tap() {
    let mut t = tracker("RightOption", HotkeyMode::Toggle);
    assert_eq!(t.on_event(&release(Key::AltGr)), None);
    assert_eq!(t.on_event(&release(Key::AltGr)), None);
    // A real tap still toggles
    t.on_event(&press(Key::AltGr));
    assert_eq!(t.on_event(&release(Key::AltGr)), Some(HotkeyAction::Toggle));
}

#[test]
fn shift_does_not_turn_the_hold_into_a_chord() {
    let mut t = tracker("RightOption", HotkeyMode::Hold);
    t.on_event(&press(Key::ShiftLeft));
    assert_eq!(t.on_event(&press(Key::AltGr)), Some(HotkeyAction::Start));
    t.on_event(&release(Key::ShiftLeft));
    // Pressed mid-hold: the one ordering where the exemption decides
    t.on_event(&press(Key::ShiftLeft));
    assert_eq!(
        t.on_event(&release(Key::AltGr)),
        Some(HotkeyAction::Stop),
        "Shift must not cancel the session"
    );
}

#[test]
fn toggle_with_a_lone_modifier_acts_on_the_release() {
    let mut t = tracker("RightOption", HotkeyMode::Toggle);
    assert_eq!(
        t.on_event(&press(Key::AltGr)),
        None,
        "the press may yet become a chord"
    );
    assert_eq!(t.on_event(&release(Key::AltGr)), Some(HotkeyAction::Toggle));
    // Used as a modifier, it toggles nothing
    assert_eq!(t.on_event(&press(Key::AltGr)), None);
    t.on_event(&press(Key::KeyE));
    t.on_event(&release(Key::KeyE));
    assert_eq!(t.on_event(&release(Key::AltGr)), None);
    // A clean tap toggles again
    t.on_event(&press(Key::AltGr));
    assert_eq!(t.on_event(&release(Key::AltGr)), Some(HotkeyAction::Toggle));
}
