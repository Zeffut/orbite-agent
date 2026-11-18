use std::fmt;
use std::path::Path;
use std::sync::Mutex;

use rusqlite::Connection;

use crate::Message;

/// Each entry brings the database from one version to the next. Entries are
/// only ever appended: an installation's database holds what its Owner said.
const MIGRATIONS: &[&str] = &[
    "CREATE TABLE conversation (
        id      INTEGER PRIMARY KEY AUTOINCREMENT,
        speaker TEXT NOT NULL CHECK (speaker IN ('owner', 'agent')),
        text    TEXT NOT NULL
    ) STRICT;",
    // Entries became richer than a speaker and a text once the Agent could
    // act: each is now kept whole, as JSON.
    "CREATE TABLE messages (
        id   INTEGER PRIMARY KEY AUTOINCREMENT,
        body TEXT NOT NULL
    ) STRICT;
    INSERT INTO messages (id, body)
        SELECT id, CASE speaker
            WHEN 'owner' THEN json_object('Owner', text)
            ELSE json_object('Agent', json_object('text', text, 'actions', json_array()))
        END
        FROM conversation ORDER BY id;
    DROP TABLE conversation;",
];

/// Where an installation keeps what must survive a restart.
pub struct Store {
    database: Mutex<Connection>,
}

/// Why the Store could not be opened, read or written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreError(String);

impl fmt::Display for StoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for StoreError {}

impl From<rusqlite::Error> for StoreError {
    fn from(error: rusqlite::Error) -> Self {
        Self(error.to_string())
    }
}

impl Store {
    /// Opens the database at `path`, creating it or bringing it up to date.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        Self::prepare(Connection::open(path)?)
    }

    /// A Store that lives only as long as the process.
    pub fn in_memory() -> Self {
        let connection = Connection::open_in_memory().expect("SQLite opens in memory");
        Self::prepare(connection).expect("migrations apply to an empty database")
    }

    fn prepare(mut connection: Connection) -> Result<Self, StoreError> {
        let version: u32 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        let version = version as usize;
        if version > MIGRATIONS.len() {
            return Err(StoreError(format!(
                "this database was written by a newer Orbite (version {version}, this one knows {})",
                MIGRATIONS.len()
            )));
        }
        for (index, migration) in MIGRATIONS.iter().enumerate().skip(version) {
            let transaction = connection.transaction()?;
            transaction.execute_batch(migration)?;
            transaction.pragma_update(None, "user_version", (index + 1) as u32)?;
            transaction.commit()?;
        }
        Ok(Self { database: Mutex::new(connection) })
    }

    /// The Conversation so far, oldest first.
    pub(crate) fn conversation(&self) -> Result<Vec<Message>, StoreError> {
        let database = self.database.lock().expect("the Store is not poisoned");
        let mut statement = database.prepare("SELECT body FROM messages ORDER BY id")?;
        let bodies = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        bodies
            .iter()
            .map(|body| serde_json::from_str(body).map_err(|error| StoreError(error.to_string())))
            .collect()
    }

    /// Adds `message` to the end of the Conversation.
    pub(crate) fn append(&self, message: &Message) -> Result<(), StoreError> {
        let body = serde_json::to_string(message).map_err(|error| StoreError(error.to_string()))?;
        self.database
            .lock()
            .expect("the Store is not poisoned")
            .execute("INSERT INTO messages (body) VALUES (?1)", [body])?;
        Ok(())
    }

    /// Takes back the latest message of the Conversation.
    pub(crate) fn forget_latest(&self) -> Result<(), StoreError> {
        self.database
            .lock()
            .expect("the Store is not poisoned")
            .execute("DELETE FROM messages WHERE id = (SELECT MAX(id) FROM messages)", ())?;
        Ok(())
    }
}
