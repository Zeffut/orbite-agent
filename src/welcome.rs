//! Setup on a screen of its own: the mascot, one question at a time,
//! answers chosen with the arrows, and what is typed in secret never shown.
//! Drawn in the terminal's own colours, so that it reads on any of them.

use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::Language;
use crate::Look;
use crate::front::{Choice, Front, Left, Mood, Picked, clean};

const BOLD: &str = "\x1b[1m";
const QUIET: &str = "\x1b[2m";
const PLAIN: &str = "\x1b[0m";
const ACCENT: &str = "\x1b[38;5;75m";
const OK: &str = "\x1b[38;5;78m";
const WARN: &str = "\x1b[38;5;210m";

/// How often the screen is drawn again.
const TICK: Duration = Duration::from_millis(80);

/// One line of the screen: what is written, and how many columns it takes.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Row {
    text: String,
    width: usize,
}

impl Row {
    fn blank() -> Self {
        Self::default()
    }

    /// `text` with nothing but letters in it, `indent` columns in.
    fn of(indent: usize, text: &str) -> Self {
        Self::styled(indent, "", text)
    }

    fn styled(indent: usize, style: &str, text: &str) -> Self {
        Self {
            text: format!("{}{style}{text}{PLAIN}", " ".repeat(indent)),
            width: indent + text.chars().count(),
        }
    }

    /// Several pieces one after the other, each with its style.
    fn pieces(indent: usize, pieces: &[(&str, &str)]) -> Self {
        let mut row = Self { text: " ".repeat(indent), width: indent };
        for (style, text) in pieces {
            row.text.push_str(&format!("{style}{text}{PLAIN}"));
            row.width += text.chars().count();
        }
        row
    }
}

/// What is being asked.
#[derive(Debug, Clone, Default)]
enum Asking {
    #[default]
    Nothing,
    Working(String),
    Choices {
        question: String,
        choices: Vec<Choice>,
        at: usize,
        /// What invites an answer of the Owner's own, and what they typed of it.
        own: Option<(String, String)>,
    },
    Line {
        question: String,
        typed: String,
        hidden: bool,
    },
}

/// Something said before a question.
#[derive(Debug, Clone)]
enum Said {
    Text(String),
    Exactly(String),
}

/// Everything the screen shows.
#[derive(Debug, Clone, Default)]
pub struct View {
    step: (usize, usize),
    kept: Vec<String>,
    said: Vec<Said>,
    trouble: Option<String>,
    asking: Asking,
    mood: Mood,
    french: bool,
    /// How the Agent looks, once that is known or being chosen.
    look: Option<Look>,
    /// What the Agent is called, once it has a name.
    name: Option<String>,
}

