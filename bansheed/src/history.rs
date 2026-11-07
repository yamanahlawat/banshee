pub mod audio;

use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use banshee_common::error::BansheeError;
use banshee_common::utils::{audio_dir, db_path};
use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, ToSqlOutput, ValueRef};
use rusqlite::{Connection, OptionalExtension, ToSql};
use rusqlite_migration::{M, Migrations};

use crate::config::KeepAudio;

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

pub fn open(keep: KeepAudio) -> Result<Store, BansheeError> {
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
    let audio = audio_dir()
        .ok_or_else(|| BansheeError::Other("Failed to get the audio folder".to_string()))?;
    audio::prepare(&audio).map_err(|e| BansheeError::file(&audio, e))?;
    Ok(Store::new(connection, audio, keep))
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
    pub audio: Option<audio::Kept>,
}

pub struct NewEntry<'a> {
    pub kind: Kind,
    pub question: Option<&'a str>,
    pub text: Option<&'a str>,
    pub outcome: Outcome,
    pub reason: Option<&'a str>,
    /// The clip at 16 kHz, kept as the row's audio unless Keep audio is off.
    /// Empty when the row has none.
    pub audio: &'a [f32],
}

impl<'a> NewEntry<'a> {
    pub fn dictation(text: &'a str) -> Self {
        Self {
            kind: Kind::Dictation,
            question: None,
            text: Some(text),
            outcome: Outcome::Transcribed,
            reason: None,
            audio: &[],
        }
    }

    pub fn failed_dictation(reason: &'a str) -> Self {
        Self {
            kind: Kind::Dictation,
            question: None,
            text: None,
            outcome: Outcome::Failed,
            reason: Some(reason),
            audio: &[],
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
            audio: &[],
        }
    }

    pub fn with_audio(self, samples: &'a [f32]) -> Self {
        Self {
            audio: samples,
            ..self
        }
    }
}

/// RFC 3339 with the machine's offset. The offset pins the instant, and
/// keeps what the speaker's own clock read.
pub fn stamp(at: chrono::DateTime<chrono::Local>) -> String {
    at.to_rfc3339_opts(chrono::SecondsFormat::Secs, false)
}

pub fn stamp_now() -> String {
    stamp(chrono::Local::now())
}

pub fn insert(conn: &Connection, entry: &NewEntry) -> rusqlite::Result<i64> {
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
    Ok(conn.last_insert_rowid())
}

const COLUMNS: &str = "id, kind, question, text, outcome, reason, timestamp";

fn entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<Entry> {
    Ok(Entry {
        id: row.get(0)?,
        kind: row.get(1)?,
        question: row.get(2)?,
        text: row.get(3)?,
        outcome: row.get(4)?,
        reason: row.get(5)?,
        timestamp: row.get(6)?,
        audio: None,
    })
}

/// Oldest first. A limit keeps the newest `limit` rows, still oldest first.
pub fn list(
    conn: &Connection,
    limit: Option<u32>,
    kind: Option<Kind>,
) -> rusqlite::Result<Vec<Entry>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM history
          WHERE ?1 IS NULL OR kind = ?1
          ORDER BY id DESC LIMIT coalesce(?2, -1)"
    ))?;
    let mut newest_first = stmt
        .query_map(rusqlite::params![kind, limit], entry)?
        .collect::<rusqlite::Result<Vec<Entry>>>()?;
    newest_first.reverse();
    Ok(newest_first)
}

pub fn clear(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM history", [])?;
    Ok(())
}

/// The rows, the folder of their audio and how long that audio lives. A
/// caller holds it under one lock, so a row and its file come and go together.
pub struct Store {
    connection: Connection,
    audio: PathBuf,
    keep: KeepAudio,
}

impl Store {
    pub fn new(connection: Connection, audio: PathBuf, keep: KeepAudio) -> Self {
        Self {
            connection,
            audio,
            keep,
        }
    }

    pub fn keep(&self, entry: &NewEntry) -> rusqlite::Result<i64> {
        let id = insert(&self.connection, entry)?;
        if self.keep != KeepAudio::Off
            && !entry.audio.is_empty()
            && let Err(error) = audio::write(&self.audio, id, entry.audio)
        {
            log::error!("Failed to keep the audio of history row {id}: {error}");
        }
        Ok(id)
    }

