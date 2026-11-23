//! What setup is shown with and answered through: lines read and written
//! one after the other, wherever there is no screen to draw on.

use std::io::{BufRead, Write};

use crate::Language;

/// `text` without what a terminal would obey rather than show: what comes
/// from outside (a Provider's refusal, a model's name) is written as text.
pub(crate) fn clean(text: &str) -> String {
    text.chars()
        .map(|letter| if letter == '\t' { ' ' } else { letter })
        .filter(|letter| *letter == '\n' || !letter.is_control())
        .collect()
}

/// One of the answers offered to a question.
#[derive(Debug, Clone, Default)]
pub struct Choice {
    /// What it is called.
    pub label: String,
    /// A few words more, said quietly beside it.
    pub hint: String,
    /// What else may be typed to choose it, where answers are typed.
    pub also: Vec<String>,
}

impl Choice {
    pub fn of(label: impl Into<String>) -> Self {
        Self { label: label.into(), ..Self::default() }
    }

    pub fn hinted(mut self, hint: impl Into<String>) -> Self {
        self.hint = hint.into();
        self
    }

    pub fn also(mut self, typed: &[&str]) -> Self {
        self.also.extend(typed.iter().map(|typed| typed.to_lowercase()));
        self
    }
}

/// What was answered to a question with choices.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Picked {
    /// One of the choices, by its place.
    Choice(usize),
    /// Something of the Owner's own, where the question allowed it.
    Typed(String),
}

/// Whoever answers left before the end.
#[derive(Debug)]
pub struct Left;

/// How the Agent looks while setup goes on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mood {
    #[default]
    Waiting,
    Working,
    Glad,
}

/// What setup asks its questions through.
pub trait Front: Send {
    /// The language to speak from here on.
    fn speak(&mut self, _language: Language) {}

    /// Setup is at its step `at` of `of`.
    fn step(&mut self, _at: usize, _of: usize) {}

    /// Asks `question` with `choices`, the one at `suggested` first in
    /// line; `own` invites an answer that is none of them.
    fn choose(
        &mut self,
        question: &str,
        choices: &[Choice],
        suggested: usize,
        own: Option<&str>,
    ) -> Result<Picked, Left>;

    /// Asks for a line, which is never empty.
    fn ask(&mut self, question: &str) -> Result<String, Left>;

    /// Asks for what must not be shown as it is typed.
    fn ask_hidden(&mut self, question: &str) -> Result<String, Left>;

    /// Says something that stays until the next answer is given.
    fn say(&mut self, text: &str);

    /// Shows something to copy or open exactly as it is: a link, a code.
    fn show(&mut self, exactly: &str);

    /// Says what is being done, until something else is said.
    fn working(&mut self, text: &str);

    /// Says that something went well, and keeps it in view.
    fn good(&mut self, text: &str);

    /// Says that something did not work.
    fn wrong(&mut self, text: &str);

    /// The end: what to say, and the link that lets a first device in.
    fn finish(&mut self, text: &str, link: Option<&str>, after: &str);
}

/// Setup as lines: every question written, every answer a line typed.
pub struct Lines<R, W> {
    typed: R,
    shown: W,
    language: Language,
    hidden: Option<ReadHidden>,
}

/// Reads an answer without showing it as it is typed.
pub type ReadHidden = Box<dyn FnMut(&str) -> std::io::Result<String> + Send>;

impl<R: BufRead + Send, W: Write + Send> Lines<R, W> {
    pub fn new(typed: R, shown: W) -> Self {
        Self { typed, shown, language: Language::English, hidden: None }
    }

    /// Reads keys and tokens with `read`, which shows nothing of what is typed.
    pub fn reading_secrets_with(&mut self, read: ReadHidden) {
        self.hidden = Some(read);
    }

    fn write(&mut self, text: &str) {
        let _ = write!(self.shown, "{text}").and_then(|()| self.shown.flush());
    }

    fn line(&mut self) -> Result<String, Left> {
        let mut line = String::new();
        match self.typed.read_line(&mut line) {
            Ok(read) if read > 0 => Ok(line.trim().to_owned()),
            _ => Err(Left),
        }
    }
}

impl<R: BufRead + Send, W: Write + Send> Front for Lines<R, W> {
    fn speak(&mut self, language: Language) {
        self.language = language;
    }

    fn choose(
        &mut self,
        question: &str,
        choices: &[Choice],
        _suggested: usize,
        own: Option<&str>,
    ) -> Result<Picked, Left> {
        let mut asked = format!("\n{question}\n");
        for (place, choice) in choices.iter().enumerate() {
            let hint =
                if choice.hint.is_empty() { String::new() } else { format!(" ({})", choice.hint) };
            asked.push_str(&clean(&format!("  {}. {}{hint}\n", place + 1, choice.label)));
        }
        if let Some(own) = own {
            asked.push_str(&format!("  {own}\n"));
        }
        self.write(&asked);
        loop {
            self.write("> ");
            let answer = self.line()?;
            let lowered = answer.to_lowercase();
            let by_number = answer
                .parse::<usize>()
                .ok()
                .filter(|number| (1..=choices.len()).contains(number))
                .map(|number| number - 1);
            let by_name = choices.iter().position(|choice| {
                choice.label.to_lowercase() == lowered || choice.also.contains(&lowered)
            });
            match by_number.or(by_name) {
                Some(place) => return Ok(Picked::Choice(place)),
                None if own.is_some() && !answer.is_empty() => return Ok(Picked::Typed(answer)),
                None => {}
            }
        }
    }

    fn ask(&mut self, question: &str) -> Result<String, Left> {
        loop {
            self.write(&format!("{question} > "));
            let answer = self.line()?;
            if !answer.is_empty() {
                return Ok(answer);
            }
        }
    }

    fn ask_hidden(&mut self, question: &str) -> Result<String, Left> {
        let asked = format!("{question} {} > ", self.language.say("(not shown)", "(non affiché)"));
        let Some(read) = &mut self.hidden else {
            loop {
                self.write(&asked);
                let answer = self.line()?;
                if !answer.is_empty() {
                    return Ok(answer);
                }
            }
        };
        loop {
            let answer = read(&asked).map_err(|_| Left)?;
            if !answer.trim().is_empty() {
                return Ok(answer.trim().to_owned());
            }
        }
    }

    fn say(&mut self, text: &str) {
        self.write(&clean(&format!("\n{text}\n")));
    }

    fn show(&mut self, exactly: &str) {
        self.write(&clean(&format!("\n  {exactly}\n")));
    }

    fn working(&mut self, text: &str) {
        self.write(&clean(&format!("{text}\n")));
    }

    fn good(&mut self, text: &str) {
        self.write(&clean(&format!("{text} ✓\n")));
    }

    fn wrong(&mut self, text: &str) {
        self.write(&clean(&format!("{text}\n")));
    }

    fn finish(&mut self, text: &str, link: Option<&str>, after: &str) {
        let link = link.map(|link| format!("\n\n  {link}\n")).unwrap_or_default();
        self.write(&clean(&format!("\n{text}{link}\n{after}\n")));
    }
}