/// Who is on the screen, as rows of text: Orbite's planet with its ring,
/// a moon going round it while it works; or, once the Agent has a look of
/// its own, the Agent, which sways while it works. `tick` is how far along
/// time is.
pub fn mascot(look: Option<Look>, mood: Mood, tick: u64) -> Vec<Row> {
    // Drawn in quarters of a letter's cell, each half a column wide and
    // half a line high: a line being twice as tall as a column is wide,
    // lengths are counted in columns.
    const COLUMNS: usize = 27;
    const LINES: usize = 8;
    let (cx, cy) = (COLUMNS as f32 / 2.0, LINES as f32);
    let (sin, cos) = (-24.0_f32).to_radians().sin_cos();
    let within = |u: f32, v: f32, rx: f32, ry: f32| (u / rx).powi(2) + (v / ry).powi(2) <= 1.0;
    // The eyes: two tall ones, shut for an instant now and then, or two
    // arches when it is glad.
    let shut = tick % 55 < 2;
    let in_an_eye = |dx: f32, dy: f32| {
        [-2.05_f32, 2.05].iter().any(|eye| {
            let down = look.map_or(0.0, |look| look.eyes_down() * 7.4);
            let (ex, ey) = (dx - eye, dy + 1.2 - down);
            match mood {
                // An arch: a bar, and a leg down from each of its ends.
                Mood::Glad => {
                    let bar = (-1.6..-0.6).contains(&ey) && ex.abs() <= 1.3;
                    let legs = (-0.6..0.4).contains(&ey) && (0.7..=1.3).contains(&ex.abs());
                    bar || legs
                }
                _ if shut => ex.abs() <= 0.8 && ey.abs() <= 0.5,
                _ => within(ex, ey, 0.8, 2.0),
            }
        })
    };
    let lit = |x: f32, y: f32| {
        let (dx, dy) = (x - cx, y - cy);
        let (u, v) = (dx * cos + dy * sin, -dx * sin + dy * cos);
        let (ring, body) = match look {
            None => (
                within(u, v, 12.6, 3.7) && !within(u, v, 11.4, 2.5),
                dx * dx + dy * dy <= 7.4 * 7.4,
            ),
            // The Agent sways as it works, and swells a little: a round
            // one would not be seen to turn.
            Some(look) => {
                let sway = (tick as f32 * 0.22).sin();
                let (turned, swollen) = if mood == Mood::Working {
                    (0.3 * sway, 1.0 + 0.04 * sway)
                } else {
                    (0.0, 1.0)
                };
                (false, look.covers(dx / (7.4 * swollen), dy / (7.4 * swollen), turned))
            }
        };
        (ring || body) && !(body && in_an_eye(dx, dy))
    };
    // The moon, on a ring a little wider, hidden when it passes behind.
    let moon = (mood == Mood::Working && look.is_none()).then(|| {
        let angle = tick as f32 * 0.16;
        let (u, v) = (14.6 * angle.cos(), 5.4 * angle.sin());
        (cx + u * cos - v * sin, cy + u * sin + v * cos, angle.sin() < 0.0)
    });
    let on_moon = |x: f32, y: f32| {
        moon.is_some_and(|(mx, my, behind)| {
            let reached = (x - mx).powi(2) + (y - my).powi(2) <= 0.9;
            reached && !(behind && (x - cx).powi(2) + (y - cy).powi(2) <= 8.6 * 8.6)
        })
    };
    // Whether most of a quarter is covered, looked at in nine places.
    let covered = |x: f32, y: f32, of: &dyn Fn(f32, f32) -> bool| {
        let mut count = 0;
        for across in 0..3 {
            for down in 0..3 {
                let at = |part: i32| (part as f32 + 0.5) / 3.0;
                count += usize::from(of(x + at(across) * 0.5, y + at(down)));
            }
        }
        count >= 5
    };
    const QUARTERS: [char; 16] =
        [' ', '▘', '▝', '▀', '▖', '▌', '▞', '▛', '▗', '▚', '▐', '▜', '▄', '▙', '▟', '█'];
    let mut rows = Vec::new();
    for line in 0..LINES {
        let mut row = Row::blank();
        for column in 0..COLUMNS {
            let (mut quarters, mut of_the_moon) = (0, 0);
            for (place, (across, down)) in [(0, 0), (1, 0), (0, 1), (1, 1)].iter().enumerate() {
                let (x, y) = (column as f32 + *across as f32 * 0.5, (line * 2 + down) as f32);
                if covered(x, y, &on_moon) {
                    of_the_moon |= 1 << place;
                } else if covered(x, y, &lit) {
                    quarters |= 1 << place;
                }
            }
            // A cell has one colour: the moon's where the moon is.
            if of_the_moon != 0 {
                row.text.push_str(&format!("{ACCENT}{}{PLAIN}", QUARTERS[of_the_moon]));
            } else {
                row.text.push(QUARTERS[quarters]);
            }
            row.width += 1;
        }
        // The Agent, in its colour.
        if let Some(look) = look {
            row.text = format!("\x1b[38;5;{}m{}{PLAIN}", look.in_a_terminal(), row.text);
        }
        rows.push(row);
    }
    rows
}

/// `text` in lines of at most `wide` columns, broken between words.
fn wrapped(text: &str, wide: usize) -> Vec<String> {
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        let indent: String = paragraph.chars().take_while(|letter| *letter == ' ').collect();
        let mut line = indent.clone();
        for word in paragraph.split_whitespace() {
            let room = line.chars().count() + 1 + word.chars().count();
            if line.trim().is_empty() {
                line.push_str(word);
            } else if room <= wide {
                line.push(' ');
                line.push_str(word);
            } else {
                lines.push(std::mem::replace(&mut line, format!("{indent}{word}")));
            }
        }
        lines.push(line);
    }
    lines
}

/// The screen as rows, for a terminal of `columns` by `lines`: where it
/// would not all fit, the question is what stays, before the mascot and
/// before what went well so far.
pub fn frame(view: &View, columns: usize, lines: usize, tick: u64) -> Vec<Row> {
    let fits = |rows: &[Row]| {
        rows.iter().map(|row| row.width.max(1).div_ceil(columns.max(1))).sum::<usize>() <= lines
    };
    let whole = rows_of(view, columns, lines, tick, lines >= 22, true);
    if fits(&whole) {
        return whole;
    }
    let without_mascot = rows_of(view, columns, lines, tick, false, true);
    if fits(&without_mascot) {
        return without_mascot;
    }
    rows_of(view, columns, lines, tick, false, false)
}

