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

    // The Conversation is copied whole even while the Agent is writing in
    // it, into a folder nobody else can look in.
    if home.join(DATABASE).exists() {
        let private = private_folder()?;
        let copy = private.join(DATABASE);
        let copied = Store::open(home.join(DATABASE))
            .and_then(|store| store.copy_to(&copy))
            .map_err(|error| io::Error::other(error.to_string()))
            .and_then(|()| packing.append_path_with_name(&copy, DATABASE));
        let _ = fs::remove_dir_all(&private);
        copied?;
    }
    // The archive itself may be written inside the folder it is made of.
    let itself = fs::canonicalize(archive).ok();
    for entry in fs::read_dir(home)? {
        let entry = entry?;
        let name = entry.file_name();
        let named = name.to_string_lossy();
        if named.starts_with(DATABASE)
            || named.ends_with(".writing")
            || fs::canonicalize(entry.path()).ok() == itself
        {
            continue;
        }
        pack(&mut packing, &entry.path(), Path::new(&name))?;
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
        // Nothing is moved unless everything can be: what is already there
        // under a name of the export would stop it half-way.
        let mut entries = Vec::new();
        for entry in fs::read_dir(&beside)? {
            let entry = entry?;
            let place = home.join(entry.file_name());
            match fs::symlink_metadata(&place) {
                Err(_) => {}
                // An empty folder, such as one the installer or a first start made, gives way.
                Ok(there) if there.is_dir() && fs::read_dir(&place)?.next().is_none() => {}
                Ok(_) => {
                    return Err(io::Error::other(format!(
                        "{} already holds {}: nothing was changed",
                        home.display(),
                        entry.file_name().to_string_lossy()
                    )));
                }
            }
            entries.push((entry.path(), place));
        }
        // Handed to whoever the folder belongs to while still beside it,
        // where nobody but this program can change anything.
        hand_over(&beside, home)?;
        for (from, place) in entries {
            if place.is_dir() {
                fs::remove_dir(&place)?;
            }
            fs::rename(from, place)?;
        }
        Ok(())
    })();
    let _ = fs::remove_dir_all(&beside);
    unpacked.map_err(|error| {
        io::Error::other(format!("{} could not be restored: {error}", archive.display()))
    })
}

/// Gives everything in `folder` to whoever `home` belongs to: a restore is
/// run as root on an installed Host, where the Agent is not root. A link is
/// given as a link: what it points at is never touched.
fn hand_over(folder: &Path, home: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, lchown};
        let owner = fs::metadata(home)?;
        let mut pending = vec![folder.to_path_buf()];
        while let Some(folder) = pending.pop() {
            for entry in fs::read_dir(&folder)? {
                let entry = entry?;
                let kept = fs::symlink_metadata(entry.path())?;
                if kept.uid() != owner.uid() || kept.gid() != owner.gid() {
                    lchown(entry.path(), Some(owner.uid()), Some(owner.gid()))?;
                }
                if kept.is_dir() {
                    pending.push(entry.path());
                }
            }
        }
    }
    Ok(())
}

/// A new folder that only this program's account can look in.
fn private_folder() -> io::Result<PathBuf> {
    use std::os::unix::fs::DirBuilderExt;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_nanos());
    let folder = std::env::temp_dir().join(format!("orbite-export-{}-{stamp}", std::process::id()));
    // Made, never taken over: a folder already there under that name is refused.
    fs::DirBuilder::new().mode(0o700).create(&folder)?;
    Ok(folder)
}

/// Adds `path` to the archive under `name`: files, folders and links as
/// they are; anything else a folder may hold (a socket, a pipe) is left out.
fn pack(
    packing: &mut tar::Builder<GzEncoder<fs::File>>,
    path: &Path,
    name: &Path,
) -> io::Result<()> {
    let kind = fs::symlink_metadata(path)?.file_type();
    if kind.is_dir() {
        packing.append_dir(name, path)?;
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            pack(packing, &entry.path(), &name.join(entry.file_name()))?;
        }
        Ok(())
    } else if kind.is_file() || kind.is_symlink() {
        packing.append_path_with_name(path, name)
    } else {
        Ok(())
    }
}
