use rusqlite::{params, Connection};
use serde::Serialize;
use std::path::Path;

#[derive(Debug, Serialize, Clone)]
pub struct Transcription {
    pub id: i64,
    pub title: String,
    pub text: String,
    pub language: String,
    pub duration_secs: f64,
    pub created_at: String,
}

pub struct Store {
    conn: Connection,
}

impl Store {
    pub fn new(db_path: &Path) -> Result<Self, String> {
        let conn =
            Connection::open(db_path).map_err(|e| format!("Failed to open database: {}", e))?;

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS transcriptions (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                title TEXT NOT NULL,
                text TEXT NOT NULL,
                language TEXT NOT NULL DEFAULT 'pt',
                duration_secs REAL NOT NULL DEFAULT 0.0,
                created_at TEXT NOT NULL DEFAULT (datetime('now', 'localtime'))
            );",
        )
        .map_err(|e| format!("Failed to create table: {}", e))?;

        Ok(Self { conn })
    }

    pub fn save(
        &self,
        title: &str,
        text: &str,
        language: &str,
        duration_secs: f64,
    ) -> Result<i64, String> {
        self.conn
            .execute(
                "INSERT INTO transcriptions (title, text, language, duration_secs) VALUES (?1, ?2, ?3, ?4)",
                params![title, text, language, duration_secs],
            )
            .map_err(|e| format!("Failed to save transcription: {}", e))?;

        Ok(self.conn.last_insert_rowid())
    }

    pub fn list(&self) -> Result<Vec<Transcription>, String> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, title, text, language, duration_secs, created_at FROM transcriptions ORDER BY created_at DESC")
            .map_err(|e| format!("Failed to prepare query: {}", e))?;

        let rows = stmt
            .query_map([], |row| {
                Ok(Transcription {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    text: row.get(2)?,
                    language: row.get(3)?,
                    duration_secs: row.get(4)?,
                    created_at: row.get(5)?,
                })
            })
            .map_err(|e| format!("Failed to query: {}", e))?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("Failed to read row: {}", e))
    }

    pub fn get(&self, id: i64) -> Result<Transcription, String> {
        self.conn
            .query_row(
                "SELECT id, title, text, language, duration_secs, created_at FROM transcriptions WHERE id = ?1",
                params![id],
                |row| {
                    Ok(Transcription {
                        id: row.get(0)?,
                        title: row.get(1)?,
                        text: row.get(2)?,
                        language: row.get(3)?,
                        duration_secs: row.get(4)?,
                        created_at: row.get(5)?,
                    })
                },
            )
            .map_err(|e| format!("Transcription not found: {}", e))
    }

    pub fn delete(&self, id: i64) -> Result<(), String> {
        let affected = self
            .conn
            .execute("DELETE FROM transcriptions WHERE id = ?1", params![id])
            .map_err(|e| format!("Failed to delete: {}", e))?;

        if affected == 0 {
            return Err(format!("Transcription with id {} not found", id));
        }
        Ok(())
    }
}