fn rows_of(
    view: &View,
    columns: usize,
    lines: usize,
    tick: u64,
    with_mascot: bool,
    with_kept: bool,
) -> Vec<Row> {
    let wide = columns.saturating_sub(4).clamp(20, 62);
    let margin = columns.saturating_sub(wide) / 2;
    let centred = |row: Row| {
        let indent = columns.saturating_sub(row.width) / 2;
        Row { text: format!("{}{}", " ".repeat(indent), row.text), width: indent + row.width }
    };
    let say =
        |french: &'static str, english: &'static str| if view.french { french } else { english };
    let mut rows = vec![Row::blank()];
    if with_mascot {
        rows.extend(mascot(view.look, view.mood, tick).into_iter().map(centred));
        rows.push(Row::blank());
    }
    // Under Orbite's planet, Orbite's name; under the Agent, its own.
    let called = match (&view.look, &view.name) {
        (Some(_), Some(name)) => name.to_lowercase(),
        _ => "orbite".to_owned(),
    };
    let spaced: Vec<String> = called.chars().take(24).map(String::from).collect();
    rows.push(centred(Row::styled(0, BOLD, &spaced.join(" "))));
    if view.step.1 > 0 {
        let mut dots = Row::blank();
        for place in 1..=view.step.1 {
            let (style, dot) = if place <= view.step.0 { ("", "●") } else { (QUIET, "○") };
            dots.text.push_str(&format!("{style}{dot}{PLAIN}"));
            dots.width += 1;
            if place < view.step.1 {
                dots.text.push_str("  ");
                dots.width += 2;
            }
        }
        rows.push(centred(dots));
    }
    rows.push(Row::blank());
    // What went well so far, the latest two.
    let kept = &view.kept[view.kept.len().saturating_sub(if with_kept { 2 } else { 0 })..];
    for kept in kept {
        for (place, line) in wrapped(kept, wide - 2).iter().enumerate() {
            let mark = if place == 0 { "✓ " } else { "  " };
            rows.push(Row::pieces(margin, &[(OK, mark), (QUIET, line)]));
        }
    }
    if !kept.is_empty() {
        rows.push(Row::blank());
    }
    for said in &view.said {
        match said {
            Said::Text(text) => {
                rows.extend(
                    wrapped(text, wide).iter().map(|line| Row::styled(margin, QUIET, line)),
                );
            }
            // Never broken by us, so that it is copied whole.
            Said::Exactly(exactly) if exactly.chars().count() <= wide => {
                rows.push(Row::styled(margin, BOLD, exactly));
            }
            Said::Exactly(exactly) => rows.push(Row::of(0, exactly)),
        }
        rows.push(Row::blank());
    }
    if let Some(trouble) = &view.trouble {
        for (place, line) in wrapped(trouble, wide - 2).iter().enumerate() {
            let mark = if place == 0 { "✗ " } else { "  " };
            rows.push(Row::pieces(margin, &[(WARN, mark), (WARN, line)]));
        }
        rows.push(Row::blank());
    }
    let hint = match &view.asking {
        Asking::Nothing => "",
        Asking::Working(doing) => {
            const TURNING: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
            let turning = TURNING[(tick % 10) as usize];
            rows.push(Row::pieces(margin, &[(ACCENT, turning), ("", " "), (QUIET, doing)]));
            ""
        }
        Asking::Choices { question, choices, at, own } => {
            for line in wrapped(question, wide) {
                rows.push(Row::styled(margin, BOLD, &line));
            }
            rows.push(Row::blank());
            // As many as the screen has room for, around the one pointed at.
            let all = choices.len() + usize::from(own.is_some());
            let room = lines.saturating_sub(rows.len() + 3).max(3);
            let first = if all <= room { 0 } else { at.saturating_sub(room / 2).min(all - room) };
            for place in first..all.min(first + room) {
                let pointed = place == *at;
                let more_above = place == first && first > 0;
                let more_below = place + 1 == first + room && first + room < all;
                let mark = match (pointed, more_above, more_below) {
                    (true, _, _) => "❯ ",
                    (false, true, _) => "↑ ",
                    (false, _, true) => "↓ ",
                    _ => "  ",
                };
                let row = match (choices.get(place), own) {
                    (Some(choice), _) => {
                        let hint = if choice.hint.is_empty() {
                            String::new()
                        } else {
                            format!("  {}", choice.hint)
                        };
                        let style = if pointed { BOLD } else { "" };
                        let room = wide.saturating_sub(2 + choice.label.chars().count());
                        let hint: String = hint.chars().take(room).collect();
                        Row::pieces(
                            margin,
                            &[(ACCENT, mark), (style, &choice.label), (QUIET, &hint)],
                        )
                    }
                    (None, Some((invites, typed))) if pointed => Row::pieces(
                        margin,
                        &[
                            (ACCENT, mark),
                            (BOLD, typed),
                            ("", "▏"),
                            (QUIET, if typed.is_empty() { invites.as_str() } else { "" }),
                        ],
                    ),
                    (None, Some((invites, typed))) => {
                        let shown = if typed.is_empty() { invites } else { typed };
                        Row::pieces(margin, &[("", mark), (QUIET, shown)])
                    }
                    (None, None) => Row::blank(),
                };
                rows.push(row);
            }
            if own.is_some() {
                say(
                    "↑↓ choisir · ou écris · Entrée valider",
                    "↑↓ choose · or type · Enter to confirm",
                )
            } else {
                say("↑↓ choisir · Entrée valider", "↑↓ choose · Enter to confirm")
            }
        }
        Asking::Line { question, typed, hidden } => {
            for line in wrapped(question, wide) {
                rows.push(Row::styled(margin, BOLD, &line));
            }
            rows.push(Row::blank());
            if *hidden {
                let length = typed.chars().count();
                let dots = "•".repeat(length.min(28));
                let counted = match (length, view.french) {
                    (0, _) => String::new(),
                    (_, true) => format!("  {length} caractères"),
                    (_, false) => format!("  {length} characters"),
                };
                rows.push(Row::pieces(
                    margin,
                    &[(ACCENT, "❯ "), ("", &dots), ("", "▏"), (QUIET, &counted)],
                ));
                say(
                    "Colle ou écris : rien n'est affiché · Entrée valider",
                    "Paste or type: nothing is shown · Enter to confirm",
                )
            } else {
                // The end of what is typed, when it is longer than the line.
                let length = typed.chars().count();
                let shown: String = typed.chars().skip(length.saturating_sub(wide - 4)).collect();
                rows.push(Row::pieces(margin, &[(ACCENT, "❯ "), ("", &shown), ("", "▏")]));
                say("Entrée valider", "Enter to confirm")
            }
        }
    };
    if !hint.is_empty() {
        rows.push(Row::blank());
        rows.push(Row::styled(margin, QUIET, hint));
    }
    rows
}

