//! What the Agent knows that outlives every conversation: about its Owner,
//! and about the work it does for them. Kept as one plain file per entry,
//! so the Owner can read, correct and delete any of it without Orbite.

use std::fs;
use std::io;
use std::path::PathBuf;

use crate::slug::slug;

pub(crate) struct Memory {
    folder: PathBuf,
}

/// How much the Agent remembers at the most, in letters: all of it is read
/// at every turn, so it is kept to what matters, and the Agent makes room
/// itself when it is full.
pub(crate) const HOLDS: usize = 10_000;

impl Memory {
    pub(crate) fn kept_in(folder: PathBuf) -> Self {
        Self { folder }
    }

    /// Every entry as (name, text), by name. Read from the files each time,
    /// so that what the Owner changed by hand is what the Agent knows.
    pub(crate) fn entries(&self) -> Vec<(String, String)> {
        let Ok(files) = fs::read_dir(&self.folder) else { return Vec::new() };
        let mut entries: Vec<(String, String)> = files
            .filter_map(Result::ok)
            .filter_map(|file| {
                let path = file.path();
                if path.extension()? != "md" {
                    return None;
                }
                let name = path.file_stem()?.to_str()?.to_owned();
                let text = fs::read_to_string(&path).ok()?;
                Some((name, text.trim().to_owned()))
            })
            .collect();
        entries.sort();
        entries
    }

    /// How many letters the memory holds now.
    pub(crate) fn used(&self) -> usize {
        self.entries().iter().map(|(name, text)| name.chars().count() + text.chars().count()).sum()
    }

    /// Keeps `text` under `name`, replacing what was kept there.
    pub(crate) fn remember(&self, name: &str, text: &str) -> io::Result<String> {
        let name = slug(name)?;
        // Full, it takes nothing more before room is made: nothing kept is
        // dropped without the Agent choosing what.
        let others: usize = self
            .entries()
            .iter()
            .filter(|(kept, _)| *kept != name)
            .map(|(kept, text)| kept.chars().count() + text.chars().count())
            .sum();
        let whole = others + name.chars().count() + text.trim().chars().count();
        // (One that makes it smaller is always taken: a memory its Owner
        // filled by hand past its size can still be brought back to it.)
        if whole > HOLDS && whole > self.used() {
            return Err(io::Error::other(format!(
                "your memory would hold {whole} letters, and it holds {HOLDS} at the most. Make \
                 room first: shorten or merge entries, forget what no longer matters, then \
                 remember this again"
            )));
        }
        fs::create_dir_all(&self.folder)?;
        // What an entry said before is kept aside once: written over by
        // mistake, it is not lost for good.
        let entry = self.folder.join(format!("{name}.md"));
        if entry.exists() {
            let aside = self.folder.join(".replaced");
            let _ = fs::create_dir_all(&aside);
            let _ = fs::copy(&entry, aside.join(format!("{name}.md")));
        }
        fs::write(entry, format!("{}\n", text.trim()))?;
        Ok(name)
    }

    /// Deletes the entry called `name`; false when there is none.
    pub(crate) fn forget(&self, name: &str) -> io::Result<bool> {
        let name = slug(name)?;
        match fs::remove_file(self.folder.join(format!("{name}.md"))) {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error),
        }
    }
}
