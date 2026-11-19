use std::fmt;
use std::path::Path;
use std::sync::Mutex;

use rusqlite::Connection;

use crate::routines::Routine;
use crate::{Message, Usage};

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
    // A long Conversation is summarised: the model is shown the summary and
    // only what came after it. Everything said is still kept, with its date,
    // so that it can be searched.
    "ALTER TABLE messages ADD COLUMN at INTEGER;
    CREATE TABLE settings (
        key   TEXT PRIMARY KEY,
        value TEXT NOT NULL
    ) STRICT;",
    "CREATE TABLE routines (
        name        TEXT PRIMARY KEY,
        instruction TEXT NOT NULL,
        cron        TEXT,
        next_run    INTEGER NOT NULL,
        paused      INTEGER NOT NULL DEFAULT 0,
        last_run    INTEGER,
        last_result TEXT
    ) STRICT;",
    "CREATE TABLE usage (
        day     TEXT NOT NULL,
        by      TEXT NOT NULL,
        model   TEXT NOT NULL,
        answers INTEGER NOT NULL DEFAULT 0,
        input   INTEGER NOT NULL DEFAULT 0,
        cached  INTEGER NOT NULL DEFAULT 0,
        output  INTEGER NOT NULL DEFAULT 0,
        PRIMARY KEY (day, by, model)
    ) STRICT;",
];

/// Where an installation keeps what must survive a restart.
pub struct Store {
    database: Mutex<Connection>,
}

