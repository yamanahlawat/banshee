use super::*;

#[test]
fn the_shipped_default_still_parses() {
    assert_eq!(hotkey("RightOption").unwrap(), Hotkey::default());
}

#[test]
fn each_legal_form_parses() {
    assert_eq!(
        hotkey("F12").unwrap(),
        Hotkey::Key {
            ctrl: false,
            alt: false,
            cmd: false,
            key: Key::F12
        }
    );
    assert_eq!(hotkey("RightOption").unwrap(), Hotkey::Modifier(Key::AltGr));
    assert_eq!(
        hotkey("Ctrl+Alt+D").unwrap(),
        Hotkey::Key {
            ctrl: true,
            alt: true,
            cmd: false,
            key: Key::KeyD
        }
    );
    assert_eq!(
        hotkey("Cmd+7").unwrap(),
        Hotkey::Key {
            ctrl: false,
            alt: false,
            cmd: true,
            key: Key::Num7
        }
    );
}

#[test]
fn chord_modifiers_accept_their_long_names() {
    assert_eq!(
        hotkey("Control+Option+Command+D").unwrap(),
        Hotkey::Key {
            ctrl: true,
            alt: true,
            cmd: true,
            key: Key::KeyD
        }
    );
}

#[test]
fn spelling_is_forgiving_and_display_is_canonical() {
    assert_eq!(
        hotkey(" ctrl + alt + d ").unwrap().to_string(),
        "Ctrl+Alt+D"
    );
    assert_eq!(hotkey("rightoption").unwrap().to_string(), "RightOption");
    assert_eq!(hotkey("f5").unwrap().to_string(), "F5");
    assert_eq!(hotkey("cmd+7").unwrap().to_string(), "Cmd+7");
}

// One table drives the parser and Display, so every bindable name must
// round-trip; a name that prints "?" or re-parses differently is drift
#[test]
fn every_bindable_modifier_round_trips() {
    for modifier in MODIFIERS.iter().filter(|m| m.refused.is_none()) {
        let parsed = hotkey(modifier.name).unwrap();
        assert_eq!(
            parsed.to_string(),
            modifier.name,
            "display must give the name back"
        );
        assert_eq!(hotkey(&parsed.to_string()).unwrap(), parsed);
    }
}

#[test]
fn every_legal_form_survives_serialize_and_deserialize() {
    let forms: Vec<&str> = MODIFIERS
        .iter()
        .filter(|modifier| modifier.refused.is_none())
        .map(|modifier| modifier.name)
        .chain(["F1", "F12", "Ctrl+Alt+D", "Cmd+7"])
        .collect();

    for text in forms {
        let parsed = hotkey(text).unwrap();
        let json = serde_json::to_string(&parsed).unwrap();
        let deserialized: Hotkey = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, parsed, "{text} did not round trip");
    }
}

#[test]
fn a_bare_typing_key_is_refused_with_the_reason() {
    let error = hotkey("A").unwrap_err();
    assert!(error.contains("typing"), "{error}");
    assert!(error.contains("Ctrl, Alt or Cmd"), "{error}");
}

#[test]
fn shift_is_reserved_for_the_mailbox() {
    for text in ["Shift", "Shift+F5", "RightShift"] {
        let error = hotkey(text).unwrap_err();
        assert!(error.contains("mailbox"), "{text}: {error}");
    }
}

#[test]
fn any_f_key_outside_the_range_gets_one_message() {
    for text in ["F0", "F13", "F300"] {
        let error = hotkey(text).unwrap_err();
        assert!(error.contains("F1 to F12"), "{text}: {error}");
    }
}

#[test]
fn keys_rdev_cannot_deliver_are_refused_with_the_reason() {
    let caps = hotkey("CapsLock").unwrap_err();
    assert!(caps.contains("lock state"), "{caps}");
    #[cfg(target_os = "macos")]
    {
        let error = hotkey("RightControl").unwrap_err();
        assert!(error.contains("never maps"), "{error}");
        assert_eq!(hotkey("Fn").unwrap(), Hotkey::Modifier(Key::Function));
    }
    #[cfg(not(target_os = "macos"))]
    {
        assert_eq!(
            hotkey("RightControl").unwrap(),
            Hotkey::Modifier(Key::ControlRight)
        );
        let cmd = hotkey("RightCommand").unwrap_err();
        assert!(cmd.contains("MetaRight"), "{cmd}");
        let fn_key = hotkey("Fn").unwrap_err();
        assert!(fn_key.contains("Fn"), "{fn_key}");
    }
}

#[test]
fn nonsense_is_told_the_legal_forms() {
    let error = hotkey("banana").unwrap_err();
    assert!(error.contains("F1-F12"), "{error}");
    assert!(error.contains("RightOption"), "{error}");
    let chord = hotkey("Ctrl+RightOption").unwrap_err();
    assert!(chord.contains("ends in a key"), "{chord}");
    let modifier = hotkey("Meta+D").unwrap_err();
    assert!(modifier.contains("Ctrl, Alt or Cmd"), "{modifier}");
}

#[test]
fn a_refused_key_keeps_its_reason_inside_a_chord() {
    let error = hotkey("Ctrl+CapsLock").unwrap_err();
    assert!(error.contains("lock state"), "{error}");
}
