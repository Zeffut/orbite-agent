//! Moving an installation to another Host: everything of the Agent in one
//! file, and that file put back in an empty folder.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;

use crate::Store;

/// The file that tells an export from any other archive.
const MARK: &str = "orbite-export.json";

/// The Conversation's own file, and what SQLite keeps beside it.
const DATABASE: &str = "orbite.db";

/// Writes everything kept in `home` to `archive`, a file only its owner can
/// read: it holds the secrets and the key that opens them.
pub fn export(home: &Path, archive: &Path) -> io::Result<()> {
    if !home.join(DATABASE).exists() && !home.join("config.json").exists() {
        return Err(io::Error::other(format!("there is no Agent in {}", home.display())));
    }
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(archive)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(archive, fs::Permissions::from_mode(0o600))?;
    }
    let mut packing = tar::Builder::new(GzEncoder::new(file, Compression::default()));
    packing.follow_symlinks(false);

    let mark = format!("{{\"orbite\":\"{}\"}}\n", env!("CARGO_PKG_VERSION"));
    let mut header = tar::Header::new_gnu();
    header.set_size(mark.len() as u64);
    header.set_mode(0o600);
    header.set_cksum();
    packing.append_data(&mut header, MARK, mark.as_bytes())?;

    // The Conversation is copied whole even while the Agent is writing in it.
    if home.join(DATABASE).exists() {
        let copy = archive.with_extension("db-copy");
        let _ = fs::remove_file(&copy);
        let copied = Store::open(home.join(DATABASE))
            .and_then(|store| store.copy_to(&copy))
            .map_err(|error| io::Error::other(error.to_string()))
            .and_then(|()| packing.append_path_with_name(&copy, DATABASE));
        let _ = fs::remove_file(&copy);
        copied?;
    }
    for entry in fs::read_dir(home)? {
        let entry = entry?;
        let name = entry.file_name();
        let named = name.to_string_lossy();
        if named.starts_with(DATABASE) || named.ends_with(".writing") || entry.path() == archive {
            continue;
        }
        if entry.file_type()?.is_dir() {
            packing.append_dir_all(&name, entry.path())?;
        } else {
            packing.append_path_with_name(entry.path(), &name)?;
        }
    }
    packing.into_inner()?.finish()?.sync_all()
}

/// Puts what `archive` holds in `home`, which must not hold an Agent yet.
pub fn restore(archive: &Path, home: &Path) -> io::Result<()> {
    if home.join(DATABASE).exists() || home.join("config.json").exists() {
        return Err(io::Error::other(format!(
            "there is already an Agent in {}: nothing was changed. Restore on a Host where Orbite was just \
             installed and not set up, or remove that Agent first.",
            home.display()
        )));
    }
    // Unpacked beside the folder first: a file that is not an export, or one
    // cut short, leaves the folder as it was.
    let beside: PathBuf = match home.file_name() {
        Some(name) => home.with_file_name(format!("{}.restoring", name.to_string_lossy())),
        None => return Err(io::Error::other("the folder to restore into has no name")),
    };
    let _ = fs::remove_dir_all(&beside);
    fs::create_dir_all(&beside)?;
    let unpacked = (|| {
        let mut packed = tar::Archive::new(GzDecoder::new(fs::File::open(archive)?));
        packed.set_preserve_permissions(true);
        packed.unpack(&beside)?;
        if !beside.join(MARK).is_file() {
            return Err(io::Error::other("this file is not an export of Orbite"));
        }
        fs::remove_file(beside.join(MARK))?;
        fs::create_dir_all(home)?;
        for entry in fs::read_dir(&beside)? {
            let entry = entry?;
            fs::rename(entry.path(), home.join(entry.file_name()))?;
        }
        Ok(())
    })();
    let _ = fs::remove_dir_all(&beside);
    unpacked.map_err(|error| {
        io::Error::other(format!("{} could not be restored: {error}", archive.display()))
    })?;
    hand_over(home)
}

/// Gives what was restored to whoever the folder belongs to: a restore is
/// run as root on an installed Host, where the Agent is not root.
fn hand_over(home: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, chown};
        let owner = fs::metadata(home)?;
        let mut pending = vec![home.to_path_buf()];
        while let Some(folder) = pending.pop() {
            for entry in fs::read_dir(&folder)? {
                let entry = entry?;
                let kept = entry.metadata()?;
                if kept.uid() != owner.uid() || kept.gid() != owner.gid() {
                    chown(entry.path(), Some(owner.uid()), Some(owner.gid()))?;
                }
                if kept.is_dir() {
                    pending.push(entry.path());
                }
            }
        }
    }
    Ok(())
}
