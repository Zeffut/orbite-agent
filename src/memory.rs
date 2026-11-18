//! What the Agent knows about its Owner, kept as one plain file per entry so
//! the Owner can read, correct and delete any of it without Orbite.

use std::fs;
use std::io;
use std::path::PathBuf;

use crate::slug::slug;

pub(crate) struct Memory {
    folder: PathBuf,
}

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

    /// Keeps `text` under `name`, replacing what was kept there.
    pub(crate) fn remember(&self, name: &str, text: &str) -> io::Result<String> {
        let name = slug(name)?;
        fs::create_dir_all(&self.folder)?;
        fs::write(self.folder.join(format!("{name}.md")), format!("{}\n", text.trim()))?;
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
