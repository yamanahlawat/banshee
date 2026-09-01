use banshee_common::error::BansheeError;
use banshee_common::utils::get_db_path;
use rusqlite::{Connection, Result};

/// The history file, ready to write. Startup and a live `daemon.save_history`
/// both reach the table through this, so the schema is created in one place.
pub fn open() -> std::result::Result<Connection, BansheeError> {
    let path = get_db_path()
        .ok_or_else(|| BansheeError::Other("Failed to get database path".to_string()))?;
    let connection = Connection::open(path).map_err(|e| BansheeError::Other(e.to_string()))?;
    TranscriptionHistory::create_table(&connection)
        .map_err(|e| BansheeError::Other(e.to_string()))?;
    Ok(connection)
}

#[derive(Debug, serde::Serialize)]
pub struct TranscriptionHistory {
    pub id: i64,
    pub text: String,
    pub timestamp: String,
}

impl TranscriptionHistory {
    pub fn new(id: i64, text: String, timestamp: String) -> Self {
        Self {
            id,
            text,
            timestamp,
        }
    }

    pub fn create_table(conn: &Connection) -> Result<()> {
        conn.execute(
            "CREATE TABLE IF NOT EXISTS transcriptions (
                id INTEGER PRIMARY KEY,
                text TEXT NOT NULL,
                timestamp TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            )",
            [],
        )?;
        Self::name_the_zone_of_older_rows(conn)?;
        Ok(())
    }

    /// SQLite's `CURRENT_TIMESTAMP` answers UTC, so marking these rows says
    /// what they already are.
    fn name_the_zone_of_older_rows(conn: &Connection) -> Result<()> {
        conn.execute(
            "UPDATE transcriptions
                SET timestamp = replace(timestamp, ' ', 'T') || 'Z'
              WHERE timestamp NOT GLOB '*[Zz]'
                AND timestamp NOT GLOB '*[+-][0-9][0-9]:[0-9][0-9]'",
            [],
        )?;
        Ok(())
    }

    /// RFC 3339 with the machine's offset. The offset pins the instant, and
    /// keeps what the speaker's own clock read.
    pub fn stamp_now() -> String {
        chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, false)
    }

    pub fn insert(conn: &Connection, text: &str) -> Result<()> {
        conn.execute(
            "INSERT INTO transcriptions (text, timestamp) VALUES (?1, ?2)",
            rusqlite::params![text, Self::stamp_now()],
        )?;
        Ok(())
    }

    pub fn list(conn: &Connection, limit: Option<u32>) -> Result<Vec<TranscriptionHistory>> {
        let to_entry = |row: &rusqlite::Row| -> Result<TranscriptionHistory> {
            Ok(TranscriptionHistory::new(
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
            ))
        };
        match limit {
            None => {
                let mut stmt =
                    conn.prepare("SELECT id, text, timestamp FROM transcriptions ORDER BY id ASC")?;
                stmt.query_map([], to_entry)?.collect()
            }
            Some(limit) => {
                let mut stmt = conn.prepare(
                    "SELECT id, text, timestamp FROM transcriptions ORDER BY id DESC LIMIT ?1",
                )?;
                let mut newest_first: Vec<TranscriptionHistory> = stmt
                    .query_map(rusqlite::params![limit], to_entry)?
                    .collect::<Result<_>>()?;
                newest_first.reverse();
                Ok(newest_first)
            }
        }
    }

    pub fn clear(conn: &Connection) -> Result<()> {
        conn.execute("DELETE FROM transcriptions", [])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(entries: &[TranscriptionHistory]) -> Vec<&str> {
        entries.iter().map(|entry| entry.text.as_str()).collect()
    }

    #[test]
    fn a_limit_returns_the_newest_rows_oldest_first() {
        let conn = crate::test_support::seeded_history(&["first", "second", "third", "fourth"]);

        let all = TranscriptionHistory::list(&conn, None).unwrap();
        assert_eq!(texts(&all), vec!["first", "second", "third", "fourth"]);

        let limited = TranscriptionHistory::list(&conn, Some(2)).unwrap();
        assert_eq!(texts(&limited), vec!["third", "fourth"]);
    }

    #[test]
    fn an_older_zone_less_row_is_marked_as_the_utc_it_already_was() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        TranscriptionHistory::create_table(&conn).unwrap();
        conn.execute(
            "INSERT INTO transcriptions (text, timestamp) VALUES ('older', '2026-08-27 12:07:24')",
            [],
        )
        .unwrap();

        TranscriptionHistory::create_table(&conn).unwrap();

        let rows = TranscriptionHistory::list(&conn, None).unwrap();
        assert_eq!(rows[0].timestamp, "2026-08-27T12:07:24Z");
    }

    #[test]
    fn a_row_that_already_names_its_zone_is_left_alone() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        TranscriptionHistory::create_table(&conn).unwrap();
        for stamp in ["2026-08-27T17:37:24+05:30", "2026-08-27T12:07:24Z"] {
            conn.execute(
                "INSERT INTO transcriptions (text, timestamp) VALUES ('kept', ?1)",
                rusqlite::params![stamp],
            )
            .unwrap();
        }

        TranscriptionHistory::create_table(&conn).unwrap();

        let rows = TranscriptionHistory::list(&conn, None).unwrap();
        assert_eq!(rows[0].timestamp, "2026-08-27T17:37:24+05:30");
        assert_eq!(rows[1].timestamp, "2026-08-27T12:07:24Z");
    }

    #[test]
    fn a_new_row_names_the_zone_it_was_spoken_in() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        TranscriptionHistory::create_table(&conn).unwrap();
        TranscriptionHistory::insert(&conn, "Yes, open the pull request.").unwrap();

        let rows = TranscriptionHistory::list(&conn, None).unwrap();

        let stamp = &rows[0].timestamp;
        // `CURRENT_TIMESTAMP` answers `2026-08-27 12:07:24`, and a reader is
        // free to misread that as local time.
        assert!(
            chrono::DateTime::parse_from_rfc3339(stamp).is_ok(),
            "not an unambiguous instant: {stamp}"
        );
    }
}
