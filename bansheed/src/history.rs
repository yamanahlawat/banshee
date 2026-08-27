use rusqlite::{Connection, Result};

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
        Ok(())
    }

    pub fn insert(conn: &Connection, text: &str) -> Result<()> {
        conn.execute(
            "INSERT INTO transcriptions (text) VALUES (?1)",
            rusqlite::params![text],
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
}
