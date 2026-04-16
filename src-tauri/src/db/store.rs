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
    pub summary: Option<String>,
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
                created_at TEXT NOT NULL DEFAULT (datetime('now', 'localtime')),
                summary TEXT
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
            .prepare("SELECT id, title, text, language, duration_secs, created_at, summary FROM transcriptions ORDER BY created_at DESC")
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
                    summary: row.get(6)?,
                })
            })
            .map_err(|e| format!("Failed to query: {}", e))?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("Failed to read row: {}", e))
    }

    pub fn get(&self, id: i64) -> Result<Transcription, String> {
        self.conn
            .query_row(
                "SELECT id, title, text, language, duration_secs, created_at, summary FROM transcriptions WHERE id = ?1",
                params![id],
                |row| {
                    Ok(Transcription {
                        id: row.get(0)?,
                        title: row.get(1)?,
                        text: row.get(2)?,
                        language: row.get(3)?,
                        duration_secs: row.get(4)?,
                        created_at: row.get(5)?,
                        summary: row.get(6)?,
                    })
                },
            )
            .map_err(|e| format!("Transcription not found: {}", e))
    }

    pub fn save_summary(&self, id: i64, summary: &str) -> Result<(), String> {
        let affected = self
            .conn
            .execute(
                "UPDATE transcriptions SET summary = ?1 WHERE id = ?2",
                params![summary, id],
            )
            .map_err(|e| format!("Failed to save summary: {}", e))?;

        if affected == 0 {
            return Err(format!("Transcription with id {} not found", id));
        }
        Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    fn create_temp_store() -> (Store, NamedTempFile) {
        let temp_file = NamedTempFile::new().expect("Failed to create temp file");
        let store = Store::new(temp_file.path()).expect("Failed to create store");
        (store, temp_file)
    }

    #[test]
    fn new_creates_table_successfully() {
        let (_, _temp_file) = create_temp_store();
    }

    #[test]
    fn save_inserts_record_and_returns_valid_id() {
        let (store, _temp_file) = create_temp_store();

        let id = store
            .save("Meeting Notes", "Some transcription text", "en", 120.5)
            .expect("Failed to save transcription");

        assert!(id > 0);
    }

    #[test]
    fn get_retrieves_saved_record_with_correct_fields() {
        let (store, _temp_file) = create_temp_store();

        let id = store
            .save("Meeting Notes", "Hello world", "en", 45.0)
            .expect("Failed to save");

        let transcription = store.get(id).expect("Failed to get transcription");

        assert_eq!(transcription.id, id);
        assert_eq!(transcription.title, "Meeting Notes");
        assert_eq!(transcription.text, "Hello world");
        assert_eq!(transcription.language, "en");
        assert_eq!(transcription.duration_secs, 45.0);
        assert!(!transcription.created_at.is_empty());
    }

    #[test]
    fn list_returns_records_ordered_by_created_at_desc() {
        let (store, _temp_file) = create_temp_store();

        // Insert with explicit timestamps to guarantee ordering
        store.conn.execute(
            "INSERT INTO transcriptions (title, text, language, duration_secs, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params!["First", "text 1", "en", 10.0, "2025-01-01 10:00:00"],
        ).expect("Failed to insert");
        store.conn.execute(
            "INSERT INTO transcriptions (title, text, language, duration_secs, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params!["Second", "text 2", "pt", 20.0, "2025-01-01 11:00:00"],
        ).expect("Failed to insert");
        store.conn.execute(
            "INSERT INTO transcriptions (title, text, language, duration_secs, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params!["Third", "text 3", "es", 30.0, "2025-01-01 12:00:00"],
        ).expect("Failed to insert");

        let records = store.list().expect("Failed to list");

        assert_eq!(records.len(), 3);
        assert_eq!(records[0].title, "Third");
        assert_eq!(records[1].title, "Second");
        assert_eq!(records[2].title, "First");
    }

    #[test]
    fn list_returns_empty_vec_when_no_records_exist() {
        let (store, _temp_file) = create_temp_store();

        let records = store.list().expect("Failed to list");

        assert!(records.is_empty());
    }

    #[test]
    fn get_returns_error_for_nonexistent_id() {
        let (store, _temp_file) = create_temp_store();

        let result = store.get(999);

        assert!(result.is_err());
    }

    #[test]
    fn delete_removes_the_record() {
        let (store, _temp_file) = create_temp_store();

        let id = store
            .save("To Delete", "some text", "en", 5.0)
            .expect("Failed to save");

        store.delete(id).expect("Failed to delete");

        let result = store.get(id);
        assert!(result.is_err());
    }

    #[test]
    fn delete_returns_error_for_nonexistent_id() {
        let (store, _temp_file) = create_temp_store();

        let result = store.delete(999);

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found"));
    }

    #[test]
    fn get_returns_none_summary_when_not_set() {
        let (store, _temp_file) = create_temp_store();

        let id = store
            .save("Meeting", "Hello world", "en", 45.0)
            .expect("Failed to save");

        let transcription = store.get(id).expect("Failed to get");

        assert!(transcription.summary.is_none());
    }

    #[test]
    fn save_summary_persists_and_get_retrieves_it() {
        let (store, _temp_file) = create_temp_store();

        let id = store
            .save("Meeting", "Long discussion about project", "en", 120.0)
            .expect("Failed to save");

        store
            .save_summary(id, "Summary of the meeting with key points")
            .expect("Failed to save summary");

        let transcription = store.get(id).expect("Failed to get");

        assert_eq!(
            transcription.summary,
            Some("Summary of the meeting with key points".to_string())
        );
    }

    #[test]
    fn save_summary_returns_error_for_nonexistent_id() {
        let (store, _temp_file) = create_temp_store();

        let result = store.save_summary(999, "some summary");

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found"));
    }

    #[test]
    fn list_includes_summary_field() {
        let (store, _temp_file) = create_temp_store();

        let id = store
            .save("Meeting", "text", "en", 10.0)
            .expect("Failed to save");
        store
            .save_summary(id, "A summary")
            .expect("Failed to save summary");
        store
            .save("No summary", "text 2", "pt", 20.0)
            .expect("Failed to save");

        let records = store.list().expect("Failed to list");

        let with_summary = records.iter().find(|r| r.title == "Meeting").unwrap();
        let without_summary = records.iter().find(|r| r.title == "No summary").unwrap();

        assert_eq!(with_summary.summary, Some("A summary".to_string()));
        assert!(without_summary.summary.is_none());
    }

    #[test]
    fn save_multiple_records_and_list_returns_all() {
        let (store, _temp_file) = create_temp_store();

        store
            .save("Alpha", "text a", "en", 10.0)
            .expect("Failed to save");
        store
            .save("Beta", "text b", "pt", 20.0)
            .expect("Failed to save");
        store
            .save("Gamma", "text c", "es", 30.0)
            .expect("Failed to save");
        store
            .save("Delta", "text d", "fr", 40.0)
            .expect("Failed to save");

        let records = store.list().expect("Failed to list");

        assert_eq!(records.len(), 4);

        let titles: Vec<&str> = records.iter().map(|r| r.title.as_str()).collect();
        assert!(titles.contains(&"Alpha"));
        assert!(titles.contains(&"Beta"));
        assert!(titles.contains(&"Gamma"));
        assert!(titles.contains(&"Delta"));
    }
}
