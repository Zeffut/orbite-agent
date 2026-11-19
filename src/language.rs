/// The language of Orbite's own words: what it says and shows itself, as
/// opposed to what the model writes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Language {
    #[default]
    English,
    French,
}

impl Language {
    /// The language of a two-letter code; English for any it does not speak.
    pub fn of(code: Option<&str>) -> Self {
        if code == Some("fr") { Self::French } else { Self::English }
    }

    /// `english` or `french`, whichever is spoken.
    pub fn say<'a>(self, english: &'a str, french: &'a str) -> &'a str {
        if self == Self::French { french } else { english }
    }
}
