use banshee_common::settings::{Domain, SETTINGS, find};
use serde_json::json;

/// Leaf keys of a parsed empty file, dotted.
fn config_leaves() -> std::collections::BTreeSet<String> {
    fn walk(
        prefix: &str,
        node: &serde_json::Value,
        leaves: &mut std::collections::BTreeSet<String>,
    ) {
        match node.as_object() {
            Some(fields) => {
                for (name, child) in fields {
                    let dotted = if prefix.is_empty() {
                        name.clone()
                    } else {
                        format!("{prefix}.{name}")
                    };
                    walk(&dotted, child, leaves);
                }
            }
            None => {
                leaves.insert(prefix.to_string());
            }
        }
    }
    let parsed = serde_json::to_value(crate::config::Config::parse("").unwrap()).unwrap();
    let mut leaves = std::collections::BTreeSet::new();
    walk("", &parsed, &mut leaves);
    leaves
}

fn value_at(node: &serde_json::Value, key: &str) -> Option<serde_json::Value> {
    key.split('.')
        .try_fold(node, |node, segment| node.get(segment))
        .cloned()
}

fn written(key: &str, value: serde_json::Value) -> bool {
    super::edit("", &[(key.to_string(), value)].into()).is_ok()
}

#[test]
fn every_config_field_has_one_row_and_every_row_a_field() {
    let mut expected = config_leaves();
    expected.insert("stt.remote.api_key".into());
    expected.insert("tts.remote.api_key".into());
    let rows: std::collections::BTreeSet<String> =
        SETTINGS.iter().map(|row| row.key.to_string()).collect();
    assert_eq!(rows, expected);
    assert_eq!(rows.len(), SETTINGS.len(), "a key has two rows");
}

#[test]
fn every_default_is_what_an_empty_file_reads() {
    let parsed = serde_json::to_value(crate::config::Config::parse("").unwrap()).unwrap();
    for row in SETTINGS.iter().filter(|row| row.domain != Domain::Secret) {
        let stated: serde_json::Value = serde_json::from_str(row.default.unwrap()).unwrap();
        assert_eq!(Some(stated), value_at(&parsed, row.key), "{}", row.key);
    }
}

#[test]
fn every_choice_parses_and_a_value_outside_is_refused() {
    let mut choices = 0;
    for row in SETTINGS {
        let Domain::Choice { values } = row.domain else {
            continue;
        };
        choices += 1;
        for value in values {
            assert!(written(row.key, json!(value)), "{} = {value}", row.key);
        }
        assert!(!written(row.key, json!("not-a-choice")), "{}", row.key);
    }
    assert_eq!(choices, 8);
}

/// The values serde's unknown-variant refusal lists, which are every value the
/// field's enum takes.
fn values_config_takes(key: &str) -> Vec<String> {
    let refusal = super::edit("", &[(key.to_string(), json!("not-a-choice"))].into())
        .expect_err("not-a-choice is no value")
        .to_string();
    let listed = refusal
        .lines()
        .find_map(|line| line.strip_prefix("unknown variant `not-a-choice`, expected "))
        .unwrap_or_else(|| panic!("{key}: no unknown-variant refusal in {refusal}"));
    listed
        .split('`')
        .skip(1)
        .step_by(2)
        .map(str::to_string)
        .collect()
}

#[test]
fn every_choice_lists_every_value_config_takes() {
    for row in SETTINGS {
        let Domain::Choice { values } = row.domain else {
            continue;
        };
        assert_eq!(values_config_takes(row.key), values, "{}", row.key);
    }
}