/// One entry of the Conversation as kept: what was said, when, and the
/// identifier a place uses to ask for what came after it.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub id: i64,
    /// When it was said, in seconds since 1970.
    pub at: i64,
    pub message: Message,
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

    /// The part of the Conversation the model is shown, oldest first: what
    /// was said since the latest summary.
    pub(crate) fn conversation(&self) -> Result<Vec<Message>, StoreError> {
        Ok(self.shown()?.into_iter().map(|(_, message)| message).collect())
    }

    /// As [`Store::conversation`], each entry with its identifier.
    pub(crate) fn shown(&self) -> Result<Vec<(i64, Message)>, StoreError> {
        let database = self.database.lock().expect("the Store is not poisoned");
        let mut statement = database.prepare(
            "SELECT id, body FROM messages
             WHERE id > COALESCE((SELECT CAST(value AS INTEGER) FROM settings
                                  WHERE key = 'summarised_through'), 0)
             ORDER BY id",
        )?;
        let rows = statement
            .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        rows.into_iter().map(|(id, body)| Ok((id, read(&body)?))).collect()
    }

    /// Writes a whole copy of the Store to `file`, which must not exist,
    /// even while another program is writing in the Store.
    pub fn copy_to(&self, file: impl AsRef<Path>) -> Result<(), StoreError> {
        let file = file.as_ref().to_string_lossy().into_owned();
        self.database.lock().unwrap().execute("VACUUM INTO ?1", [file])?;
        Ok(())
    }

    /// The Conversation as said, for a place that shows it: the entries
    /// after `after`, oldest first, or without it the latest ones; never more
    /// than `limit`. Everything is there, summarised for the model or not.
    pub fn history(&self, after: Option<i64>, limit: usize) -> Result<Vec<Entry>, StoreError> {
        let database = self.database.lock().expect("the Store is not poisoned");
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let mut statement = database.prepare(match after {
            Some(_) => {
                "SELECT id, COALESCE(at, 0), body FROM messages WHERE id > ?1 ORDER BY id LIMIT ?2"
            }
            None => {
                "SELECT id, COALESCE(at, 0), body FROM
                     (SELECT * FROM messages WHERE id > ?1 ORDER BY id DESC LIMIT ?2) ORDER BY id"
            }
        })?;
        let rows = statement
            .query_map((after.unwrap_or(0), limit), |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?, row.get::<_, String>(2)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        rows.into_iter().map(|(id, at, body)| Ok(Entry { id, at, message: read(&body)? })).collect()
    }

    /// The summary of everything the model is no longer shown.
    pub(crate) fn summary(&self) -> Result<Option<String>, StoreError> {
        let database = self.database.lock().expect("the Store is not poisoned");
        let mut statement = database.prepare("SELECT value FROM settings WHERE key = 'summary'")?;
        let mut rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        Ok(rows.next().transpose()?)
    }

    /// Stops showing the model the entries up to `through`. `summary`, when
    /// given, replaces the summary; otherwise the earlier one is kept.
    pub(crate) fn summarise_through(
        &self,
        through: i64,
        summary: Option<&str>,
    ) -> Result<(), StoreError> {
        let mut database = self.database.lock().expect("the Store is not poisoned");
        let transaction = database.transaction()?;
        let set = "INSERT INTO settings (key, value) VALUES (?1, ?2)
                   ON CONFLICT (key) DO UPDATE SET value = excluded.value";
        transaction.execute(set, ("summarised_through", through.to_string()))?;
        if let Some(summary) = summary {
            transaction.execute(set, ("summary", summary))?;
        }
        transaction.commit()?;
        Ok(())
    }

    /// What was said that matches `query`, most relevant first, each with its
    /// date. Everything ever said is searched, summarised or not.
    pub(crate) fn search(&self, query: &str) -> Result<Vec<(String, Message)>, StoreError> {
        let words: Vec<String> = query
            .split(|c: char| !c.is_alphanumeric())
            .filter(|word| word.chars().count() >= 3)
            .map(str::to_lowercase)
            .collect();
        if words.is_empty() {
            return Ok(Vec::new());
        }
        let database = self.database.lock().expect("the Store is not poisoned");
        let mut statement = database.prepare(
            "SELECT id, COALESCE(date(at, 'unixepoch'), ''), body FROM messages ORDER BY id DESC",
        )?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let mut found = Vec::new();
        for (id, date, body) in rows {
            let message = read(&body)?;
            let text = match &message {
                Message::Owner(text) => text,
                Message::Agent { text, .. } if !text.is_empty() => text,
                _ => continue,
            };
            let text = text.to_lowercase();
            let matched = words.iter().filter(|word| text.contains(word.as_str())).count();
            if matched > 0 {
                found.push((matched, id, date, message));
            }
        }
        found.sort_by_key(|entry| std::cmp::Reverse((entry.0, entry.1)));
        Ok(found.into_iter().take(8).map(|(_, _, date, message)| (date, message)).collect())
    }

    /// Adds what one answer cost to the count of `day` (written YYYY-MM-DD).
    pub(crate) fn record_usage(&self, day: &str, usage: &Usage) -> Result<(), StoreError> {
        let count = |tokens: u64| i64::try_from(tokens).unwrap_or(i64::MAX);
        self.database.lock().expect("the Store is not poisoned").execute(
            "INSERT INTO usage (day, by, model, answers, input, cached, output)
             VALUES (?1, ?2, ?3, 1, ?4, ?5, ?6)
             ON CONFLICT (day, by, model) DO UPDATE SET
                 answers = answers + 1, input = input + excluded.input,
                 cached = cached + excluded.cached, output = output + excluded.output",
            (
                day,
                &usage.by,
                &usage.model,
                count(usage.input),
                count(usage.cached),
                count(usage.output),
            ),
        )?;
        Ok(())
    }

    /// What was used on each day from `since` on, as (day, answers, tokens),
    /// the latest day first.
    /// The tokens each Provider read and wrote on `day`, by Provider name.
    pub fn tokens_used_on(&self, day: &str) -> Result<Vec<(String, u64)>, StoreError> {
        let mut totals: Vec<(String, u64)> = Vec::new();
        for (on, _, usage) in self.usage_since(day)? {
            if on != day {
                continue;
            }
            let tokens = usage.input + usage.output;
            match totals.iter_mut().find(|(by, _)| *by == usage.by) {
                Some((_, total)) => *total += tokens,
                None => totals.push((usage.by, tokens)),
            }
        }
        Ok(totals)
    }

    pub(crate) fn usage_since(&self, since: &str) -> Result<Vec<(String, u64, Usage)>, StoreError> {
        let database = self.database.lock().expect("the Store is not poisoned");
        let mut statement = database.prepare(
            "SELECT day, by, model, answers, input, cached, output FROM usage
             WHERE day >= ?1 ORDER BY day DESC, by, model",
        )?;
        let count = |tokens: i64| u64::try_from(tokens).unwrap_or(0);
        let used = statement
            .query_map([since], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    count(row.get(3)?),
                    Usage {
                        by: row.get(1)?,
                        model: row.get(2)?,
                        input: count(row.get(4)?),
                        cached: count(row.get(5)?),
                        output: count(row.get(6)?),
                    },
                ))
            })?
            .collect::<Result<_, _>>()?;
        Ok(used)
    }

    /// The value kept under `key`, if any.
    pub(crate) fn setting(&self, key: &str) -> Result<Option<String>, StoreError> {
        let database = self.database.lock().expect("the Store is not poisoned");
        let mut statement = database.prepare("SELECT value FROM settings WHERE key = ?1")?;
        let mut rows = statement.query_map([key], |row| row.get::<_, String>(0))?;
        Ok(rows.next().transpose()?)
    }

    /// Keeps `value` under `key`, or removes it when there is none.
    pub(crate) fn set_setting(&self, key: &str, value: Option<&str>) -> Result<(), StoreError> {
        let database = self.database.lock().expect("the Store is not poisoned");
        match value {
            Some(value) => database.execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2)
                 ON CONFLICT (key) DO UPDATE SET value = excluded.value",
                (key, value),
            )?,
            None => database.execute("DELETE FROM settings WHERE key = ?1", [key])?,
        };
        Ok(())
    }

    /// Keeps `routine`, replacing the one of the same name.
    pub(crate) fn save_routine(&self, routine: &Routine) -> Result<(), StoreError> {
        self.database.lock().expect("the Store is not poisoned").execute(
            "INSERT INTO routines (name, instruction, cron, next_run, paused, last_run, last_result)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT (name) DO UPDATE SET
                 instruction = excluded.instruction, cron = excluded.cron,
                 next_run = excluded.next_run, paused = excluded.paused,
                 last_run = excluded.last_run, last_result = excluded.last_result",
            (
                &routine.name,
                &routine.instruction,
                &routine.cron,
                routine.next_run,
                routine.paused,
                routine.last_run,
                &routine.last_result,
            ),
        )?;
        Ok(())
    }

    /// Every Routine, by name.
    pub(crate) fn routines(&self) -> Result<Vec<Routine>, StoreError> {
        let database = self.database.lock().expect("the Store is not poisoned");
        let mut statement = database.prepare(
            "SELECT name, instruction, cron, next_run, paused, last_run, last_result
             FROM routines ORDER BY name",
        )?;
        let routines = statement
            .query_map([], |row| {
                Ok(Routine {
                    name: row.get(0)?,
                    instruction: row.get(1)?,
                    cron: row.get(2)?,
                    next_run: row.get(3)?,
                    paused: row.get(4)?,
                    last_run: row.get(5)?,
                    last_result: row.get(6)?,
                })
            })?
            .collect::<Result<_, _>>()?;
        Ok(routines)
    }

    /// Deletes the Routine called `name`; false when there is none.
    pub(crate) fn remove_routine(&self, name: &str) -> Result<bool, StoreError> {
        let removed = self
            .database
            .lock()
            .expect("the Store is not poisoned")
            .execute("DELETE FROM routines WHERE name = ?1", [name])?;
        Ok(removed > 0)
    }

    /// Erases the Conversation and its summary.
    pub(crate) fn reset(&self) -> Result<(), StoreError> {
        self.database.lock().expect("the Store is not poisoned").execute_batch(
            "DELETE FROM messages;
             DELETE FROM settings WHERE key IN ('summary', 'summarised_through');",
        )?;
        Ok(())
    }

    /// Adds `message` to the end of the Conversation.
    pub(crate) fn append(&self, message: &Message) -> Result<(), StoreError> {
        let body = serde_json::to_string(message).map_err(|error| StoreError(error.to_string()))?;
        self.database
            .lock()
            .expect("the Store is not poisoned")
            .execute("INSERT INTO messages (body, at) VALUES (?1, unixepoch())", [body])?;
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

fn read(body: &str) -> Result<Message, StoreError> {
    serde_json::from_str(body).map_err(|error| StoreError(error.to_string()))
}