    #[cfg(test)]
    pub fn keep_audio(&self) -> KeepAudio {
        self.keep
    }

    pub fn set_keep_audio(&mut self, keep: KeepAudio) {
        self.keep = keep;
    }

    pub fn audio_folder(&self) -> AudioFolder {
        AudioFolder {
            dir: self.audio.clone(),
            period: self.keep.period(),
        }
    }

    /// The rows without their audio: `AudioFolder::described` adds it.
    pub fn list(&self, limit: Option<u32>, kind: Option<Kind>) -> rusqlite::Result<Vec<Entry>> {
        list(&self.connection, limit, kind)
    }

    pub fn get(&self, id: i64) -> rusqlite::Result<Option<Entry>> {
        let row = self
            .connection
            .query_row(
                &format!("SELECT {COLUMNS} FROM history WHERE id = ?1"),
                [id],
                entry,
            )
            .optional()?;
        Ok(row.map(|mut row| {
            row.audio = audio::kept(&self.audio, row.id, self.keep.period());
            row
        }))
    }

    #[cfg(test)]
    pub fn audio_path(&self, id: i64) -> Option<PathBuf> {
        Some(audio::path(&self.audio, id)).filter(|path| path.is_file())
    }

    /// The row's kind and where its audio file would be, present or not.
    pub fn kind_and_audio_file(&self, id: i64) -> rusqlite::Result<Option<(Kind, PathBuf)>> {
        let kind = self
            .connection
            .query_row("SELECT kind FROM history WHERE id = ?1", [id], |row| {
                row.get(0)
            })
            .optional()?;
        Ok(kind.map(|kind| (kind, audio::path(&self.audio, id))))
    }

    pub fn retried(&self, id: i64, text: &str) -> rusqlite::Result<Entry> {
        self.connection.execute(
            "UPDATE history SET text = ?1, outcome = ?2, reason = NULL WHERE id = ?3",
            rusqlite::params![text, Outcome::Transcribed, id],
        )?;
        self.get(id)?.ok_or(rusqlite::Error::QueryReturnedNoRows)
    }

    /// The files go first, so a file that will not go keeps its row.
    pub fn clear(&self) -> Result<(), BansheeError> {
        audio::remove_all(&self.audio).map_err(|e| BansheeError::file(&self.audio, e))?;
        clear(&self.connection).map_err(|e| BansheeError::Other(e.to_string()))
    }
}

/// The audio folder and how long its files live, copied out of the store so
/// a scan of the folder runs without the history lock.
pub struct AudioFolder {
    dir: PathBuf,
    period: Option<Duration>,
}

impl AudioFolder {
    pub fn described(&self, mut rows: Vec<Entry>) -> Vec<Entry> {
        let ids = rows.iter().map(|row| row.id).collect();
        let mut kept = audio::kept_of(&self.dir, &ids, self.period);
        for row in &mut rows {
            row.audio = kept.remove(&row.id);
        }
        rows
    }

    pub fn purge(&self, now: SystemTime) {
        let Some(period) = self.period else {
            return;
        };
        let removed = audio::purge(&self.dir, period, now);
        if removed > 0 {
            log::info!("Removed {removed} kept audio files past their period");
        }
    }
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

