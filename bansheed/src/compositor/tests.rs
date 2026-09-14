use super::*;
use std::path::PathBuf;

fn scratch(name: &str) -> PathBuf {
    crate::test_support::scratch(&format!("compositor-{name}"))
}

fn written(changes: &[Change]) -> (&PathBuf, &Option<String>, &String) {
    let Some(Change::WriteFile {
        path,
        before,
        after,
        ..
    }) = changes.first()
    else {
        panic!("the first change must write the config: {changes:?}");
    };
    (path, before, after)
}

fn key(text: &str) -> Hotkey {
    Hotkey::try_from(text.to_string()).unwrap()
}

const LUA_HOLD: &str = "\
-- BEGIN BANSHEE MANAGED BLOCK
o.bind(\"F9\", \"Banshee: hold to dictate\", \"banshee record start --dictate\")
o.bind(\"F9\", nil, \"banshee record stop\", { release = true })
o.bind(\"SHIFT + F9\", \"Banshee: hold to record\", \"banshee record start\")
o.bind(\"SHIFT + F9\", nil, \"banshee record stop\", { release = true })
-- END BANSHEE MANAGED BLOCK
";

const LUA_TAP: &str = "\
-- BEGIN BANSHEE MANAGED BLOCK
o.bind(\"F9\", \"Banshee: tap to dictate\", \"banshee record toggle --dictate\")
o.bind(\"SHIFT + F9\", \"Banshee: tap to record\", \"banshee record toggle\")
-- END BANSHEE MANAGED BLOCK
";

const CONF_HOLD: &str = "\
# BEGIN BANSHEE MANAGED BLOCK
bind  = , F9, exec, banshee record start --dictate
bindr = , F9, exec, banshee record stop
bind  = SHIFT, F9, exec, banshee record start
bindr = SHIFT, F9, exec, banshee record stop
# END BANSHEE MANAGED BLOCK
";

const CONF_TAP: &str = "\
# BEGIN BANSHEE MANAGED BLOCK
bind = , F9, exec, banshee record toggle --dictate
bind = SHIFT, F9, exec, banshee record toggle
# END BANSHEE MANAGED BLOCK
";

#[test]
fn an_omarchy_lua_config_gets_the_lua_block() {
    let hypr = scratch("lua");
    std::fs::write(hypr.join("bindings.lua"), "-- mine\n").unwrap();

    let rebind = plan(&hypr, key("F9"), HotkeyMode::Hold).unwrap();

    let (path, before, after) = written(&rebind.changes);
    assert_eq!(path, &hypr.join("bindings.lua"));
    assert_eq!(rebind.path, hypr.join("bindings.lua"));
    assert_eq!(before.as_deref(), Some("-- mine\n"));
    assert_eq!(after, &format!("-- mine\n\n{LUA_HOLD}"));
    let _ = std::fs::remove_dir_all(&hypr);
}

#[test]
fn a_plain_hyprland_config_gets_the_conf_block() {
    let hypr = scratch("conf");
    std::fs::write(hypr.join("hyprland.conf"), "monitor=,preferred,auto,1\n").unwrap();

    let rebind = plan(&hypr, key("F9"), HotkeyMode::Hold).unwrap();

    let (path, _, after) = written(&rebind.changes);
    assert_eq!(path, &hypr.join("hyprland.conf"));
    assert_eq!(after, &format!("monitor=,preferred,auto,1\n\n{CONF_HOLD}"));
    let _ = std::fs::remove_dir_all(&hypr);
}

#[test]
fn tap_binds_one_toggle_per_key_with_no_release_bind_in_lua() {
    let hypr = scratch("lua-tap");
    std::fs::write(hypr.join("bindings.lua"), "-- mine\n").unwrap();

    let rebind = plan(&hypr, key("F9"), HotkeyMode::Toggle).unwrap();

    let (_, _, after) = written(&rebind.changes);
    assert_eq!(after, &format!("-- mine\n\n{LUA_TAP}"));
    let _ = std::fs::remove_dir_all(&hypr);
}

#[test]
fn tap_binds_one_toggle_per_key_with_no_release_bind_in_conf() {
    let hypr = scratch("conf-tap");
    std::fs::write(hypr.join("hyprland.conf"), "monitor=,preferred,auto,1\n").unwrap();

    let rebind = plan(&hypr, key("F9"), HotkeyMode::Toggle).unwrap();

    let (_, _, after) = written(&rebind.changes);
    assert_eq!(after, &format!("monitor=,preferred,auto,1\n\n{CONF_TAP}"));
    let _ = std::fs::remove_dir_all(&hypr);
}

#[test]
fn a_chord_key_takes_hyprland_modifier_names_in_lua() {
    let hypr = scratch("lua-chord");
    std::fs::write(hypr.join("bindings.lua"), "").unwrap();

    let rebind = plan(&hypr, key("Ctrl+Alt+D"), HotkeyMode::Hold).unwrap();

    let (_, _, after) = written(&rebind.changes);
    assert_eq!(
        after,
        "-- BEGIN BANSHEE MANAGED BLOCK
o.bind(\"CTRL + ALT + D\", \"Banshee: hold to dictate\", \"banshee record start --dictate\")
o.bind(\"CTRL + ALT + D\", nil, \"banshee record stop\", { release = true })
o.bind(\"SHIFT + CTRL + ALT + D\", \"Banshee: hold to record\", \"banshee record start\")
o.bind(\"SHIFT + CTRL + ALT + D\", nil, \"banshee record stop\", { release = true })
-- END BANSHEE MANAGED BLOCK
"
    );
    let _ = std::fs::remove_dir_all(&hypr);
}

#[test]
fn a_chord_key_takes_hyprland_modifier_names_in_conf() {
    let hypr = scratch("conf-chord");
    std::fs::write(hypr.join("hyprland.conf"), "").unwrap();

    let rebind = plan(&hypr, key("Cmd+7"), HotkeyMode::Toggle).unwrap();

    let (_, _, after) = written(&rebind.changes);
    assert_eq!(
        after,
        "# BEGIN BANSHEE MANAGED BLOCK
bind = SUPER, 7, exec, banshee record toggle --dictate
bind = SHIFT SUPER, 7, exec, banshee record toggle
# END BANSHEE MANAGED BLOCK
"
    );
    let _ = std::fs::remove_dir_all(&hypr);
}

#[test]
fn choosing_tap_replaces_a_hold_block_in_place() {
    let hypr = scratch("lua-hold-to-tap");
    let before = format!("-- mine\n\n{LUA_HOLD}-- later\n");
    std::fs::write(hypr.join("bindings.lua"), &before).unwrap();

    let rebind = plan(&hypr, key("F9"), HotkeyMode::Toggle).unwrap();

    let (_, _, after) = written(&rebind.changes);
    assert_eq!(after, &format!("-- mine\n\n{LUA_TAP}-- later\n"));
    let _ = std::fs::remove_dir_all(&hypr);
}

#[test]
fn choosing_hold_replaces_a_tap_block_in_place() {
    let hypr = scratch("conf-tap-to-hold");
    let before = format!("monitor=,preferred,auto,1\n\n{CONF_TAP}exec-once = waybar\n");
    std::fs::write(hypr.join("hyprland.conf"), &before).unwrap();

    let rebind = plan(&hypr, key("F9"), HotkeyMode::Hold).unwrap();

    let (_, _, after) = written(&rebind.changes);
    assert_eq!(
        after,
        &format!("monitor=,preferred,auto,1\n\n{CONF_HOLD}exec-once = waybar\n")
    );
    let _ = std::fs::remove_dir_all(&hypr);
}

#[test]
fn a_bound_file_with_a_section_below_the_block_needs_no_change() {
    let hypr = scratch("lua-bound-with-section");
    let bound = format!("-- mine\n\n{LUA_TAP}\n-- later\n");
    std::fs::write(hypr.join("bindings.lua"), &bound).unwrap();

    let rebind = plan(&hypr, key("F9"), HotkeyMode::Toggle).unwrap();

    assert_eq!(rebind.changes, Vec::new());
    assert_eq!(rebind.strays, Vec::<usize>::new());
    let _ = std::fs::remove_dir_all(&hypr);
}

#[test]
fn a_banshee_record_bind_outside_the_markers_is_left_alone_and_named() {
    let hypr = scratch("lua-own-bind");
    let own = "o.bind(\"F12\", \"mine\", \"banshee record start\")\n";
    std::fs::write(hypr.join("bindings.lua"), format!("{own}\n{LUA_HOLD}")).unwrap();

    let rebind = plan(&hypr, key("F9"), HotkeyMode::Toggle).unwrap();

    let (_, _, after) = written(&rebind.changes);
    assert_eq!(after, &format!("{own}\n{LUA_TAP}"));
    assert_eq!(rebind.strays, vec![1]);
    let _ = std::fs::remove_dir_all(&hypr);
}

#[test]
fn a_start_marker_with_no_end_marker_is_refused_by_name() {
    let hypr = scratch("lua-no-end");
    let cut = "-- BEGIN BANSHEE MANAGED BLOCK\no.bind(\"F9\", nil, \"banshee record toggle\")\n";
    std::fs::write(hypr.join("bindings.lua"), cut).unwrap();

    let Err(BansheeError::Rejected(reason)) = plan(&hypr, key("F9"), HotkeyMode::Toggle) else {
        panic!("a block with no end marker has no known extent, so it must be refused");
    };
    assert!(reason.contains("bindings.lua"), "{reason}");
    assert!(reason.contains("END BANSHEE MANAGED BLOCK"), "{reason}");
    let _ = std::fs::remove_dir_all(&hypr);
}

#[test]
fn older_banshee_lines_stay_and_are_named_below_the_appended_block() {
    let hypr = scratch("lua-older-lines");
    let before = "-- o.bind(\"SUPER + H\", nil, \"voxtype record toggle\")
-- o.bind(\"SUPER + PERIOD\", nil, \"omarchy-shell shell toggle omarchy.emojis\")

-- Banshee: global hotkey, X11-only, bound here for Wayland instead.
o.bind(\"F5\", \"Banshee: start dictation\", \"banshee record start --dictate\")
o.bind(\"F5\", nil, \"banshee record stop\", { release = true })
o.bind(\"SHIFT + F5\", \"Banshee: start recording\", \"banshee record start\")
o.bind(\"SHIFT + F5\", nil, \"banshee record stop\", { release = true })


-- Banshee
o.bind(\"F9\", \"Banshee: tap to dictate\", \"banshee record toggle --dictate\")
o.bind(\"SHIFT + F9\", \"Banshee: tap to record\", \"banshee record toggle\")
";
    std::fs::write(hypr.join("bindings.lua"), before).unwrap();

    let rebind = plan(&hypr, key("F5"), HotkeyMode::Toggle).unwrap();

    let (_, _, after) = written(&rebind.changes);
    assert_eq!(
        after,
        &format!(
            "{before}
-- BEGIN BANSHEE MANAGED BLOCK
o.bind(\"F5\", \"Banshee: tap to dictate\", \"banshee record toggle --dictate\")
o.bind(\"SHIFT + F5\", \"Banshee: tap to record\", \"banshee record toggle\")
-- END BANSHEE MANAGED BLOCK
"
        )
    );
    assert_eq!(rebind.strays, vec![5, 6, 7, 8, 12, 13]);
    let _ = std::fs::remove_dir_all(&hypr);
}

#[test]
fn a_bind_formatted_across_lines_stays_whole_and_is_named() {
    let hypr = scratch("lua-multiline");
    let formatted = "o.bind(
    \"SUPER + F9\",
    \"Banshee: hold to dictate\",
    \"banshee record start --dictate\"
)
";
    std::fs::write(hypr.join("bindings.lua"), formatted).unwrap();

    let rebind = plan(&hypr, key("F9"), HotkeyMode::Hold).unwrap();

    let (_, _, after) = written(&rebind.changes);
    assert_eq!(after, &format!("{formatted}\n{LUA_HOLD}"));
    assert_eq!(rebind.strays, vec![4]);
    let _ = std::fs::remove_dir_all(&hypr);
}

#[test]
fn an_appended_block_adds_no_second_blank_line() {
    let hypr = scratch("lua-trailing-blank");
    std::fs::write(hypr.join("bindings.lua"), "-- mine\n\n").unwrap();

    let rebind = plan(&hypr, key("F9"), HotkeyMode::Hold).unwrap();

    let (_, _, after) = written(&rebind.changes);
    assert_eq!(after, &format!("-- mine\n\n{LUA_HOLD}"));
    let _ = std::fs::remove_dir_all(&hypr);
}

#[test]
fn the_lua_file_wins_when_both_exist() {
    let hypr = scratch("both");
    std::fs::write(hypr.join("bindings.lua"), "").unwrap();
    std::fs::write(hypr.join("hyprland.conf"), "").unwrap();

    let rebind = plan(&hypr, key("F9"), HotkeyMode::Hold).unwrap();

    let (path, _, _) = written(&rebind.changes);
    assert_eq!(path, &hypr.join("bindings.lua"));
    let _ = std::fs::remove_dir_all(&hypr);
}

#[test]
fn a_config_that_holds_the_block_for_the_chosen_key_and_mode_needs_no_change() {
    for (mode, block) in [(HotkeyMode::Hold, LUA_HOLD), (HotkeyMode::Toggle, LUA_TAP)] {
        let hypr = scratch("bound");
        std::fs::write(hypr.join("bindings.lua"), format!("-- mine\n\n{block}")).unwrap();

        let rebind = plan(&hypr, key("F9"), mode).unwrap();

        assert_eq!(rebind.changes, Vec::new(), "{mode:?}");
        let _ = std::fs::remove_dir_all(&hypr);
    }
}

#[test]
fn a_directory_with_no_hyprland_config_is_refused_by_name() {
    let hypr = scratch("empty");

    let Err(BansheeError::Rejected(reason)) = plan(&hypr, key("F9"), HotkeyMode::Hold) else {
        panic!("a directory with no config must be refused");
    };
    assert!(reason.contains("bindings.lua"), "{reason}");
    assert!(reason.contains("hyprland.conf"), "{reason}");
    let _ = std::fs::remove_dir_all(&hypr);
}

#[test]
fn the_printed_block_matches_the_file_the_machine_uses() {
    let hypr = scratch("snippet");
    std::fs::write(hypr.join("hyprland.conf"), "").unwrap();
    assert!(
        Layout::detect(&hypr)
            .unwrap()
            .block(key("F9"), HotkeyMode::Toggle)
            .starts_with("# BEGIN BANSHEE MANAGED BLOCK\n")
    );
    std::fs::write(hypr.join("bindings.lua"), "").unwrap();
    assert!(
        Layout::detect(&hypr)
            .unwrap()
            .block(key("F9"), HotkeyMode::Toggle)
            .starts_with("-- BEGIN BANSHEE MANAGED BLOCK\n")
    );
    let _ = std::fs::remove_dir_all(&hypr);
}

#[test]
fn the_answer_to_hold_or_tap_names_a_mode() {
    let cases = [
        ("tap", HotkeyMode::Toggle),
        ("Tap", HotkeyMode::Toggle),
        ("hold", HotkeyMode::Hold),
        ("HOLD", HotkeyMode::Hold),
    ];
    for (answer, want) in cases {
        assert_eq!(mode_from_answer(answer).unwrap(), want, "{answer:?}");
    }
}

#[test]
fn an_answer_that_is_neither_hold_nor_tap_is_refused() {
    let Err(BansheeError::Rejected(reason)) = mode_from_answer("maybe") else {
        panic!("an unknown answer must be refused");
    };
    assert!(
        reason.contains("hold") && reason.contains("tap"),
        "{reason}"
    );
}

#[test]
fn the_answer_to_which_key_names_a_hotkey() {
    let cases = [("F5", "F5"), ("f6", "F6"), ("ctrl + alt + d", "Ctrl+Alt+D")];
    for (answer, want) in cases {
        assert_eq!(key_from_answer(answer).unwrap(), key(want), "{answer:?}");
    }
}

#[test]
fn a_lone_modifier_is_refused_because_hyprland_fires_it_on_release() {
    let Err(BansheeError::Rejected(reason)) = key_from_answer("RightOption") else {
        panic!("a lone modifier must be refused for Hyprland");
    };
    assert!(reason.contains("F-key"), "{reason}");
}

#[test]
fn a_key_the_parser_does_not_know_is_refused() {
    let Err(BansheeError::Rejected(reason)) = key_from_answer("Banana") else {
        panic!("an unknown key must be refused");
    };
    assert!(reason.contains("Banana"), "{reason}");
}

#[test]
fn the_default_key_is_the_saved_key_unless_hyprland_cannot_bind_it() {
    assert_eq!(default_key(key("Ctrl+Alt+D")), key("Ctrl+Alt+D"));
    assert_eq!(default_key(key("F5")), key("F5"));
    assert_eq!(default_key(Hotkey::default()), key("F9"));
}
