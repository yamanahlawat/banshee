use banshee_common::error::BansheeError;
use banshee_common::utils::db_path;
use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, ToSqlOutput, ValueRef};
use rusqlite::{Connection, ToSql};
use rusqlite_migration::{M, Migrations};

const MIGRATIONS: &[M<'static>] = &[M::up(
    "CREATE TABLE IF NOT EXISTS transcriptions (
         id INTEGER PRIMARY KEY, text TEXT NOT NULL,
         timestamp TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP);
     CREATE TABLE history (
         id INTEGER PRIMARY KEY AUTOINCREMENT,
         kind TEXT NOT NULL,
         question TEXT,
         text TEXT,
         outcome TEXT NOT NULL,
         reason TEXT,
         timestamp TEXT NOT NULL);
     INSERT INTO history (id, kind, text, outcome, timestamp)
         SELECT id, 'dictation', text, 'transcribed',
                CASE WHEN timestamp GLOB '*[Zz]' OR timestamp GLOB '*[+-][0-9][0-9]:[0-9][0-9]'
                     THEN timestamp ELSE replace(timestamp, ' ', 'T') || 'Z' END
           FROM transcriptions ORDER BY id;
     DROP TABLE transcriptions;",
)];

pub fn open() -> Result<Connection, BansheeError> {
    let path =
        db_path().ok_or_else(|| BansheeError::Other("Failed to get database path".to_string()))?;
    let mut connection = Connection::open(&path).map_err(|e| BansheeError::Other(e.to_string()))?;
    prepare(&mut connection)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| BansheeError::file(&path, e))?;
    }
    Ok(connection)
}

pub fn prepare(conn: &mut Connection) -> Result<(), BansheeError> {
    conn.pragma_update(None, "secure_delete", "ON")
        .map_err(|e| BansheeError::Other(e.to_string()))?;
    Migrations::from_slice(MIGRATIONS)
        .to_latest(conn)
        .map_err(|e| BansheeError::Other(e.to_string()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Dictation,
    Question,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Transcribed,
    Failed,
    NoAnswer,
    Closed,
    Unasked,
}

macro_rules! stored_as_text {
    ($type:ident { $($variant:ident => $text:literal),+ }) => {
        impl ToSql for $type {
            fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
                Ok(ToSqlOutput::from(match self {
                    $($type::$variant => $text),+
                }))
            }
        }

        impl FromSql for $type {
            fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
                match value.as_str()? {
                    $($text => Ok($type::$variant),)+
                    other => Err(FromSqlError::Other(
                        format!("unknown {}: {other}", stringify!($type)).into(),
                    )),
                }
            }
        }
    };
}

stored_as_text!(Kind { Dictation => "dictation", Question => "question" });
stored_as_text!(Outcome {
    Transcribed => "transcribed",
    Failed => "failed",
    NoAnswer => "no_answer",
    Closed => "closed",
    Unasked => "unasked"
});

#[derive(Debug, serde::Serialize)]
pub struct Entry {
    pub id: i64,
    pub kind: Kind,
    pub question: Option<String>,
    pub text: Option<String>,
    pub outcome: Outcome,
    pub reason: Option<String>,
    pub timestamp: String,
}

pub struct NewEntry<'a> {
    pub kind: Kind,
    pub question: Option<&'a str>,
    pub text: Option<&'a str>,
    pub outcome: Outcome,
    pub reason: Option<&'a str>,
}

impl<'a> NewEntry<'a> {
    pub fn dictation(text: &'a str) -> Self {
        Self {
            kind: Kind::Dictation,
            question: None,
            text: Some(text),
            outcome: Outcome::Transcribed,
            reason: None,
        }
    }

    pub fn failed_dictation(reason: &'a str) -> Self {
        Self {
            kind: Kind::Dictation,
            question: None,
            text: None,
            outcome: Outcome::Failed,
            reason: Some(reason),
        }
    }

    pub fn question(
        question: &'a str,
        text: Option<&'a str>,
        outcome: Outcome,
        reason: Option<&'a str>,
    ) -> Self {
        Self {
            kind: Kind::Question,
            question: Some(question),
            text,
            outcome,
            reason,
        }
    }
}

/// RFC 3339 with the machine's offset. The offset pins the instant, and
/// keeps what the speaker's own clock read.
pub fn stamp_now() -> String {
    chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, false)
}

pub fn insert(conn: &Connection, entry: &NewEntry) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO history (kind, question, text, outcome, reason, timestamp)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![
            entry.kind,
            entry.question,
            entry.text,
            entry.outcome,
            entry.reason,
            stamp_now()
        ],
    )?;
    Ok(())
}

