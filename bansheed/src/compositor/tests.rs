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

#[test]
fn an_omarchy_lua_config_gets_the_lua_block_and_a_reload() {
    let hypr = scratch("lua");
    std::fs::write(hypr.join("bindings.lua"), "-- mine\n").unwrap();

    let changes = plan(&hypr).unwrap();

    let (path, before, after) = written(&changes);
    assert_eq!(path, &hypr.join("bindings.lua"));
    assert_eq!(before.as_deref(), Some("-- mine\n"));
    assert!(after.starts_with("-- mine\n"), "{after}");
    assert!(after.contains("-- Banshee\n"), "{after}");
    assert!(
        after.contains(
            r#"o.bind("F9", "Banshee: hold to dictate", "banshee record start --dictate")"#
        ),
        "{after}"
    );
    assert!(
        after.contains(r#"o.bind("F9", nil, "banshee record stop", { release = true })"#),
        "{after}"
    );
    assert_eq!(
        changes.get(1),
        Some(&Change::Run {
            argv: vec!["hyprctl".to_string(), "reload".to_string()]
        })
    );
    let _ = std::fs::remove_dir_all(&hypr);
}

#[test]
fn a_plain_hyprland_config_gets_the_conf_block() {
    let hypr = scratch("conf");
    std::fs::write(hypr.join("hyprland.conf"), "monitor=,preferred,auto,1\n").unwrap();

    let changes = plan(&hypr).unwrap();

    let (path, _, after) = written(&changes);
    assert_eq!(path, &hypr.join("hyprland.conf"));
    assert!(after.contains("# Banshee\n"), "{after}");
    assert!(
        after.contains("bind  = , F9, exec, banshee record start --dictate\n"),
        "{after}"
    );
    assert!(
        after.contains("bindr = , F9, exec, banshee record stop\n"),
        "{after}"
    );
    assert!(
        after.contains("bind  = SHIFT, F9, exec, banshee record start\n"),
        "{after}"
    );
    assert!(
        after.contains("bindr = SHIFT, F9, exec, banshee record stop\n"),
        "{after}"
    );
    let _ = std::fs::remove_dir_all(&hypr);
}

#[test]
fn the_lua_file_wins_when_both_exist() {
    let hypr = scratch("both");
    std::fs::write(hypr.join("bindings.lua"), "").unwrap();
    std::fs::write(hypr.join("hyprland.conf"), "").unwrap();

    let changes = plan(&hypr).unwrap();

    let (path, _, _) = written(&changes);
    assert_eq!(path, &hypr.join("bindings.lua"));
    let _ = std::fs::remove_dir_all(&hypr);
}

#[test]
fn a_config_that_holds_the_block_needs_no_change() {
    let hypr = scratch("bound");
    std::fs::write(hypr.join("bindings.lua"), "-- mine\n").unwrap();
    let bound = plan(&hypr).unwrap();
    let (_, _, after) = written(&bound);
    std::fs::write(hypr.join("bindings.lua"), after).unwrap();

    assert_eq!(plan(&hypr).unwrap(), Vec::new());
    let _ = std::fs::remove_dir_all(&hypr);
}

#[test]
fn a_directory_with_no_hyprland_config_is_refused_by_name() {
    let hypr = scratch("empty");

    let Err(BansheeError::Rejected(reason)) = plan(&hypr) else {
        panic!("a directory with no config must be refused");
    };
    assert!(reason.contains("bindings.lua"), "{reason}");
    assert!(reason.contains("hyprland.conf"), "{reason}");
    let _ = std::fs::remove_dir_all(&hypr);
}

#[test]
fn the_layout_block_matches_the_file_the_machine_uses() {
    let hypr = scratch("snippet");
    std::fs::write(hypr.join("hyprland.conf"), "").unwrap();
    assert!(
        Layout::detect(&hypr)
            .unwrap()
            .block
            .starts_with("\n# Banshee\n")
    );
    std::fs::write(hypr.join("bindings.lua"), "").unwrap();
    assert!(
        Layout::detect(&hypr)
            .unwrap()
            .block
            .starts_with("\n-- Banshee\n")
    );
    let _ = std::fs::remove_dir_all(&hypr);
}
