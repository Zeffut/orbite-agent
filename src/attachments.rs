//! Files that travel with what the Owner says. A Channel saves each file it
//! receives and adds a line naming it to the Owner's words; that line is all
//! the Conversation keeps of it.

use std::path::{Path, PathBuf};

/// The largest image shown to a model.
const IMAGE_LIMIT: u64 = 5_000_000;

/// The line a Channel adds to the Owner's words for a file sent with them.
pub fn attached(file: &Path) -> String {
    format!("[attached: {}]", file.display())
}

/// The files named in `text` as sent with it.
pub(crate) fn attachments(text: &str) -> impl Iterator<Item = PathBuf> + '_ {
    text.lines().filter_map(|line| {
        let file = line.trim().strip_prefix("[attached: ")?.strip_suffix(']')?;
        Some(PathBuf::from(file))
    })
}

/// True when `file` is an image a model can be shown: of a kind models take,
/// present, and not too big.
pub(crate) fn is_showable_image(file: &Path) -> bool {
    image_type(file).is_some()
        && std::fs::metadata(file).is_ok_and(|about| about.is_file() && about.len() <= IMAGE_LIMIT)
}

/// The media type of `file` when it is an image, by its extension.
pub(crate) fn image_type(file: &Path) -> Option<&'static str> {
    match file.extension()?.to_str()?.to_lowercase().as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "webp" => Some("image/webp"),
        "gif" => Some("image/gif"),
        _ => None,
    }
}
