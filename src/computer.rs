//! The Agent's Computer: a screen of its own on the Host, looked at through
//! pictures and acted on with a mouse and a keyboard, as a person would.

use std::io::{self, BufRead, BufReader};
use std::process::{Child, Command, Stdio};

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