/// A link as a square code a phone's camera reads, drawn dark on light
/// whatever the terminal's colours.
pub fn square_code(link: &str) -> Vec<Row> {
    let Ok(code) = qrcode::QrCode::new(link.as_bytes()) else { return Vec::new() };
    let side = code.width();
    let colours = code.to_colors();
    // A light border around it, which readers look for.
    let dark = |x: isize, y: isize| {
        (0..side as isize).contains(&x)
            && (0..side as isize).contains(&y)
            && colours[y as usize * side + x as usize] == qrcode::Color::Dark
    };
    let mut rows = Vec::new();
    let border = 2isize;
    let mut y = -border;
    while y < side as isize + border {
        let mut text = String::from("\x1b[38;5;232;48;5;255m");
        for x in -border..side as isize + border {
            text.push(match (dark(x, y), dark(x, y + 1)) {
                (true, true) => '█',
                (true, false) => '▀',
                (false, true) => '▄',
                (false, false) => ' ',
            });
        }
        text.push_str(PLAIN);
        rows.push(Row { text, width: side + 2 * border as usize });
        y += 2;
    }
    rows
}

/// A link as a square code, as a picture a page shows: dark on light,
/// with the border readers look for.
pub fn code_picture(link: &str) -> Option<String> {
    let code = qrcode::QrCode::new(link.as_bytes()).ok()?;
    let side = code.width();
    let colours = code.to_colors();
    let border = 3;
    let mut dark = String::new();
    for (place, colour) in colours.iter().enumerate() {
        if *colour == qrcode::Color::Dark {
            let (x, y) = (place % side + border, place / side + border);
            dark.push_str(&format!("M{x} {y}h1v1h-1z"));
        }
    }
    let whole = side + 2 * border;
    Some(format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {whole} {whole}\" \
         shape-rendering=\"crispEdges\"><rect width=\"{whole}\" height=\"{whole}\" \
         fill=\"#ffffff\"/><path d=\"{dark}\" fill=\"#000000\"/></svg>"
    ))
}

/// A link as a square code, as lines a terminal shows.
pub fn square_code_text(link: &str) -> String {
    square_code(link).iter().map(|row| format!("  {}\n", row.text)).collect()
}

/// What stays in the terminal once setup is over.
pub fn farewell(text: &str, link: Option<&str>, after: &str, columns: usize) -> Vec<Row> {
    let wide = columns.saturating_sub(4).clamp(20, 62);
    let mut rows = vec![Row::blank()];
    rows.extend(wrapped(text, wide).iter().map(|line| Row::styled(2, BOLD, line)));
    if let Some(link) = link {
        rows.push(Row::blank());
        rows.push(Row::styled(2, ACCENT, link));
        // A code to point a phone at, where a phone can follow the link.
        let of_this_machine = ["http://127.", "http://localhost", "http://[::1]"]
            .iter()
            .any(|local| link.starts_with(local));
        if !of_this_machine {
            rows.push(Row::blank());
            rows.extend(
                square_code(link)
                    .into_iter()
                    .map(|row| Row { text: format!("  {}", row.text), width: row.width + 2 }),
            );
        }
    }
    if !after.is_empty() {
        rows.push(Row::blank());
        rows.extend(wrapped(after, wide).iter().map(|line| Row::styled(2, QUIET, line)));
    }
    rows.push(Row::blank());
    rows
}

/// A key pressed.
#[derive(Debug, PartialEq, Eq)]
enum Key {
    Up,
    Down,
    Enter,
    Erase,
    EraseAll,
    Letter(char),
    Leave,
    Other,
}

/// The terminal taken over for the time of setup, and given back as it was.
pub struct Welcome {
    view: Arc<Mutex<View>>,
    over: Arc<AtomicBool>,
    drawing: Option<std::thread::JoinHandle<()>>,
    as_it_was: libc::termios,
    /// The looks the choices being asked would give, shown as each is
    /// pointed at.
    previews: Vec<Look>,
}