    fn wav_files(dir: &std::path::Path) -> usize {
        std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "wav"))
            .count()
    }

    fn listed(store: &Store) -> Vec<Entry> {
        store
            .audio_folder()
            .described(store.list(None, None).unwrap())
    }

    #[test]
    fn insert_answers_the_new_rows_id() {
        let conn = memory();
        assert_eq!(insert(&conn, &NewEntry::dictation("one")).unwrap(), 1);
        assert_eq!(insert(&conn, &NewEntry::dictation("two")).unwrap(), 2);
    }

    #[test]
    fn a_kept_row_lists_its_audio() {
        let store = crate::test_support::seeded_history(&[]);
        let id = store
            .keep(&NewEntry::dictation("hello").with_audio(&[0.0; 16_000]))
            .unwrap();

        let rows = listed(&store);

        assert_eq!(rows[0].id, id);
        let audio = rows[0].audio.as_ref().expect("the row lists its audio");
        assert_eq!(audio.seconds, 1.0);
        assert!(audio.until.is_some());
        assert!(store.audio_path(id).is_some());
    }

    #[test]
    fn a_list_gives_each_row_its_own_audio() {
        let store = crate::test_support::seeded_history(&[]);
        let half = store
            .keep(&NewEntry::dictation("half").with_audio(&[0.0; 8_000]))
            .unwrap();
        let none = store.keep(&NewEntry::dictation("none")).unwrap();
        let whole = store
            .keep(&NewEntry::dictation("whole").with_audio(&[0.0; 16_000]))
            .unwrap();

        let rows = listed(&store);

        let seconds: Vec<_> = rows
            .iter()
            .map(|row| (row.id, row.audio.as_ref().map(|audio| audio.seconds)))
            .collect();
        assert_eq!(
            seconds,
            vec![(half, Some(0.5)), (none, None), (whole, Some(1.0))]
        );
    }

    #[test]
    fn keep_audio_off_writes_no_file() {
        let mut store = crate::test_support::seeded_history(&[]);
        store.set_keep_audio(KeepAudio::Off);
        let id = store
            .keep(&NewEntry::dictation("hello").with_audio(&[0.0; 16_000]))
            .unwrap();

        assert!(store.audio_path(id).is_none());
        let rows = listed(&store);
        assert_eq!(rows.len(), 1);
        assert!(rows[0].audio.is_none());
    }

    #[test]
    fn a_row_without_samples_has_no_audio() {
        let store = crate::test_support::seeded_history(&[]);
        let id = store
            .keep(&NewEntry::question(
                "Ship it?",
                None,
                Outcome::NoAnswer,
                None,
            ))
            .unwrap();

        assert!(store.audio_path(id).is_none());
        let row = store.get(id).unwrap().unwrap();
        assert!(row.audio.is_none());
    }

    #[test]
    fn empty_samples_write_no_file() {
        let store = crate::test_support::seeded_history(&[]);
        let id = store
            .keep(&NewEntry::failed_dictation("The stream broke.").with_audio(&[]))
            .unwrap();

        assert!(store.audio_path(id).is_none());
        assert_eq!(wav_files(&store.audio), 0);
        let row = store.get(id).unwrap().unwrap();
        assert!(row.audio.is_none());
    }

    #[test]
    fn clear_removes_the_audio_with_the_rows() {
        let mut store = crate::test_support::seeded_history(&[]);
        store.set_keep_audio(KeepAudio::Forever);
        for text in ["one", "two"] {
            store
                .keep(&NewEntry::dictation(text).with_audio(&[0.0; 160]))
                .unwrap();
        }
        assert_eq!(wav_files(&store.audio), 2);

        store.clear().unwrap();

        assert_eq!(wav_files(&store.audio), 0);
        assert!(store.list(None, None).unwrap().is_empty());
    }

    #[test]
    fn a_store_kept_forever_purges_nothing() {
        let mut store = crate::test_support::seeded_history(&[]);
        let now = SystemTime::now();
        store.set_keep_audio(KeepAudio::Forever);
        let id = store
            .keep(&NewEntry::dictation("old").with_audio(&[0.0; 160]))
            .unwrap();
        let file = store.audio_path(id).unwrap();
        crate::test_support::set_modified(&file, now - 400 * crate::test_support::DAY);

        store.audio_folder().purge(now);

        assert!(file.exists());
    }

    #[test]
    fn retried_replaces_the_text_and_clears_the_reason() {
        let store = crate::test_support::seeded_history(&[]);
        let id = store
            .keep(&NewEntry::failed_dictation("The server did not answer.").with_audio(&[0.0; 160]))
            .unwrap();

        let row = store.retried(id, "Open the pull request.").unwrap();

        assert_eq!(row.id, id);
        assert_eq!(row.text.as_deref(), Some("Open the pull request."));
        assert_eq!(row.outcome, Outcome::Transcribed);
        assert_eq!(row.reason, None);
        assert!(row.audio.is_some());
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
