//! Reusable ways of doing things, kept in the open Agent Skills format: one
//! folder per Skill holding a `SKILL.md` (a short header, then instructions)
//! and whatever other files the Skill needs. Skills written for other agents
//! work as they are.

use std::fs;
use std::io;
use std::path::PathBuf;

use crate::slug::slug;

pub(crate) struct Skills {
    folder: PathBuf,
}

/// A Skill as read from its folder.
pub(crate) struct Skill {
    pub(crate) name: String,
    /// What it is for and when to use it: all the model sees until it uses it.
    pub(crate) description: String,
    pub(crate) instructions: String,
    pub(crate) folder: PathBuf,
}

impl Skills {
    pub(crate) fn kept_in(folder: PathBuf) -> Self {
        Self { folder }
    }

    /// Every readable Skill, by name. Folders that are not Skills are left out.
    pub(crate) fn all(&self) -> Vec<Skill> {
        let Ok(folders) = fs::read_dir(&self.folder) else { return Vec::new() };
        let mut skills: Vec<Skill> = folders
            .filter_map(Result::ok)
            .filter_map(|folder| self.read(folder.file_name().to_str()?))
            .collect();
        skills.sort_by(|a, b| a.name.cmp(&b.name));
        skills
    }

    pub(crate) fn find(&self, name: &str) -> Option<Skill> {
        self.read(&slug(name).ok()?)
    }

    /// True when a Skill is already kept under `name`.
    pub(crate) fn has(&self, name: &str) -> bool {
        slug(name).is_ok_and(|name| self.folder.join(name).join("SKILL.md").exists())
    }

    /// Writes the Skill's `SKILL.md`, leaving its other files alone.
    pub(crate) fn save(
        &self,
        name: &str,
        description: &str,
        instructions: &str,
    ) -> io::Result<String> {
        let name = slug(name)?;
        let folder = self.folder.join(&name);
        fs::create_dir_all(&folder)?;
        // A JSON string is also a valid YAML one, whatever it contains.
        let description =
            serde_json::to_string(&one_line(description)).map_err(io::Error::other)?;
        let content = format!(
            "---\nname: {name}\ndescription: {description}\n---\n\n{}\n",
            instructions.trim()
        );
        fs::write(folder.join("SKILL.md"), content)?;
        Ok(name)
    }

    /// Deletes the Skill and its files; false when there is none.
    pub(crate) fn remove(&self, name: &str) -> io::Result<bool> {
        let name = slug(name)?;
        if !self.folder.join(&name).join("SKILL.md").exists() {
            return Ok(false);
        }
        fs::remove_dir_all(self.folder.join(name))?;
        Ok(true)
    }

    fn read(&self, name: &str) -> Option<Skill> {
        let folder = self.folder.join(name);
        let content = fs::read_to_string(folder.join("SKILL.md")).ok()?;
        let (header, instructions) = content.strip_prefix("---")?.split_once("\n---")?;
        Some(Skill {
            name: name.to_owned(),
            description: header_value(header, "description")?,
            instructions: instructions.trim().to_owned(),
            folder,
        })
    }
}

/// The value of `key` in a Skill's header. The header is YAML, of which
/// Skills use little: a value on its line, quoted or not, or folded over the
/// indented lines that follow.
fn header_value(header: &str, key: &str) -> Option<String> {
    let mut lines = header.lines();
    let first = lines.by_ref().find_map(|line| line.strip_prefix(key)?.strip_prefix(':'))?.trim();
    let value = if first.is_empty() || first.starts_with(['>', '|']) {
        let folded: Vec<&str> = lines
            .take_while(|line| line.starts_with([' ', '\t']) || line.is_empty())
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .collect();
        folded.join(" ")
    } else if first.starts_with('"') {
        serde_json::from_str(first).unwrap_or_else(|_| first.trim_matches('"').to_owned())
    } else {
        first.trim_matches('\'').to_owned()
    };
    (!value.is_empty()).then_some(value)
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}