impl Welcome {
    /// True where there is a terminal that can show it.
    pub fn fits_here() -> bool {
        use std::io::IsTerminal;
        let terminal = std::env::var("TERM").unwrap_or_default();
        std::io::stdin().is_terminal()
            && std::io::stdout().is_terminal()
            && !matches!(terminal.as_str(), "" | "dumb" | "linux")
            && std::env::var_os("ORBITE_PLAIN").is_none()
    }

    /// Takes the terminal: its own screen, keys read as they are pressed.
    pub fn open() -> std::io::Result<Self> {
        // SAFETY: `termios` is plain data that `tcgetattr` fills in whole.
        let mut as_it_was: libc::termios = unsafe { std::mem::zeroed() };
        // SAFETY: descriptor 0 is this process's own input.
        if unsafe { libc::tcgetattr(0, &mut as_it_was) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        let mut raw = as_it_was;
        // Ctrl-C still stops it, at any moment: what is stopped then gives
        // the terminal back before it goes (`give_back`).
        raw.c_lflag &= !(libc::ICANON | libc::ECHO | libc::IEXTEN);
        raw.c_iflag &= !(libc::IXON | libc::ICRNL);
        raw.c_cc[libc::VSUSP] = 0;
        raw.c_cc[libc::VQUIT] = 0;
        AS_IT_WAS.store(Box::into_raw(Box::new(as_it_was)), Ordering::SeqCst);
        for signal in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP] {
            // SAFETY: `give_back` only makes calls a signal handler may make.
            unsafe { libc::signal(signal, give_back as *const () as libc::sighandler_t) };
        }
        raw.c_cc[libc::VMIN] = 1;
        raw.c_cc[libc::VTIME] = 0;
        // SAFETY: as above, with settings derived from the terminal's own.
        if unsafe { libc::tcsetattr(0, libc::TCSANOW, &raw) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        put("\x1b[?1049h\x1b[?25l\x1b[2J");
        let view = Arc::new(Mutex::new(View::default()));
        let over = Arc::new(AtomicBool::new(false));
        let drawing = std::thread::spawn({
            let (view, over) = (view.clone(), over.clone());
            move || draw_until(&view, &over)
        });
        Ok(Self { view, over, drawing: Some(drawing), as_it_was, previews: Vec::new() })
    }

    fn change(&self, change: impl FnOnce(&mut View)) {
        if let Ok(mut view) = self.view.lock() {
            change(&mut view);
        }
    }

    /// Gives the terminal back as it was found.
    fn close(&mut self) {
        self.over.store(true, Ordering::SeqCst);
        if let Some(drawing) = self.drawing.take() {
            let _ = drawing.join();
            put("\x1b[?25h\x1b[?1049l");
            // SAFETY: the settings read from this same terminal at the start.
            unsafe { libc::tcsetattr(0, libc::TCSANOW, &self.as_it_was) };
            for signal in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP] {
                // SAFETY: back to what a signal does by itself.
                unsafe { libc::signal(signal, libc::SIG_DFL) };
            }
            AS_IT_WAS.store(std::ptr::null_mut(), Ordering::SeqCst);
        }
    }

    /// The next key pressed.
    fn key(&mut self) -> Result<Key, Left> {
        let Some(first) = byte(None) else { return Err(Left) };
        Ok(match first {
            4 => Key::Leave,
            b'\r' | b'\n' => Key::Enter,
            0x7f | 0x08 => Key::Erase,
            0x15 => Key::EraseAll,
            0x1b => {
                // An arrow says more right away; Escape alone says nothing.
                let opens = byte(Some(40)).filter(|opens| matches!(opens, b'[' | b'O'));
                match (opens, opens.and_then(|_| byte(Some(40)))) {
                    (Some(_), Some(b'A')) => Key::Up,
                    (Some(_), Some(b'B')) => Key::Down,
                    (Some(b'['), Some(mut last)) => {
                        // Whatever else it is, read to its end and passed over.
                        while !(0x40..=0x7e).contains(&last) {
                            let Some(next) = byte(Some(40)) else { break };
                            last = next;
                        }
                        Key::Other
                    }
                    _ => Key::Other,
                }
            }
            first if first < 0x20 => Key::Other,
            first => {
                // A letter of several bytes is read whole.
                let more = match first {
                    0xc0..=0xdf => 1,
                    0xe0..=0xef => 2,
                    0xf0..=0xf7 => 3,
                    _ => 0,
                };
                let mut bytes = vec![first];
                for _ in 0..more {
                    bytes.extend(byte(Some(40)));
                }
                match std::str::from_utf8(&bytes).ok().and_then(|text| text.chars().next()) {
                    Some(letter) => Key::Letter(letter),
                    None => Key::Other,
                }
            }
        })
    }

    fn leave(&mut self) -> Left {
        self.close();
        Left
    }

