//! The Agent's Computer: a screen of its own on the Host, looked at through
//! pictures and acted on with a mouse and a keyboard, as a person would.

use std::io::{self, BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{ConnectionExt, ImageFormat};

/// One screen, and the program that serves it when Orbite started it.
pub struct Screen {
    display: String,
    serving: Option<Child>,
    /// The key to the screen, and the file programs read it from: without
    /// it, no other account of the machine can look at the screen or act on
    /// it. None for a screen Orbite did not start.
    key: Option<(Vec<u8>, PathBuf)>,
}

/// The way of proving oneself to a screen that Orbite uses.
const KEY_KIND: &[u8] = b"MIT-MAGIC-COOKIE-1";

impl Screen {
    /// Starts a screen of its own, `width` by `height`, that nothing else
    /// on the machine looks at; it ends with this value.
    pub fn start(width: u16, height: u16) -> io::Result<Self> {
        // A key made for this screen, in a file only this account reads, in
        // the form X programs look their key up in: for any host, any number.
        let mut key = vec![0u8; 16];
        getrandom::fill(&mut key).map_err(io::Error::other)?;
        static SCREENS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let kept = std::env::temp_dir().join(format!(
            "orbite-screen-{}-{}.key",
            std::process::id(),
            SCREENS.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let mut entry = vec![0xff, 0xff, 0, 0, 0, 0];
        entry.extend_from_slice(&(KEY_KIND.len() as u16).to_be_bytes());
        entry.extend_from_slice(KEY_KIND);
        entry.extend_from_slice(&(key.len() as u16).to_be_bytes());
        entry.extend_from_slice(&key);
        {
            use std::io::Write;
            use std::os::unix::fs::OpenOptionsExt;
            let _ = std::fs::remove_file(&kept);
            let mut file =
                std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&kept)?;
            file.write_all(&entry)?;
        }
        // The display's number is chosen by the server, which says it on its output.
        let mut serving = Command::new("Xvfb")
            .args([
                "-displayfd",
                "1",
                "-screen",
                "0",
                &format!("{width}x{height}x24"),
                "-nolisten",
                "tcp",
                "-auth",
            ])
            .arg(&kept)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let mut said = String::new();
        let told = serving.stdout.take().map(|output| BufReader::new(output).read_line(&mut said));
        let number = said.trim();
        if !matches!(told, Some(Ok(_))) || number.is_empty() || number.parse::<u32>().is_err() {
            let _ = serving.kill();
            let _ = serving.wait();
            let _ = std::fs::remove_file(&kept);
            return Err(io::Error::other("the screen did not start"));
        }
        Ok(Self { display: format!(":{number}"), serving: Some(serving), key: Some((key, kept)) })
    }

    /// A screen that is already there, by its X display name.
    pub fn at(display: impl Into<String>) -> Self {
        Self { display: display.into(), serving: None, key: None }
    }

    /// The screen's X display name, for programs to show themselves on it.
    pub fn display(&self) -> &str {
        &self.display
    }

    /// False once the program serving a screen Orbite started has ended.
    fn is_served(&mut self) -> bool {
        self.serving.as_mut().is_none_or(|serving| matches!(serving.try_wait(), Ok(None)))
    }

    /// A connection to the screen, as Orbite itself has: with its key.
    pub fn connect(&self) -> io::Result<(x11rb::rust_connection::RustConnection, usize)> {
        let Some((key, _)) = &self.key else {
            return x11rb::connect(Some(&self.display)).map_err(io::Error::other);
        };
        let number = self.display.trim_start_matches(':');
        let socket = std::os::unix::net::UnixStream::connect(format!("/tmp/.X11-unix/X{number}"))?;
        let (stream, _) = x11rb::rust_connection::DefaultStream::from_unix_stream(socket)
            .map_err(io::Error::other)?;
        let connection = x11rb::rust_connection::RustConnection::connect_to_stream_with_auth_info(
            stream,
            0,
            KEY_KIND.to_vec(),
            key.clone(),
        )
        .map_err(io::Error::other)?;
        Ok((connection, 0))
    }

    /// The file programs shown on this screen read its key from.
    pub fn key_file(&self) -> Option<&Path> {
        self.key.as_ref().map(|(_, kept)| kept.as_path())
    }

    /// What the screen shows now, as a PNG file.
    pub fn picture(&self) -> io::Result<Vec<u8>> {
        let (width, height, points) = self.points()?;
        Ok(crate::png::encode(width, height, &points))
    }

    /// What the screen shows now: its size, then red, green and blue for
    /// each point, row after row.
    fn points(&self) -> io::Result<(u32, u32, Vec<u8>)> {
        let (connection, number) = self.connect()?;
        let screen = &connection.setup().roots[number];
        let (width, height) = (screen.width_in_pixels, screen.height_in_pixels);
        let taken = connection
            .get_image(ImageFormat::Z_PIXMAP, screen.root, 0, 0, width, height, !0)
            .map_err(io::Error::other)?
            .reply()
            .map_err(io::Error::other)?;
        // Four bytes a point, blue first, as the screen keeps them.
        let across = taken.data.len() / (width as usize * height as usize).max(1);
        if across < 3 {
            return Err(io::Error::other(
                "the screen keeps its picture in a form Orbite does not read",
            ));
        }
        let mut points = Vec::with_capacity(width as usize * height as usize * 3);
        for point in taken.data.chunks_exact(across) {
            points.extend_from_slice(&[point[2], point[1], point[0]]);
        }
        Ok((width.into(), height.into(), points))
    }

    /// Moves the pointer to a point of the screen and clicks there.
    pub fn click(&self, x: u16, y: u16) -> io::Result<()> {
        self.act(&["mousemove", "--sync", &x.to_string(), &y.to_string(), "click", "1"])
    }

    /// Clicks twice at a point, as to open something or pick a word.
    pub fn double_click(&self, x: u16, y: u16) -> io::Result<()> {
        self.act(&[
            "mousemove",
            "--sync",
            &x.to_string(),
            &y.to_string(),
            "click",
            "--repeat",
            "2",
            "1",
        ])
    }

    /// Types `text`, as on a keyboard. A letter the keyboard does not have
    /// is first given a key of its own, and programs are left the time to
    /// learn of it before that key is pressed: pressed at once, as the usual
    /// tools do, browsers miss it and the letter is lost.
    pub fn type_text(&self, text: &str) -> io::Result<()> {
        use x11rb::protocol::xtest::ConnectionExt as _;
        const PRESS: u8 = 2;
        const RELEASE: u8 = 3;
        let wrong = |error: &dyn std::fmt::Display| io::Error::other(error.to_string());
        let (connection, number) = self.connect()?;
        let setup = connection.setup();
        let root = setup.roots[number].root;
        let (first, last) = (setup.min_keycode, setup.max_keycode);
        let count = last - first + 1;
        let read = || -> io::Result<(usize, Vec<u32>)> {
            let keyboard = connection
                .get_keyboard_mapping(first, count)
                .map_err(|e| wrong(&e))?
                .reply()
                .map_err(|e| wrong(&e))?;
            Ok((usize::from(keyboard.keysyms_per_keycode), keyboard.keysyms))
        };
        // The sign a letter is known by to the screen.
        let sign = |letter: char| -> u32 {
            match letter {
                '\n' | '\r' => 0xff0d,
                '\t' => 0xff09,
                letter if (letter as u32) < 0x100 => letter as u32,
                letter => 0x0100_0000 + letter as u32,
            }
        };
        let letters: Vec<char> = text.chars().collect();
        let mut at = 0;
        while at < letters.len() {
            let (per, signs) = read()?;
            if per == 0 {
                return Err(io::Error::other("the screen has no keyboard"));
            }
            // Where a sign is on the keyboard: its key, and whether with Shift.
            let find = |signs: &[u32], wanted: u32| -> Option<(u8, bool)> {
                signs.chunks(per).enumerate().find_map(|(key, levels)| {
                    let level = levels.iter().take(2).position(|sign| *sign == wanted)?;
                    Some((first + key as u8, level == 1))
                })
            };
            let shift = find(&signs, 0xffe1).map(|(key, _)| key);
            let mut free: Vec<u8> = signs
                .chunks(per)
                .enumerate()
                .filter(|(_, levels)| levels.iter().all(|sign| *sign == 0))
                .map(|(key, _)| first + key as u8)
                .collect();
            // As many letters as can be typed with the keys there are.
            let mut lent: Vec<(u8, u32)> = Vec::new();
            let mut upto = at;
            while upto < letters.len() {
                let wanted = sign(letters[upto]);
                if find(&signs, wanted).is_none() && !lent.iter().any(|(_, sign)| *sign == wanted) {
                    let Some(key) = free.pop() else { break };
                    lent.push((key, wanted));
                }
                upto += 1;
            }
            if upto == at {
                return Err(io::Error::other("the screen's keyboard has no key to spare"));
            }
            for (key, wanted) in &lent {
                connection
                    .change_keyboard_mapping(1, *key, per as u8, &vec![*wanted; per])
                    .map_err(|e| wrong(&e))?;
            }
            connection.flush().map_err(|e| wrong(&e))?;
            if !lent.is_empty() {
                std::thread::sleep(Duration::from_millis(350));
            }
            let (_, signs) = read()?;
            for letter in &letters[at..upto] {
                let Some((key, shifted)) = find(&signs, sign(*letter)) else { continue };
                let shift = shift.filter(|_| shifted);
                let mut strokes = Vec::new();
                strokes.extend(shift.map(|shift| (PRESS, shift)));
                strokes.extend([(PRESS, key), (RELEASE, key)]);
                strokes.extend(shift.map(|shift| (RELEASE, shift)));
                for (kind, key) in strokes {
                    connection
                        .xtest_fake_input(kind, key, 0, root, 0, 0, 0)
                        .map_err(|e| wrong(&e))?;
                }
                connection.flush().map_err(|e| wrong(&e))?;
                std::thread::sleep(Duration::from_millis(12));
            }
            // The keys lent go back once what they typed has been taken in.
            if !lent.is_empty() {
                std::thread::sleep(Duration::from_millis(350));
                for (key, _) in &lent {
                    connection
                        .change_keyboard_mapping(1, *key, per as u8, &vec![0; per])
                        .map_err(|e| wrong(&e))?;
                }
                connection.flush().map_err(|e| wrong(&e))?;
            }
            at = upto;
        }
        // Everything asked of the screen has been done before this returns.
        connection.get_input_focus().map_err(|e| wrong(&e))?.reply().map_err(|e| wrong(&e))?;
        Ok(())
    }

    /// Presses a key or a combination, such as `Return` or `ctrl+l`.
    pub fn key(&self, keys: &str) -> io::Result<()> {
        self.act(&["key", "--", keys])
    }

    /// Turns the wheel, down for a positive number of notches and up for a negative one.
    pub fn scroll(&self, notches: i32) -> io::Result<()> {
        let button = if notches >= 0 { "5" } else { "4" };
        self.act(&["click", "--repeat", &notches.unsigned_abs().max(1).to_string(), button])
    }

    /// Has the mouse and keyboard do something on this screen.
    fn act(&self, gesture: &[&str]) -> io::Result<()> {
        let mut acting = Command::new("xdotool");
        // With no language set, as under a service, the keyboard program
        // reads a text one byte at a time and drops every letter with an
        // accent: it is told the text is UTF-8.
        acting
            .args(gesture)
            .env("DISPLAY", &self.display)
            .env_remove("XAUTHORITY")
            .env("LC_ALL", "C.UTF-8");
        if let Some(key) = self.key_file() {
            acting.env("XAUTHORITY", key);
        }
        let done =
            acting.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped()).output()?;
        if done.status.success() {
            Ok(())
        } else {
            Err(io::Error::other(String::from_utf8_lossy(&done.stderr).trim().to_owned()))
        }
    }
}

impl Drop for Screen {
    fn drop(&mut self) {
        if let Some((_, kept)) = &self.key {
            let _ = std::fs::remove_file(kept);
        }
        if let Some(serving) = &mut self.serving {
            let _ = serving.kill();
            let _ = serving.wait();
        }
    }
}

/// The browsers Orbite knows how to start, by command, in order of
/// preference, and whether each is of Chrome's family.
const BROWSERS: [(&str, bool); 5] = [
    ("firefox", false),
    ("firefox-esr", false),
    ("google-chrome", true),
    ("chromium", true),
    ("chromium-browser", true),
];

/// Settings a person would choose on Firefox's first day, chosen for the
/// Agent: no welcome pages, no question about being the default browser.
const FIREFOX_FIRST_DAY: &str = r#"user_pref("browser.aboutwelcome.enabled", false);
user_pref("browser.shell.checkDefaultBrowser", false);
user_pref("browser.startup.homepage_override.mstone", "ignore");
user_pref("startup.homepage_welcome_url", "");
user_pref("startup.homepage_welcome_url.additional", "");
user_pref("datareporting.policy.dataSubmissionPolicyBypassNotification", true);
user_pref("browser.tabs.warnOnClose", false);
user_pref("browser.sessionstore.resume_from_crash", false);
"#;

/// The Agent's Computer: its screen, and a browser that keeps what sites
/// leave in it from one day to the next.
pub struct Computer {
    screen: Screen,
    /// Where the browser keeps its sign-ins, history and settings.
    profile: PathBuf,
    /// The browser's command, and whether it is of Chrome's family.
    browser: (String, bool),
    running: Option<Child>,
    size: (u16, u16),
}

impl Computer {
    /// True when this machine has what a Computer is made of: the program
    /// that serves a screen, the one that moves the mouse, and a browser.
    pub fn can_start() -> bool {
        let there = |command: &str, argument: &str| {
            Command::new(command)
                .arg(argument)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok()
        };
        there("Xvfb", "-help")
            && there("xdotool", "version")
            && BROWSERS.iter().any(|(browser, _)| there(browser, "--version"))
    }

    /// Starts a screen of its own for a browser that keeps its data in `profile`.
    pub fn start(profile: impl AsRef<Path>, width: u16, height: u16) -> io::Result<Self> {
        // ORBITE_BROWSER names one of them, when the first found is not the one wanted.
        let wanted = std::env::var("ORBITE_BROWSER").ok().filter(|wanted| !wanted.is_empty());
        let found = BROWSERS
            .iter()
            .filter(|(command, _)| wanted.as_deref().is_none_or(|wanted| wanted == *command))
            .find(|(command, _)| {
                Command::new(command)
                    .arg("--version")
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                    .is_ok_and(|status| status.success())
            });
        let Some((command, chrome)) = found else {
            return Err(io::Error::other("no browser Orbite knows is installed"));
        };
        std::fs::create_dir_all(&profile)?;
        // An Orbite that was killed leaves its screen and browser running:
        // the browser would take every address asked of this one, and show
        // it on a screen nobody looks at.
        stop_what_was_left(profile.as_ref())?;
        let screen = Screen::start(width, height)?;
        let me = std::process::id();
        note(profile.as_ref(), &format!("owner {me} {}", started_at(me)));
        if let Some(key) = screen.key_file() {
            note(profile.as_ref(), &format!("key {}", key.display()));
        }
        if let Some(serving) = &screen.serving {
            note(profile.as_ref(), &serving.id().to_string());
        }
        Ok(Self {
            screen,
            profile: profile.as_ref().to_owned(),
            browser: ((*command).to_owned(), *chrome),
            running: None,
            size: (width, height),
        })
    }

    pub fn screen(&self) -> &Screen {
        &self.screen
    }

    /// How far down the screen a page starts, below the browser's own bars.
    pub fn page_top(&self) -> u16 {
        if self.browser.1 { 87 } else { 72 }
    }

    /// Shows `address` in the browser, starting the browser when it is not running.
    pub fn open(&mut self, address: &str) -> io::Result<()> {
        let running =
            self.running.as_mut().is_some_and(|browser| matches!(browser.try_wait(), Ok(None)));
        let (command, chrome) = &self.browser;
        let mut starting = Command::new(command);
        // On this screen and no other: a browser that finds a desktop
        // session in its environment would open there instead.
        starting
            .env("DISPLAY", self.screen.display())
            .env_remove("WAYLAND_DISPLAY")
            .env_remove("XDG_SESSION_TYPE")
            .env_remove("XAUTHORITY")
            .envs(self.screen.key_file().map(|key| ("XAUTHORITY", key)))
            .env("MOZ_ENABLE_WAYLAND", "0")
            .env(
                "LC_ALL",
                std::env::var("LC_ALL")
                    .or_else(|_| std::env::var("LANG"))
                    .unwrap_or_else(|_| "C.UTF-8".into()),
            )
            // Whatever the browser keeps outside its profile stays with it,
            // in the Agent's folder, whoever asked for it to be started.
            .env("HOME", self.profile.parent().unwrap_or(&self.profile))
            .env_remove("XDG_CONFIG_HOME")
            .env_remove("XDG_CACHE_HOME")
            .env_remove("XDG_RUNTIME_DIR")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if *chrome {
            starting
                .arg(format!("--user-data-dir={}", self.profile.display()))
                .args([
                    "--ozone-platform=x11",
                    "--no-first-run",
                    "--no-default-browser-check",
                    "--password-store=basic",
                ])
                .arg("--window-position=0,0")
                .arg(format!("--window-size={},{}", self.size.0, self.size.1));
        } else {
            // Firefox's own first-day pages would cover what was asked for.
            let settled = self.profile.join("user.js");
            if !settled.exists() {
                std::fs::write(&settled, FIREFOX_FIRST_DAY)?;
            }
            starting.arg("--profile").arg(&self.profile).args([
                "--width",
                &self.size.0.to_string(),
                "--height",
                &self.size.1.to_string(),
            ]);
        }
        // Asked of a browser already running, the address opens in it.
        let started = starting.arg(address).spawn()?;
        if !running {
            note(&self.profile, &started.id().to_string());
        }
        if running {
            // That second program hands the address over and ends.
            std::thread::spawn(move || {
                let mut handing_over = started;
                let _ = handing_over.wait();
            });
        } else {
            self.running = Some(started);
        }
        Ok(())
    }

    /// Asks the browser to close, giving it time to keep what it has to.
    pub fn close_browser(&mut self) {
        let Some(mut browser) = self.running.take() else { return };
        // Firefox is asked as a person would, by its own shortcut: stopped
        // from outside, it does not always write down what sites just kept.
        if !self.browser.1 && self.screen.key("ctrl+q").is_ok() {
            let until = Instant::now() + Duration::from_secs(10);
            while Instant::now() < until && matches!(browser.try_wait(), Ok(None)) {
                std::thread::sleep(Duration::from_millis(100));
            }
        }
        // SAFETY: a signal sent to a program this one started.
        unsafe { libc::kill(browser.id() as libc::pid_t, libc::SIGTERM) };
        let until = Instant::now() + Duration::from_secs(10);
        while Instant::now() < until && matches!(browser.try_wait(), Ok(None)) {
            std::thread::sleep(Duration::from_millis(100));
        }
        let _ = browser.kill();
        let _ = browser.wait();
    }
}

impl Drop for Computer {
    fn drop(&mut self) {
        self.close_browser();
        let _ = std::fs::remove_file(running_file(&self.profile));
    }
}

/// The Computer as the Agent holds it: started the first time it is used,
/// with the pictures taken of its screen kept in its folder.
pub(crate) struct Desk {
    folder: PathBuf,
    computer: std::sync::Mutex<Option<Computer>>,
    taken: std::sync::atomic::AtomicU32,
}

/// The size of the Computer's screen.
const SCREEN: (u16, u16) = (1280, 800);

/// How much of the screen may differ between two looks for it to count as
/// still: about a text cursor that blinks, in colour values.
const CURSOR_SIZED: usize = 600;

/// The size of the picture of the screen a model is shown: two thirds of it.
pub(crate) const SHOWN: (u16, u16) = (853, 533);

/// The point of the screen that a point of the picture shown to a model stands for.
pub(crate) fn on_the_screen(x: u16, y: u16) -> (u16, u16) {
    let scaled = |at: u16, shown: u16, whole: u16| {
        ((u32::from(at) * u32::from(whole) + u32::from(shown) / 2) / u32::from(shown))
            .min(u32::from(whole) - 1) as u16
    };
    (scaled(x, SHOWN.0, SCREEN.0), scaled(y, SHOWN.1, SCREEN.1))
}

/// Where the smaller picture a model is shown is kept, beside the whole one.
pub(crate) fn shown_to_the_model(whole: &Path) -> PathBuf {
    whole.with_extension("shown.png")
}

/// How many pictures of the screen are kept on disk.
const PICTURES_KEPT: u32 = 6;

impl Desk {
    pub(crate) fn in_folder(folder: PathBuf) -> Self {
        Self {
            folder,
            computer: std::sync::Mutex::new(None),
            taken: std::sync::atomic::AtomicU32::new(0),
        }
    }

    /// Does `something` with the Computer, starting it first when needed.
    pub(crate) fn with<T>(
        &self,
        something: impl FnOnce(&mut Computer) -> io::Result<T>,
    ) -> io::Result<T> {
        let mut computer = self.computer.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        // A screen that died is not one to act on: a new Computer is started.
        if computer.as_mut().is_some_and(|computer| !computer.screen.is_served()) {
            drop(computer.take());
        }
        if computer.is_none() {
            *computer = Some(Computer::start(self.folder.join("profile"), SCREEN.0, SCREEN.1)?);
        }
        match computer.as_mut() {
            Some(computer) => something(computer),
            None => Err(io::Error::other("the Computer did not start")),
        }
    }

    /// Closes the browser and the screen, when they were started.
    pub(crate) fn stop(&self) {
        let mut computer = self.computer.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        drop(computer.take());
    }

    /// What the screen shows now, to tell later whether it changed.
    pub(crate) fn glance(&self) -> io::Result<Vec<u8>> {
        self.with(|computer| computer.screen().points()).map(|(_, _, points)| points)
    }

    /// A picture of the screen once it has stopped changing for `calm`
    /// looks in a row, half a second apart, or after `patience` at most,
    /// kept as a file. A blinking cursor is not a change, and a screen all
    /// black is not a settled one. With `before`, a
    /// screen still as it was then is not taken for a settled one: what
    /// was asked for has not shown yet.
    pub(crate) async fn picture(
        &self,
        patience: Duration,
        calm: u32,
        before: Option<&[u8]>,
    ) -> io::Result<PathBuf> {
        let differing = |one: &[u8], other: &[u8]| {
            one.len() != other.len()
                || one.iter().zip(other).filter(|(new, old)| new != old).count() > CURSOR_SIZED
        };
        let until = Instant::now() + patience;
        let mut seen = self.with(|computer| computer.screen().points())?;
        let mut still = 0;
        while still < calm && Instant::now() < until {
            tokio::time::sleep(Duration::from_millis(500)).await;
            let now = self.with(|computer| computer.screen().points())?;
            let moved = differing(&now.2, &seen.2);
            let unchanged_since_before = before.is_some_and(|before| !differing(&now.2, before));
            // All black is a browser between two paintings, not a page.
            let blank = now.2.iter().all(|light| *light == 0);
            still = if moved || unchanged_since_before || blank { 0 } else { still + 1 };
            seen = now;
        }
        // Whole for the Owner, and smaller for the model: a picture costs a
        // model about as many tokens as it has points.
        let small = crate::png::reduced(seen.0, seen.1, &seen.2, SHOWN.0.into(), SHOWN.1.into());
        let small = crate::png::encode(SHOWN.0.into(), SHOWN.1.into(), &small);
        let seen = crate::png::encode(seen.0, seen.1, &seen.2);
        let number = self.taken.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let pictures = self.folder.join("pictures");
        std::fs::create_dir_all(&pictures)?;
        let file = pictures.join(format!("screen-{number}.png"));
        std::fs::write(&file, seen)?;
        std::fs::write(shown_to_the_model(&file), small)?;
        if let Some(old) = number.checked_sub(PICTURES_KEPT) {
            let old = pictures.join(format!("screen-{old}.png"));
            let _ = std::fs::remove_file(shown_to_the_model(&old));
            let _ = std::fs::remove_file(old);
        }
        Ok(file)
    }
}

/// Where the programs a Computer started are noted, beside its profile.
fn running_file(profile: &Path) -> PathBuf {
    profile.with_extension("running")
}

/// Notes a line about what runs for the Computer of `profile`: who uses it,
/// where its screen's key is, and each program it started.
fn note(profile: &Path, line: &str) {
    use std::io::Write;
    if let Ok(mut noted) =
        std::fs::OpenOptions::new().create(true).append(true).open(running_file(profile))
    {
        let _ = writeln!(noted, "{line}");
    }
}

/// When the program `pid` started, as the system counts it: with its
/// number, what tells it from any later program given the same number.
fn started_at(pid: u32) -> String {
    std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .ok()
        .and_then(|stat| {
            stat.rsplit(')')
                .next()
                .and_then(|after| after.split_whitespace().nth(19).map(str::to_owned))
        })
        .unwrap_or_default()
}

/// Stops the screen and browser an earlier Computer of `profile` left
/// running, and removes its screen's key. Fails when the Orbite that noted
/// them is still running: its Computer is in use, not left behind. A number
/// noted then may be another program's by now: only a screen server, or a
/// program started on this very profile, is stopped.
fn stop_what_was_left(profile: &Path) -> io::Result<()> {
    let noted = running_file(profile);
    let Ok(lines) = std::fs::read_to_string(&noted) else { return Ok(()) };
    for line in lines.lines() {
        if let Some(owner) = line.strip_prefix("owner ")
            && let Some((pid, started)) = owner.split_once(' ')
            && let Ok(pid) = pid.parse::<u32>()
            && pid != std::process::id()
            && !started.is_empty()
            && started_at(pid) == started
        {
            return Err(io::Error::other(
                "the agent's computer is in use by another Orbite running on this machine: stop that one first",
            ));
        }
    }
    let _ = std::fs::remove_file(&noted);
    let mine = profile.display().to_string();
    let mut stopped: Vec<i32> = Vec::new();
    for line in lines.lines() {
        if let Some(key) = line.strip_prefix("key ") {
            // Only what Orbite itself names that way is removed.
            if Path::new(key)
                .file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with("orbite-screen-"))
            {
                let _ = std::fs::remove_file(key);
            }
            continue;
        }
        let Ok(pid) = line.trim().parse::<i32>() else { continue };
        let Ok(command) = std::fs::read(format!("/proc/{pid}/cmdline")) else { continue };
        let command = String::from_utf8_lossy(&command).replace('\0', " ");
        if command.starts_with("Xvfb -displayfd") || command.contains(&mine) {
            // SAFETY: a signal sent to a program an earlier Orbite started.
            unsafe { libc::kill(pid, libc::SIGTERM) };
            stopped.push(pid);
        }
    }
    // The browser has to have let go of its profile before another takes
    // it: waited for, then made to.
    let gone = |pid: &i32| {
        std::fs::read(format!("/proc/{pid}/cmdline")).map_or(true, |command| command.is_empty())
    };
    let until = Instant::now() + Duration::from_secs(8);
    while !stopped.iter().all(gone) && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(100));
    }
    for pid in stopped.iter().filter(|pid| !gone(pid)) {
        // SAFETY: as above.
        unsafe { libc::kill(*pid, libc::SIGKILL) };
    }
    if !stopped.is_empty() {
        std::thread::sleep(Duration::from_millis(300));
        // What a browser stopped that way leaves to say its profile is taken.
        for lock in ["lock", ".parentlock", "SingletonLock", "SingletonCookie", "SingletonSocket"] {
            let _ = std::fs::remove_file(profile.join(lock));
        }
    }
    Ok(())
}
