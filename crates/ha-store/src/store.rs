use std::path::Path;
use std::time::Duration;

use rusqlite::Connection;

use crate::key::StoreKey;
use crate::{migrations, StoreError};

/// An open, encrypted, migrated store.
///
/// One `Store` owns one connection. The SQLCipher key, foreign-key
/// enforcement, and busy timeout are all per-connection state in SQLite, so
/// every store must be opened through this type — a hand-rolled
/// `Connection` skips the key and silently runs without the schema's
/// constraints.
pub struct Store {
    conn: Connection,
}

impl Store {
    /// Open the encrypted database at `path`, creating it if needed, then
    /// apply the key, configure the connection, and run pending migrations.
    ///
    /// The key is supplied at runtime (environment / OS keystore hand-off)
    /// and is never read from or written to disk by this crate.
    pub fn open(path: impl AsRef<Path>, key: &StoreKey) -> Result<Self, StoreError> {
        Self::initialize(Connection::open(path)?, key)
    }

    /// Open an in-memory encrypted store — the workhorse for tests.
    pub fn open_in_memory(key: &StoreKey) -> Result<Self, StoreError> {
        Self::initialize(Connection::open_in_memory()?, key)
    }

    fn initialize(conn: Connection, key: &StoreKey) -> Result<Self, StoreError> {
        apply_key(&conn, key)?;
        configure(&conn)?;
        migrations::migrate(&conn)?;
        Ok(Self { conn })
    }

    /// Escape hatch for callers that need raw SQL (agents, the CLI demo).
    /// The connection already has the key applied, foreign keys enforced,
    /// and migrations run.
    pub fn conn(&mut self) -> &mut Connection {
        &mut self.conn
    }
}

/// SQLCipher requires the key before any other statement reads the file.
/// A wrong key only surfaces on the first real read, so probe
/// `sqlite_master` here to fail fast with a single, clear error.
fn apply_key(conn: &Connection, key: &StoreKey) -> Result<(), StoreError> {
    conn.pragma_update(None, "key", key.passphrase().to_owned())?;
    let probe = conn.query_row("SELECT count(*) FROM sqlite_master", [], |row| {
        row.get::<_, i64>(0)
    });
    probe.map_err(|err| {
        if is_not_a_database(&err) {
            StoreError::KeyRejected(err)
        } else {
            StoreError::Sqlite(err)
        }
    })?;
    Ok(())
}

fn is_not_a_database(err: &rusqlite::Error) -> bool {
    matches!(
        err,
        rusqlite::Error::SqliteFailure(ffi, _)
            if ffi.code == rusqlite::ErrorCode::NotADatabase
    )
}

fn configure(conn: &Connection) -> Result<(), StoreError> {
    conn.busy_timeout(Duration::from_secs(5))?;
    // Foreign-key enforcement is per-connection in SQLite: every client
    // must set it, or the schema's constraints silently run alone.
    conn.execute_batch("PRAGMA foreign_keys = ON;")?;
    Ok(())
}