    /// Reads a line typed under `question`.
    fn line(&mut self, question: &str, hidden: bool) -> Result<String, Left> {
        forget_what_was_typed_ahead();
        self.change(|view| {
            view.mood = Mood::Waiting;
            view.asking = Asking::Line { question: clean(question), typed: String::new(), hidden };
        });
        let mut typed = String::new();
        loop {
            match self.key()? {
                Key::Leave => return Err(self.leave()),
                Key::Enter if !typed.trim().is_empty() => break,
                Key::Erase => {
                    typed.pop();
                }
                Key::EraseAll => typed.clear(),
                Key::Letter(letter) => typed.push(letter),
                _ => {}
            }
            let now = typed.clone();
            self.change(|view| {
                if let Asking::Line { typed, .. } = &mut view.asking {
                    *typed = now;
                }
            });
        }
        self.answered();
        Ok(typed.trim().to_owned())
    }

    /// What was said for the question just answered goes.
    fn answered(&self) {
        self.change(|view| {
            view.said.clear();
            view.trouble = None;
            view.asking = Asking::Nothing;
        });
    }
}

/// The terminal's settings before setup took it, for `give_back`.
static AS_IT_WAS: std::sync::atomic::AtomicPtr<libc::termios> =
    std::sync::atomic::AtomicPtr::new(std::ptr::null_mut());

/// Stopped by a signal, gives the terminal back as it was and ends.
extern "C" fn give_back(signal: libc::c_int) {
    const BACK: &[u8] = b"\x1b[?25h\x1b[?1049l";
    let as_it_was = AS_IT_WAS.load(Ordering::SeqCst);
    // SAFETY: only calls a signal handler may make, on this process's own
    // terminal, with settings kept for as long as the process lives.
    unsafe {
        if !as_it_was.is_null() {
            libc::tcsetattr(0, libc::TCSANOW, as_it_was);
        }
        libc::write(1, BACK.as_ptr().cast(), BACK.len());
        libc::_exit(128 + signal);
    }
}

/// Writes to the terminal; one that went away is no reason to fail.
fn put(text: &str) {
    let mut out = std::io::stdout();
    let _ = out.write_all(text.as_bytes()).and_then(|()| out.flush());
}

/// What was typed before a question was asked is not an answer to it: a
/// line pasted after a key must not confirm what comes next.
fn forget_what_was_typed_ahead() {
    // SAFETY: descriptor 0 is this process's own input.
    unsafe { libc::tcflush(0, libc::TCIFLUSH) };
}

/// Reads one byte typed, waiting at most `patience` milliseconds when given.
fn byte(patience: Option<i32>) -> Option<u8> {
    if let Some(patience) = patience {
        let mut waiting = libc::pollfd { fd: 0, events: libc::POLLIN, revents: 0 };
        // SAFETY: one descriptor, described by a value that outlives the call.
        if unsafe { libc::poll(&mut waiting, 1, patience) } <= 0 {
            return None;
        }
    }
    let mut read = [0u8; 1];
    // SAFETY: reads at most one byte into a buffer of one.
    let got = unsafe { libc::read(0, read.as_mut_ptr().cast(), 1) };
    (got == 1).then_some(read[0])
}

/// The terminal's size, in columns and lines.
fn size() -> (usize, usize) {
    // SAFETY: `winsize` is plain data that the call fills in.
    let mut size: libc::winsize = unsafe { std::mem::zeroed() };
    // SAFETY: descriptor 1 is this process's own output.
    let known = unsafe { libc::ioctl(1, libc::TIOCGWINSZ, &mut size) } == 0;
    if known && size.ws_col > 0 && size.ws_row > 0 {
        (size.ws_col as usize, size.ws_row as usize)
    } else {
        (80, 24)
    }
}

/// Draws the screen again and again until `over`, writing only the lines
/// that changed: what the Owner selected to copy stays selected.
fn draw_until(view: &Mutex<View>, over: &AtomicBool) {
    // Each row drawn, with the line it starts on.
    let mut drawn: Vec<(usize, Row)> = Vec::new();
    let mut drawn_for = (0, 0);
    let mut tick = 0u64;
    while !over.load(Ordering::SeqCst) {
        let (columns, lines) = size();
        let rows = match view.lock() {
            Ok(view) => frame(&view, columns, lines, tick),
            Err(_) => break,
        };
        let mut out = String::new();
        if drawn_for != (columns, lines) {
            out.push_str("\x1b[2J");
            drawn.clear();
            drawn_for = (columns, lines);
        }
        let mut line = 1;
        let mut placed = Vec::new();
        // From a row that moved or that takes several lines, all is drawn anew.
        let mut anew = false;
        for (place, row) in rows.into_iter().enumerate() {
            if line > lines {
                break;
            }
            let before = drawn.get(place);
            if !anew && before.is_some_and(|(at, _)| *at != line)
                || (before.map(|(_, before)| before) != Some(&row)
                    && (row.width > columns || before.is_some_and(|(_, old)| old.width > columns)))
            {
                anew = true;
                out.push_str(&format!("\x1b[{line};1H\x1b[J"));
            }
            if anew || before != Some(&(line, row.clone())) {
                out.push_str(&format!("\x1b[{line};1H{}\x1b[K", row.text));
            }
            // A row longer than the terminal is wide takes several lines.
            let takes = row.width.max(1).div_ceil(columns);
            placed.push((line, row));
            line += takes;
        }
        if drawn.len() > placed.len() && line <= lines && !anew {
            out.push_str(&format!("\x1b[{line};1H\x1b[J"));
        }
        if !out.is_empty() {
            put(&out);
        }
        drawn = placed;
        tick += 1;
        std::thread::sleep(TICK);
    }
}

