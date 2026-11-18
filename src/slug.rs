use std::io;

/// The longest name an entry kept in a folder can have.
const NAME_LIMIT: usize = 60;

/// `name` as a file name: lowercase words joined by dashes. Nothing of a path
/// survives, so whatever is kept under it stays inside its folder.
pub(crate) fn slug(name: &str) -> io::Result<String> {
    let mut slug = String::new();
    for character in name.trim().chars().flat_map(char::to_lowercase) {
        if character.is_alphanumeric() {
            slug.push(character);
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug: String = slug.trim_end_matches('-').chars().take(NAME_LIMIT).collect();
    if slug.is_empty() {
        return Err(io::Error::other("a name must contain letters or digits"));
    }
    Ok(slug)
}