#[test]
fn every_range_is_the_range_config_set_enforces() {
    let whole_from_zero = Domain::Number {
        min: 0.0,
        max: None,
        whole: true,
    };
    for (key, stated, legal, refused) in [
        (
            "stt.vad_threshold",
            Domain::Number {
                min: 0.0,
                max: Some(1.0),
                whole: false,
            },
            vec![json!(0.0), json!(1.0)],
            vec![json!(-0.01), json!(1.01)],
        ),
        (
            "tts.remote.sample_rate",
            Domain::Number {
                min: 1.0,
                max: Some(4_294_967_295.0),
                whole: true,
            },
            vec![json!(1), json!(4294967295u64)],
            vec![json!(0), json!(4294967296u64)],
        ),
        (
            "stt.endpoint_silence_ms",
            whole_from_zero,
            vec![json!(0), json!(i64::MAX)],
            vec![json!(-1), json!(0.5), json!(i64::MAX as u64 + 1)],
        ),
        (
            "models.unload_after",
            whole_from_zero,
            vec![json!(0), json!(i64::MAX)],
            vec![json!(-1), json!(0.5), json!(i64::MAX as u64 + 1)],
        ),
    ] {
        assert_eq!(find(key).unwrap().domain, stated, "{key}");
        for value in legal {
            assert!(written(key, value.clone()), "{key} = {value}");
        }
        for value in refused {
            assert!(!written(key, value.clone()), "{key} = {value}");
        }
    }
    let numbers = SETTINGS
        .iter()
        .filter(|row| matches!(row.domain, Domain::Number { .. }))
        .count();
    assert_eq!(numbers, 4, "a number without a tested range");
}

/// `config set` removes a secret with an empty string instead.
#[test]
fn a_null_removes_every_row_but_a_secret() {
    for row in SETTINGS {
        let result = super::tests::configure_in_scratch(
            None,
            [(row.key.to_string(), serde_json::Value::Null)].into(),
            true,
        );
        if row.domain == Domain::Secret {
            let error = result.expect_err(row.key).to_string();
            assert!(error.contains("takes the key as a string"), "{error}");
        } else {
            assert!(result.is_ok(), "{}: {result:?}", row.key);
        }
    }
}

#[test]
fn every_secret_is_a_key_the_credentials_file_holds() {
    for row in SETTINGS {
        assert_eq!(
            row.domain == Domain::Secret,
            crate::credentials::RemoteKey::of_setting(row.key).is_some(),
            "{}",
            row.key
        );
    }
}

#[test]
fn the_rows_that_send_out_are_each_sides_provider_server_and_key() {
    use crate::credentials::RemoteKey;
    let sides: std::collections::BTreeSet<&str> = [RemoteKey::Stt, RemoteKey::Tts]
        .into_iter()
        .flat_map(|side| {
            [
                side.provider_setting(),
                side.base_url_setting(),
                side.setting(),
            ]
        })
        .collect();
    let rows: std::collections::BTreeSet<&str> = SETTINGS
        .iter()
        .filter(|row| row.sends_out)
        .map(|row| row.key)
        .collect();
    assert_eq!(rows, sides);
}

#[test]
fn every_row_that_sends_out_is_refused_without_persist() {
    for row in SETTINGS.iter().filter(|row| row.sends_out) {
        let value = match row.default {
            Some(default) => serde_json::from_str(default).unwrap(),
            None => json!("sk-x"),
        };
        let one: super::Assignments = [(row.key.to_string(), value)].into();
        assert!(
            super::refuse_sending_without_persist(&one, false).is_err(),
            "{}",
            row.key
        );
        assert!(
            super::refuse_sending_without_persist(&one, true).is_ok(),
            "{}",
            row.key
        );
    }
}

#[test]
fn a_write_without_persist_that_sends_out_names_why() {
    let dir = crate::test_support::unique_scratch("settings-table");
    let error = super::configure_at(
        &dir.join("config.toml"),
        &dir.join("credentials.toml"),
        None,
        [(
            "stt.remote.base_url".to_string(),
            json!("https://example.com/v1"),
        )]
        .into(),
        false,
    )
    .expect_err("a server is never set in memory only");
    assert!(
        error
            .to_string()
            .contains("'stt.remote.base_url' decides what leaves this machine"),
        "{error}"
    );
    assert!(!dir.join("config.toml").exists());
    assert!(!dir.join("credentials.toml").exists());
}

#[test]
fn every_row_applies_live() {
    for row in SETTINGS.iter().filter(|row| row.domain != Domain::Secret) {
        assert!(
            super::live(row.key).is_some(),
            "{} has no live apply",
            row.key
        );
    }
}