impl Drop for Welcome {
    fn drop(&mut self) {
        self.close();
    }
}

impl Front for Welcome {
    fn speak(&mut self, language: Language) {
        self.change(|view| view.french = language == Language::French);
    }

    fn step(&mut self, at: usize, of: usize) {
        self.change(|view| view.step = (at, of));
    }

    fn choose(
        &mut self,
        question: &str,
        choices: &[Choice],
        suggested: usize,
        own: Option<&str>,
    ) -> Result<Picked, Left> {
        let all = choices.len() + usize::from(own.is_some());
        let mut at = suggested.min(all.saturating_sub(1));
        let mut typed = String::new();
        forget_what_was_typed_ahead();
        self.change(|view| {
            view.mood = Mood::Waiting;
            view.asking = Asking::Choices {
                question: clean(question),
                choices: choices
                    .iter()
                    .map(|choice| Choice::of(clean(&choice.label)).hinted(clean(&choice.hint)))
                    .collect(),
                at,
                own: own.map(|invites| (clean(invites), String::new())),
            };
        });
        let picked = loop {
            let on_own = own.is_some() && at == choices.len();
            match self.key()? {
                Key::Leave => return Err(self.leave()),
                Key::Up => at = at.checked_sub(1).unwrap_or(all - 1),
                Key::Down => at = (at + 1) % all,
                Key::Enter if on_own && !typed.trim().is_empty() => {
                    break Picked::Typed(typed.trim().to_owned());
                }
                Key::Enter if !on_own => break Picked::Choice(at),
                Key::Erase if on_own => {
                    typed.pop();
                }
                Key::EraseAll if on_own => typed.clear(),
                // Writing goes to the answer of one's own, wherever one was.
                Key::Letter(letter) if own.is_some() => {
                    at = choices.len();
                    typed.push(letter);
                }
                // Without one, a number is the place of a choice.
                Key::Letter(letter) => {
                    if let Some(place) = letter.to_digit(10).map(|digit| digit as usize)
                        && (1..=all).contains(&place)
                    {
                        at = place - 1;
                    }
                }
                _ => {}
            }
            let (now_at, now_typed) = (at, typed.clone());
            let pointed_at = self.previews.get(at).copied();
            self.change(|view| {
                if pointed_at.is_some() {
                    view.look = pointed_at;
                }
                if let Asking::Choices { at, own, .. } = &mut view.asking {
                    *at = now_at;
                    if let Some((_, typed)) = own {
                        *typed = now_typed;
                    }
                }
            });
        };
        self.answered();
        Ok(picked)
    }

    fn named(&mut self, name: &str) {
        self.change(|view| view.name = Some(clean(name).replace('\n', " ")));
    }

    fn looks(&mut self, look: Option<Look>) {
        self.change(|view| view.look = look);
    }

    fn choose_a_look(
        &mut self,
        question: &str,
        choices: &[(Choice, Look)],
        suggested: usize,
    ) -> Result<usize, Left> {
        let named: Vec<Choice> = choices.iter().map(|(choice, _)| choice.clone()).collect();
        self.previews = choices.iter().map(|(_, look)| *look).collect();
        self.looks(self.previews.get(suggested).copied());
        let picked = self.choose(question, &named, suggested, None);
        self.previews.clear();
        match picked? {
            Picked::Choice(place) => Ok(place),
            Picked::Typed(_) => Ok(suggested),
        }
    }

    fn ask(&mut self, question: &str) -> Result<String, Left> {
        self.line(question, false)
    }

    fn ask_hidden(&mut self, question: &str) -> Result<String, Left> {
        self.line(question, true)
    }

    fn say(&mut self, text: &str) {
        self.change(|view| {
            view.said.push(Said::Text(clean(text)));
            view.asking = Asking::Nothing;
            view.mood = Mood::Waiting;
        });
    }

    fn show(&mut self, exactly: &str) {
        self.change(|view| view.said.push(Said::Exactly(clean(exactly).replace('\n', " "))));
    }

    fn working(&mut self, text: &str) {
        self.change(|view| {
            view.mood = Mood::Working;
            view.asking = Asking::Working(clean(text.trim_end_matches('.')) + "…");
        });
    }

    fn good(&mut self, text: &str) {
        self.change(|view| {
            view.kept.push(clean(text));
            view.trouble = None;
            view.asking = Asking::Nothing;
            view.mood = Mood::Waiting;
        });
    }

    fn wrong(&mut self, text: &str) {
        self.change(|view| {
            view.trouble = Some(clean(text));
            view.asking = Asking::Nothing;
            view.mood = Mood::Waiting;
        });
    }

