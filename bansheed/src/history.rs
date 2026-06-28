use rusqlite::{Connection, Result};

#[derive(Debug)]
pub struct TranscriptionHistory {
    pub id: i64,
    pub text: String,
}

impl TranscriptionHistory {
    pub fn new(id: i64, text: String) -> Self {
        Self { id, text }
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

    pub fn list(conn: &Connection) -> Result<Vec<TranscriptionHistory>> {
        let mut stmt =
            conn.prepare("SELECT id, text, timestamp FROM transcriptions ORDER BY id DESC")?;
        let transcription_iter = stmt.query_map([], |row| {
            Ok(TranscriptionHistory::new(row.get(0)?, row.get(1)?))
        })?;

        let mut transcriptions = Vec::new();
        for transcription in transcription_iter {
            transcriptions.push(transcription?);
        }
        Ok(transcriptions)
    }

    pub fn clear(conn: &Connection) -> Result<()> {
        conn.execute("DELETE FROM transcriptions", [])?;
        Ok(())
    }
}