/// Oldest first. A limit keeps the newest `limit` rows, still oldest first.
pub fn list(
    conn: &Connection,
    limit: Option<u32>,
    kind: Option<Kind>,
) -> rusqlite::Result<Vec<Entry>> {
    let mut stmt = conn.prepare(
        "SELECT id, kind, question, text, outcome, reason, timestamp FROM history
          WHERE ?1 IS NULL OR kind = ?1
          ORDER BY id DESC LIMIT coalesce(?2, -1)",
    )?;
    let mut newest_first = stmt
        .query_map(rusqlite::params![kind, limit], |row| {
            Ok(Entry {
                id: row.get(0)?,
                kind: row.get(1)?,
                question: row.get(2)?,
                text: row.get(3)?,
                outcome: row.get(4)?,
                reason: row.get(5)?,
                timestamp: row.get(6)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<Entry>>>()?;
    newest_first.reverse();
    Ok(newest_first)
}

pub fn clear(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM history", [])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memory() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        prepare(&mut conn).unwrap();
        conn
    }

    #[test]
    fn todays_rows_become_dictations_with_their_zone_named() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE transcriptions (id INTEGER PRIMARY KEY, text TEXT NOT NULL,
                 timestamp TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP);
             INSERT INTO transcriptions (text, timestamp) VALUES
                 ('older', '2026-08-27 12:07:24'),
                 ('newer', '2026-08-27T17:37:24+05:30');",
        )
        .unwrap();
        prepare(&mut conn).unwrap();
        let rows = list(&conn, None, None).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].kind, Kind::Dictation);
        assert_eq!(rows[0].outcome, Outcome::Transcribed);
        assert_eq!(rows[0].text.as_deref(), Some("older"));
        assert_eq!(rows[0].timestamp, "2026-08-27T12:07:24Z");
        assert_eq!(rows[1].timestamp, "2026-08-27T17:37:24+05:30");
    }

    #[test]
    fn a_second_open_changes_nothing() {
        let mut conn = memory();
        insert(&conn, &NewEntry::dictation("kept")).unwrap();
        let before = list(&conn, None, None).unwrap();

        prepare(&mut conn).unwrap();

        let after = list(&conn, None, None).unwrap();
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].id, before[0].id);
        assert_eq!(after[0].text.as_deref(), Some("kept"));
    }

    #[test]
    fn a_question_row_keeps_its_question_answer_and_outcome() {
        let conn = memory();
        insert(
            &conn,
            &NewEntry::question(
                "Open the pull request?",
                Some("Yes"),
                Outcome::Transcribed,
                None,
            ),
        )
        .unwrap();

        let rows = list(&conn, None, None).unwrap();

        assert_eq!(rows[0].kind, Kind::Question);
        assert_eq!(rows[0].question.as_deref(), Some("Open the pull request?"));
        assert_eq!(rows[0].text.as_deref(), Some("Yes"));
        assert_eq!(rows[0].outcome, Outcome::Transcribed);
        assert_eq!(rows[0].reason, None);
    }

    #[test]
    fn a_failed_row_has_no_text_and_keeps_its_reason() {
        let conn = memory();
        insert(
            &conn,
            &NewEntry::failed_dictation("The server did not answer."),
        )
        .unwrap();

        let rows = list(&conn, None, None).unwrap();

        assert_eq!(rows[0].text, None);
        assert_eq!(rows[0].outcome, Outcome::Failed);
        assert_eq!(
            rows[0].reason.as_deref(),
            Some("The server did not answer.")
        );
    }

    #[test]
    fn a_kind_filter_and_a_limit_return_the_newest_of_that_kind_oldest_first() {
        let conn = memory();
        insert(&conn, &NewEntry::dictation("a")).unwrap();
        insert(
            &conn,
            &NewEntry::question("q1", Some("x"), Outcome::Transcribed, None),
        )
        .unwrap();
        insert(&conn, &NewEntry::dictation("b")).unwrap();
        insert(&conn, &NewEntry::dictation("c")).unwrap();
        insert(
            &conn,
            &NewEntry::question("q2", Some("y"), Outcome::Transcribed, None),
        )
        .unwrap();

        let dictations = list(&conn, Some(2), Some(Kind::Dictation)).unwrap();
        let texts: Vec<_> = dictations.iter().map(|e| e.text.as_deref()).collect();
        assert_eq!(texts, vec![Some("b"), Some("c")]);

        let questions = list(&conn, None, Some(Kind::Question)).unwrap();
        let asked: Vec<_> = questions.iter().map(|e| e.question.as_deref()).collect();
        assert_eq!(asked, vec![Some("q1"), Some("q2")]);
    }

    #[test]
    fn ids_are_not_reused_after_a_clear() {
        let conn = memory();
        insert(&conn, &NewEntry::dictation("one")).unwrap();
        let first = list(&conn, None, None).unwrap()[0].id;

        clear(&conn).unwrap();
        insert(&conn, &NewEntry::dictation("two")).unwrap();

        assert!(list(&conn, None, None).unwrap()[0].id > first);
    }

    #[test]
    fn secure_delete_is_on() {
        let conn = memory();
        let on: i64 = conn
            .query_row("PRAGMA secure_delete", [], |row| row.get(0))
            .unwrap();
        assert_eq!(on, 1);
    }

    #[test]
    fn a_new_row_names_the_zone_it_was_spoken_in() {
        let conn = memory();
        insert(&conn, &NewEntry::dictation("Yes, open the pull request.")).unwrap();

        let rows = list(&conn, None, None).unwrap();

        let stamp = &rows[0].timestamp;
        // `CURRENT_TIMESTAMP` answers `2026-08-27 12:07:24`, and a reader is
        // free to misread that as local time.
        assert!(
            chrono::DateTime::parse_from_rfc3339(stamp).is_ok(),
            "not an unambiguous instant: {stamp}"
        );
    }
}
