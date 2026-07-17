use crate::error::{DuplffError, Result};
use rusqlite::{params, Connection};
use std::path::Path;
use std::sync::Mutex;

/// Persistent hash cache backed by SQLite.
pub struct HashCache {
    conn: Mutex<Connection>,
}

impl HashCache {
    /// Open (or create) the default cache at ~/.cache/duplff/hashes.db
    pub fn open_default() -> Result<Self> {
        let cache_dir = dirs::cache_dir()
            .ok_or_else(|| DuplffError::CacheError("cannot determine cache directory".into()))?
            .join("duplff");
        std::fs::create_dir_all(&cache_dir)
            .map_err(|e| DuplffError::CacheError(e.to_string()))?;
        Self::open(&cache_dir.join("hashes.db"))
    }

    /// Open (or create) a cache at a specific path.
    pub fn open(path: &Path) -> Result<Self> {
        let conn =
            Connection::open(path).map_err(|e| DuplffError::CacheError(e.to_string()))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS hashes (
                path TEXT NOT NULL,
                size INTEGER NOT NULL,
                mtime INTEGER NOT NULL,
                partial_hash BLOB,
                full_hash BLOB,
                PRIMARY KEY (path, size, mtime)
            );
            PRAGMA journal_mode=WAL;
            PRAGMA synchronous=NORMAL;",
        )
        .map_err(|e| DuplffError::CacheError(e.to_string()))?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Look up a cached partial hash.
    pub fn get_partial(&self, path: &Path, size: u64, mtime: u64) -> Option<[u8; 32]> {
        let conn = self.conn.lock().ok()?;
        let mut stmt = conn
            .prepare(
                "SELECT partial_hash FROM hashes WHERE path = ?1 AND size = ?2 AND mtime = ?3",
            )
            .ok()?;
        stmt.query_row(params![path.to_str()?, size as i64, mtime as i64], |row| {
            let blob: Vec<u8> = row.get(0)?;
            Ok(blob)
        })
        .ok()
        .and_then(|b| b.try_into().ok())
    }

    /// Look up a cached full hash.
    pub fn get_full(&self, path: &Path, size: u64, mtime: u64) -> Option<[u8; 32]> {
        let conn = self.conn.lock().ok()?;
        let mut stmt = conn
            .prepare(
                "SELECT full_hash FROM hashes WHERE path = ?1 AND size = ?2 AND mtime = ?3",
            )
            .ok()?;
        stmt.query_row(params![path.to_str()?, size as i64, mtime as i64], |row| {
            let blob: Vec<u8> = row.get(0)?;
            Ok(blob)
        })
        .ok()
        .and_then(|b| b.try_into().ok())
    }

    /// Store a partial hash.
    pub fn put_partial(&self, path: &Path, size: u64, mtime: u64, hash: &[u8; 32]) {
        if let Ok(conn) = self.conn.lock() {
            let _ = conn.execute(
                "INSERT OR REPLACE INTO hashes (path, size, mtime, partial_hash) VALUES (?1, ?2, ?3, ?4)",
                params![path.to_str().unwrap_or(""), size as i64, mtime as i64, hash.as_slice()],
            );
        }
    }

    /// Store a full hash (updates existing row if partial was already stored).
    pub fn put_full(&self, path: &Path, size: u64, mtime: u64, hash: &[u8; 32]) {
        if let Ok(conn) = self.conn.lock() {
            let updated = conn
                .execute(
                    "UPDATE hashes SET full_hash = ?4 WHERE path = ?1 AND size = ?2 AND mtime = ?3",
                    params![
                        path.to_str().unwrap_or(""),
                        size as i64,
                        mtime as i64,
                        hash.as_slice()
                    ],
                )
                .unwrap_or(0);
            if updated == 0 {
                let _ = conn.execute(
                    "INSERT OR REPLACE INTO hashes (path, size, mtime, full_hash) VALUES (?1, ?2, ?3, ?4)",
                    params![path.to_str().unwrap_or(""), size as i64, mtime as i64, hash.as_slice()],
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn cache_round_trip_partial() {
        let dir = TempDir::new().unwrap();
        let cache = HashCache::open(&dir.path().join("test.db")).unwrap();
        let path = Path::new("/test/file.txt");
        let hash = [42u8; 32];
        cache.put_partial(path, 100, 12345, &hash);
        assert_eq!(cache.get_partial(path, 100, 12345), Some(hash));
    }

    #[test]
    fn cache_round_trip_full() {
        let dir = TempDir::new().unwrap();
        let cache = HashCache::open(&dir.path().join("test.db")).unwrap();
        let path = Path::new("/test/file.txt");
        let hash = [99u8; 32];
        cache.put_full(path, 200, 67890, &hash);
        assert_eq!(cache.get_full(path, 200, 67890), Some(hash));
    }

    #[test]
    fn cache_miss_returns_none() {
        let dir = TempDir::new().unwrap();
        let cache = HashCache::open(&dir.path().join("test.db")).unwrap();
        assert_eq!(cache.get_partial(Path::new("/nope"), 1, 1), None);
    }

    #[test]
    fn cache_invalidated_by_mtime_change() {
        let dir = TempDir::new().unwrap();
        let cache = HashCache::open(&dir.path().join("test.db")).unwrap();
        let path = Path::new("/test/file.txt");
        let hash = [42u8; 32];
        cache.put_partial(path, 100, 12345, &hash);
        assert_eq!(cache.get_partial(path, 100, 99999), None);
    }
}
