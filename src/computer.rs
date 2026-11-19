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
}

impl Screen {
    /// Starts a screen of its own, `width` by `height`, that nothing else
    /// on the machine looks at; it ends with this value.
    pub fn start(width: u16, height: u16) -> io::Result<Self> {
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
            ])
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
            return Err(io::Error::other("the screen did not start"));
        }
        Ok(Self { display: format!(":{number}"), serving: Some(serving) })
    }

    /// A screen that is already there, by its X display name.
    pub fn at(display: impl Into<String>) -> Self {
        Self { display: display.into(), serving: None }
    }

    /// The screen's X display name, for programs to show themselves on it.
    pub fn display(&self) -> &str {
        &self.display
    }

    /// What the screen shows now, as a PNG file.
    pub fn picture(&self) -> io::Result<Vec<u8>> {
        let (connection, number) = x11rb::connect(Some(&self.display)).map_err(io::Error::other)?;
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
        Ok(crate::png::encode(width.into(), height.into(), &points))
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

    /// Types `text`, as on a keyboard.
    pub fn type_text(&self, text: &str) -> io::Result<()> {
        self.act(&["type", "--delay", "12", "--", text])
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
        let done = Command::new("xdotool")
            .args(gesture)
            .env("DISPLAY", &self.display)
            .env_remove("XAUTHORITY")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()?;
        if done.status.success() {
            Ok(())
        } else {
            Err(io::Error::other(String::from_utf8_lossy(&done.stderr).trim().to_owned()))
        }
    }
}

impl Drop for Screen {
    fn drop(&mut self) {
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
        Ok(Self {
            screen: Screen::start(width, height)?,
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
            .env("MOZ_ENABLE_WAYLAND", "0")
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
        if computer.is_none() {
            *computer = Some(Computer::start(self.folder.join("profile"), SCREEN.0, SCREEN.1)?);
        }
        match computer.as_mut() {
            Some(computer) => something(computer),
            None => Err(io::Error::other("the Computer did not start")),
        }
    }

    /// A picture of the screen once it has stopped changing for `calm`
    /// looks in a row, half a second apart, or after `patience` at most,
    /// kept as a file.
    pub(crate) async fn picture(&self, patience: Duration, calm: u32) -> io::Result<PathBuf> {
        let until = Instant::now() + patience;
        let mut seen = self.with(|computer| computer.screen().picture())?;
        let mut still = 0;
        while still < calm && Instant::now() < until {
            tokio::time::sleep(Duration::from_millis(500)).await;
            let now = self.with(|computer| computer.screen().picture())?;
            still = if now == seen { still + 1 } else { 0 };
            seen = now;
        }
        let number = self.taken.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let pictures = self.folder.join("pictures");
        std::fs::create_dir_all(&pictures)?;
        let file = pictures.join(format!("screen-{number}.png"));
        std::fs::write(&file, seen)?;
        if let Some(old) = number.checked_sub(PICTURES_KEPT) {
            let _ = std::fs::remove_file(pictures.join(format!("screen-{old}.png")));
        }
        Ok(file)
    }
}