    fn finish(&mut self, text: &str, link: Option<&str>, after: &str) {
        // A glad instant on the screen, then what stays in the terminal.
        let (text, after) = (clean(text), clean(after));
        let link = link.map(|link| clean(link).replace('\n', ""));
        let first = text.split_inclusive('.').next().unwrap_or(&text).to_owned();
        self.change(|view| {
            view.mood = Mood::Glad;
            view.step = (view.step.1, view.step.1);
            view.said.clear();
            view.trouble = None;
            view.asking = Asking::Nothing;
            view.kept.push(first);
        });
        std::thread::sleep(Duration::from_millis(1400));
        self.close();
        let (columns, _) = size();
        for row in farewell(&text, link.as_deref(), &after, columns) {
            put(&format!("{}\n", row.text));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn letters(rows: &[Row]) -> String {
        // What is written, without what colours it.
        let mut plain = String::new();
        for row in rows {
            let mut colouring = false;
            for letter in row.text.chars() {
                match (colouring, letter) {
                    (false, '\x1b') => colouring = true,
                    (true, 'm') => colouring = false,
                    (false, letter) => plain.push(letter),
                    _ => {}
                }
            }
            plain.push('\n');
        }
        plain
    }

    #[test]
    fn a_row_takes_the_room_of_its_letters_and_not_of_its_colours() {
        for row in frame(&View::default(), 80, 30, 0)
            .iter()
            .chain(&mascot(None, Mood::Working, 7))
            .chain(&mascot(Some(Look::called(Some("star"), Some("teal"))), Mood::Working, 7))
        {
            let written = letters(std::slice::from_ref(row));
            assert_eq!(written.trim_end_matches('\n').chars().count(), row.width, "{row:?}");
        }
    }

    #[test]
    fn what_is_typed_in_secret_is_never_on_the_screen() {
        let view = View {
            asking: Asking::Line {
                question: "Your API key".into(),
                typed: "sk-example-only".into(),
                hidden: true,
            },
            ..View::default()
        };
        let shown = letters(&frame(&view, 80, 30, 0));
        assert!(!shown.contains("sk-example"), "{shown}");
        assert!(shown.contains("15 characters"), "{shown}");
    }

    #[test]
    fn a_small_terminal_keeps_the_choice_pointed_at_in_view() {
        let choices: Vec<Choice> =
            (1..=15).map(|place| Choice::of(format!("choice {place}"))).collect();
        let view = View {
            asking: Asking::Choices { question: "Which?".into(), choices, at: 13, own: None },
            ..View::default()
        };
        let rows = frame(&view, 80, 24, 0);
        assert!(rows.len() <= 24, "{} rows", rows.len());
        assert!(letters(&rows).contains("❯ choice 14"), "{}", letters(&rows));
    }

    #[test]
    fn the_question_stays_in_view_on_a_terminal_too_small_for_everything() {
        let telegram = "You will talk to your agent in Telegram, through a bot of your own:\n  \
                        1. In Telegram, open a chat with @BotFather.\n  \
                        2. Send /newbot and follow its two questions (a name, then a username \
                        ending in \"bot\").\n  3. BotFather gives you a token: paste it here.";
        let view = View {
            step: (3, 4),
            kept: vec!["Anthropic answers (claude-sonnet-5-5)".into(), "Reached".into()],
            said: vec![Said::Text(telegram.into())],
            asking: Asking::Line {
                question: "Bot token".into(),
                typed: String::new(),
                hidden: true,
            },
            ..View::default()
        };
        for (columns, lines) in [(80, 24), (60, 18), (100, 40)] {
            let rows = frame(&view, columns, lines, 0);
            let takes: usize = rows.iter().map(|row| row.width.max(1).div_ceil(columns)).sum();
            assert!(takes <= lines, "{takes} lines on a terminal of {lines}");
            assert!(letters(&rows).contains("Bot token"), "{}", letters(&rows));
        }
        // With room for it, the mascot is there.
        assert!(letters(&frame(&view, 100, 40, 0)).contains('█'));
        assert!(!letters(&frame(&view, 80, 24, 0)).contains('█'));
    }

    #[test]
    fn what_comes_from_outside_is_written_and_never_obeyed() {
        let said = clean("refused\x1b[2J\x1b]0;title\x07 key\r\nnext\tline");
        assert_eq!(said, "refused[2J]0;title key\nnext line");
    }

    #[test]
    fn a_link_is_drawn_for_a_page_too() {
        let picture = code_picture("https://host.example.ts.net:8377/#ABCD-EFGH").unwrap();
        assert!(picture.starts_with("<svg") && picture.contains("M3 3h1v1h-1z"), "{picture}");
    }

    #[test]
    fn a_link_is_drawn_as_a_code_and_never_broken() {
        let link = "https://host.example.ts.net:8377/#123456";
        let rows = farewell("All set.", Some(link), "", 60);
        assert!(letters(&rows).contains(link));
        let code = square_code(link);
        assert!(code.len() > 10 && code.iter().all(|row| row.width == code[0].width));
    }
}
