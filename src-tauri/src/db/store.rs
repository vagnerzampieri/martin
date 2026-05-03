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
    pub status: String,
}

#[derive(Debug, Serialize, Clone)]
pub struct PendingRecording {
    pub id: i64,
    pub file_path: String,
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

        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(|e| format!("Failed to set WAL mode: {}", e))?;
        conn.pragma_update(None, "synchronous", "NORMAL")
            .map_err(|e| format!("Failed to set synchronous mode: {}", e))?;

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

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS pending_recordings (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                file_path TEXT NOT NULL,
                duration_secs REAL NOT NULL DEFAULT 0.0,
                created_at TEXT NOT NULL DEFAULT (datetime('now', 'localtime'))
            );",
        )
        .map_err(|e| format!("Failed to create pending_recordings table: {}", e))?;

        // Migration: add `status` column if missing. Idempotent — older databases
        // (created before this column existed) gain it on next launch with
        // existing rows backfilled to 'complete' via the column DEFAULT.
        let migration_result = conn.execute(
            "ALTER TABLE transcriptions ADD COLUMN status TEXT NOT NULL DEFAULT 'complete'",
            [],
        );
        match migration_result {
            Ok(_) => {}
            Err(rusqlite::Error::SqliteFailure(_, Some(msg)))
                if msg.contains("duplicate column name") => {}
            Err(e) => return Err(format!("Failed to add status column: {}", e)),
        }

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

    pub fn insert_partial(&self, title: &str, language: &str) -> Result<i64, String> {
        self.conn
            .execute(
                "INSERT INTO transcriptions (title, text, language, duration_secs, status) VALUES (?1, '', ?2, 0.0, 'partial')",
                params![title, language],
            )
            .map_err(|e| format!("Failed to insert partial transcription: {}", e))?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn update_text(&self, id: i64, text: &str, duration_secs: f64) -> Result<(), String> {
        let affected = self
            .conn
            .execute(
                "UPDATE transcriptions SET text = ?1, duration_secs = ?2 WHERE id = ?3",
                params![text, duration_secs, id],
            )
            .map_err(|e| format!("Failed to update text: {}", e))?;
        if affected == 0 {
            return Err(format!("Transcription with id {} not found", id));
        }
        Ok(())
    }

    #[allow(dead_code)]
    pub fn mark_complete(&self, id: i64) -> Result<(), String> {
        let affected = self
            .conn
            .execute(
                "UPDATE transcriptions SET status = 'complete' WHERE id = ?1",
                params![id],
            )
            .map_err(|e| format!("Failed to mark complete: {}", e))?;
        if affected == 0 {
            return Err(format!("Transcription with id {} not found", id));
        }
        Ok(())
    }

    /// Removes partial rows that have no text and no duration — these can only
    /// come from a force-kill that happened before the first segment callback
    /// fired. Returns the number of rows deleted.
    pub fn delete_empty_partials(&self) -> Result<usize, String> {
        let affected = self
            .conn
            .execute(
                "DELETE FROM transcriptions WHERE status = 'partial' AND text = '' AND duration_secs = 0.0",
                [],
            )
            .map_err(|e| format!("Failed to sweep empty partials: {}", e))?;
        Ok(affected)
    }

    pub fn list(&self) -> Result<Vec<Transcription>, String> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, title, text, language, duration_secs, created_at, summary, status FROM transcriptions ORDER BY created_at DESC")
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
                    status: row.get(7)?,
                })
            })
            .map_err(|e| format!("Failed to query: {}", e))?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("Failed to read row: {}", e))
    }

    pub fn get(&self, id: i64) -> Result<Transcription, String> {
        self.conn
            .query_row(
                "SELECT id, title, text, language, duration_secs, created_at, summary, status FROM transcriptions WHERE id = ?1",
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
                        status: row.get(7)?,
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

    pub fn save_pending(&self, file_path: &str, duration_secs: f64) -> Result<i64, String> {
        self.conn
            .execute(
                "INSERT INTO pending_recordings (file_path, duration_secs) VALUES (?1, ?2)",
                params![file_path, duration_secs],
            )
            .map_err(|e| format!("Failed to save pending recording: {}", e))?;

        Ok(self.conn.last_insert_rowid())
    }

    pub fn get_pending(&self, id: i64) -> Result<PendingRecording, String> {
        self.conn
            .query_row(
                "SELECT id, file_path, duration_secs, created_at FROM pending_recordings WHERE id = ?1",
                params![id],
                |row| {
                    Ok(PendingRecording {
                        id: row.get(0)?,
                        file_path: row.get(1)?,
                        duration_secs: row.get(2)?,
                        created_at: row.get(3)?,
                    })
                },
            )
            .map_err(|e| format!("Pending recording not found: {}", e))
    }

    pub fn list_pending(&self) -> Result<Vec<PendingRecording>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, file_path, duration_secs, created_at FROM pending_recordings ORDER BY created_at DESC",
            )
            .map_err(|e| format!("Failed to prepare query: {}", e))?;

        let rows = stmt
            .query_map([], |row| {
                Ok(PendingRecording {
                    id: row.get(0)?,
                    file_path: row.get(1)?,
                    duration_secs: row.get(2)?,
                    created_at: row.get(3)?,
                })
            })
            .map_err(|e| format!("Failed to query: {}", e))?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("Failed to read row: {}", e))
    }

    pub fn delete_pending(&self, id: i64) -> Result<(), String> {
        let affected = self
            .conn
            .execute("DELETE FROM pending_recordings WHERE id = ?1", params![id])
            .map_err(|e| format!("Failed to delete pending recording: {}", e))?;

        if affected == 0 {
            return Err(format!("Pending recording with id {} not found", id));
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

    #[test]
    fn save_pending_inserts_and_returns_valid_id() {
        let (store, _temp_file) = create_temp_store();

        let id = store
            .save_pending("/tmp/recording_123.wav", 120.5)
            .expect("Failed to save pending");

        assert!(id > 0);
    }

    #[test]
    fn get_pending_retrieves_saved_record() {
        let (store, _temp_file) = create_temp_store();

        let id = store
            .save_pending("/tmp/rec.wav", 60.0)
            .expect("Failed to save");

        let pending = store.get_pending(id).expect("Failed to get");

        assert_eq!(pending.id, id);
        assert_eq!(pending.file_path, "/tmp/rec.wav");
        assert_eq!(pending.duration_secs, 60.0);
        assert!(!pending.created_at.is_empty());
    }

    #[test]
    fn list_pending_returns_records_ordered_by_created_at_desc() {
        let (store, _temp_file) = create_temp_store();

        store.conn.execute(
            "INSERT INTO pending_recordings (file_path, duration_secs, created_at) VALUES (?1, ?2, ?3)",
            params!["/tmp/first.wav", 10.0, "2025-01-01 10:00:00"],
        ).expect("Failed to insert");
        store.conn.execute(
            "INSERT INTO pending_recordings (file_path, duration_secs, created_at) VALUES (?1, ?2, ?3)",
            params!["/tmp/second.wav", 20.0, "2025-01-01 11:00:00"],
        ).expect("Failed to insert");

        let records = store.list_pending().expect("Failed to list");

        assert_eq!(records.len(), 2);
        assert_eq!(records[0].file_path, "/tmp/second.wav");
        assert_eq!(records[1].file_path, "/tmp/first.wav");
    }

    #[test]
    fn list_pending_returns_empty_when_no_records() {
        let (store, _temp_file) = create_temp_store();

        let records = store.list_pending().expect("Failed to list");

        assert!(records.is_empty());
    }

    #[test]
    fn delete_pending_removes_record() {
        let (store, _temp_file) = create_temp_store();

        let id = store
            .save_pending("/tmp/rec.wav", 30.0)
            .expect("Failed to save");

        store.delete_pending(id).expect("Failed to delete");

        let result = store.get_pending(id);
        assert!(result.is_err());
    }

    #[test]
    fn delete_pending_returns_error_for_nonexistent_id() {
        let (store, _temp_file) = create_temp_store();

        let result = store.delete_pending(999);

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found"));
    }

    #[test]
    fn new_runs_migration_idempotently() {
        let temp_file = NamedTempFile::new().expect("temp file");
        let path = temp_file.path().to_path_buf();

        let _ = Store::new(&path).expect("first open");

        let store = Store::new(&path).expect("second open");

        let id = store.save("t", "x", "pt", 1.0).expect("save");
        let row: String = store
            .conn
            .query_row(
                "SELECT status FROM transcriptions WHERE id = ?1",
                rusqlite::params![id],
                |r| r.get(0),
            )
            .expect("query");
        assert_eq!(row, "complete");
    }

    #[test]
    fn get_returns_status_field() {
        let (store, _temp_file) = create_temp_store();
        let id = store.save("t", "txt", "pt", 1.0).expect("save");
        let t = store.get(id).expect("get");
        assert_eq!(t.status, "complete");
    }

    #[test]
    fn list_returns_status_field() {
        let (store, _temp_file) = create_temp_store();
        store.save("t1", "x", "pt", 1.0).expect("save");
        let rows = store.list().expect("list");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].status, "complete");
    }

    #[test]
    fn insert_partial_creates_row_with_status_partial() {
        let (store, _temp_file) = create_temp_store();
        let id = store
            .insert_partial("Dictation", "pt")
            .expect("insert_partial");
        let t = store.get(id).expect("get");
        assert_eq!(t.title, "Dictation");
        assert_eq!(t.text, "");
        assert_eq!(t.language, "pt");
        assert_eq!(t.duration_secs, 0.0);
        assert_eq!(t.status, "partial");
    }

    #[test]
    fn update_text_overwrites_text_and_duration() {
        let (store, _temp_file) = create_temp_store();
        let id = store.insert_partial("t", "pt").expect("insert");

        store.update_text(id, "first chunk", 5.0).expect("update");
        let row = store.get(id).expect("get");
        assert_eq!(row.text, "first chunk");
        assert_eq!(row.duration_secs, 5.0);
        assert_eq!(row.status, "partial");

        store
            .update_text(id, "first chunk and more", 12.0)
            .expect("update");
        let row = store.get(id).expect("get");
        assert_eq!(row.text, "first chunk and more");
        assert_eq!(row.duration_secs, 12.0);
    }

    #[test]
    fn update_text_returns_err_for_missing_id() {
        let (store, _temp_file) = create_temp_store();
        let err = store.update_text(999, "x", 1.0).unwrap_err();
        assert!(err.contains("not found"));
    }

    #[test]
    fn mark_complete_flips_status() {
        let (store, _temp_file) = create_temp_store();
        let id = store.insert_partial("t", "pt").expect("insert");
        store.mark_complete(id).expect("mark");
        let row = store.get(id).expect("get");
        assert_eq!(row.status, "complete");
    }

    #[test]
    fn delete_empty_partials_removes_only_empty_partials() {
        let (store, _temp_file) = create_temp_store();

        let kept_complete = store.save("c", "x", "pt", 1.0).expect("save");
        let kept_partial_with_text = store.insert_partial("p1", "pt").expect("insert");
        store
            .update_text(kept_partial_with_text, "some text", 5.0)
            .expect("update");
        let removed_id = store.insert_partial("ghost", "pt").expect("insert");

        let removed = store.delete_empty_partials().expect("sweep");
        assert_eq!(removed, 1, "only the empty partial should be deleted");

        assert!(store.get(kept_complete).is_ok());
        assert!(store.get(kept_partial_with_text).is_ok());
        assert!(store.get(removed_id).is_err());
    }

    #[test]
    fn migration_backfills_existing_rows_with_complete_status() {
        let temp_file = NamedTempFile::new().expect("temp file");
        let path = temp_file.path().to_path_buf();

        {
            let conn = rusqlite::Connection::open(&path).expect("open raw");
            conn.execute(
                "CREATE TABLE transcriptions (
                    id INTEGER PRIMARY KEY,
                    title TEXT NOT NULL,
                    text TEXT NOT NULL,
                    language TEXT NOT NULL,
                    duration_secs REAL NOT NULL,
                    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                    summary TEXT
                )",
                [],
            )
            .expect("create v0.1.0 schema");
            conn.execute(
                "INSERT INTO transcriptions (title, text, language, duration_secs) VALUES ('old1', 'a', 'pt', 1.0), ('old2', 'b', 'en', 2.0)",
                [],
            )
            .expect("seed");
        }

        let store = Store::new(&path).expect("upgrade open");
        let rows: Vec<String> = store
            .conn
            .prepare("SELECT status FROM transcriptions ORDER BY id")
            .expect("prepare")
            .query_map([], |r| r.get::<_, String>(0))
            .expect("query_map")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect rows");
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|s| s == "complete"));
    }
}
