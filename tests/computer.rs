//! The Agent's Computer, without a browser yet: a screen of its own that is
//! looked at and acted on as a person would. Each test starts its own
//! screen, and is skipped where no screen can be started.

use std::io::Read;
use std::time::Duration;

use orbite::Screen;
use x11rb::connection::Connection;
use x11rb::protocol::Event;
use x11rb::protocol::xproto::{ConnectionExt, CreateWindowAux, EventMask, WindowClass};

/// A screen for one test, or None where the machine cannot start one.
fn screen() -> Option<Screen> {
    match Screen::start(640, 400) {
        Ok(screen) => Some(screen),
        // Where a screen is promised, as in CI, its absence is a failure.
        Err(error) if std::env::var_os("ORBITE_TESTS_NEED_A_SCREEN").is_some() => {
            panic!("no screen can be started here: {error}")
        }
        Err(error) => {
            eprintln!("skipped: no screen can be started here ({error})");
            None
        }
    }
}

/// The colour of one point of a picture written by Orbite: (red, green, blue).
fn point(png: &[u8], x: usize, y: usize) -> (u8, u8, u8) {
    let width = u32::from_be_bytes(png[16..20].try_into().unwrap()) as usize;
    // The picture's data: every IDAT part, one after the other.
    let (mut at, mut packed) = (8, Vec::new());
    while at < png.len() {
        let length = u32::from_be_bytes(png[at..at + 4].try_into().unwrap()) as usize;
        if &png[at + 4..at + 8] == b"IDAT" {
            packed.extend_from_slice(&png[at + 8..at + 8 + length]);
        }
        at += 12 + length;
    }
    let mut rows = Vec::new();
    flate2::read::ZlibDecoder::new(&packed[..]).read_to_end(&mut rows).unwrap();
    let start = y * (1 + width * 3) + 1 + x * 3;
    (rows[start], rows[start + 1], rows[start + 2])
}

/// A plain red window covering the left half of the screen, which reports
/// the keys it is sent.
struct Window {
    connection: x11rb::rust_connection::RustConnection,
}

impl Window {
    fn on(screen: &Screen) -> Self {
        let (connection, number) = x11rb::connect(Some(screen.display())).unwrap();
        let root = &connection.setup().roots[number];
        let window = connection.generate_id().unwrap();
        let look = CreateWindowAux::new()
            .background_pixel(0x00ff_0000)
            .event_mask(EventMask::KEY_PRESS | EventMask::BUTTON_PRESS);
        connection
            .create_window(
                24,
                window,
                root.root,
                0,
                0,
                320,
                400,
                0,
                WindowClass::INPUT_OUTPUT,
                root.root_visual,
                &look,
            )
            .unwrap();
        connection.map_window(window).unwrap();
        connection.flush().unwrap();
        std::thread::sleep(Duration::from_millis(200));
        Self { connection }
    }

    /// What reached the window within a moment: keys pressed, buttons pressed.
    fn received(&self) -> (usize, Vec<(i16, i16)>) {
        std::thread::sleep(Duration::from_millis(400));
        let (mut keys, mut clicks) = (0, Vec::new());
        while let Some(event) = self.connection.poll_for_event().unwrap() {
            match event {
                Event::KeyPress(_) => keys += 1,
                Event::ButtonPress(press) => clicks.push((press.event_x, press.event_y)),
                _ => {}
            }
        }
        (keys, clicks)
    }
}

#[test]
fn the_screen_is_seen_as_a_picture_of_its_size() {
    let Some(screen) = screen() else { return };

    let picture = screen.picture().unwrap();

    assert_eq!(&picture[..8], &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]);
    assert_eq!(u32::from_be_bytes(picture[16..20].try_into().unwrap()), 640);
    assert_eq!(u32::from_be_bytes(picture[20..24].try_into().unwrap()), 400);
}

#[test]
fn what_is_on_the_screen_is_in_the_picture() {
    let Some(screen) = screen() else { return };
    let _window = Window::on(&screen);

    let picture = screen.picture().unwrap();

    assert_eq!(point(&picture, 100, 200), (255, 0, 0), "the window is red");
    assert_ne!(point(&picture, 500, 200), (255, 0, 0), "beside the window is not");
}

#[test]
fn a_click_lands_where_it_was_aimed() {
    let Some(screen) = screen() else { return };
    let window = Window::on(&screen);

    screen.click(120, 80).unwrap();

    assert_eq!(window.received().1, [(120, 80)]);
}

#[test]
fn what_is_typed_reaches_the_window_under_the_pointer() {
    let Some(screen) = screen() else { return };
    let window = Window::on(&screen);
    screen.click(120, 80).unwrap();
    window.received();

    screen.type_text("hello").unwrap();
    screen.key("Return").unwrap();

    assert_eq!(window.received().0, 6, "five letters and Return");
}

#[test]
fn two_screens_do_not_see_each_other() {
    let (Some(first), Some(second)) = (screen(), screen()) else { return };
    let _window = Window::on(&first);

    assert_ne!(first.display(), second.display());
    assert_eq!(point(&first.picture().unwrap(), 100, 200), (255, 0, 0));
    assert_ne!(point(&second.picture().unwrap(), 100, 200), (255, 0, 0));
}

mod browser {
    use std::time::{Duration, Instant};

    use orbite::Computer;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::point;

    /// A page that is red, turns green once "hello" was typed in its field,
    /// and is blue from the start when that was typed on an earlier visit.
    const PAGE: &str = r#"<!doctype html><html><body style="margin:0;background:rgb(255,0,0)">
<input id="field" autofocus style="position:absolute;left:40px;top:40px;width:300px;height:40px;font-size:24px">
<script>
  if (localStorage.getItem("typed") === "hello") document.body.style.background = "rgb(0,0,255)";
  document.getElementById("field").addEventListener("input", event => {
    if (event.target.value === "hello") {
      localStorage.setItem("typed", "hello");
      document.body.style.background = "rgb(0,128,0)";
    }
  });
</script></body></html>"#;

    /// A Computer for one test, or None where no screen or browser is to be had.
    fn started(profile: &std::path::Path) -> Option<Computer> {
        match Computer::start(profile, 800, 600) {
            Ok(computer) => Some(computer),
            Err(error) if std::env::var_os("ORBITE_TESTS_NEED_A_SCREEN").is_some() => {
                panic!("no Computer can be started here: {error}")
            }
            Err(error) => {
                eprintln!("skipped: no Computer can be started here ({error})");
                None
            }
        }
    }

    /// Waits until the page, well inside the window, is of `colour`.
    fn becomes(computer: &Computer, colour: (u8, u8, u8)) -> bool {
        let until = Instant::now() + Duration::from_secs(40);
        while Instant::now() < until {
            if computer.screen().picture().is_ok_and(|picture| point(&picture, 600, 450) == colour)
            {
                return true;
            }
            std::thread::sleep(Duration::from_millis(300));
        }
        // Kept for whoever looks into the failure.
        if let (Some(folder), Ok(picture)) =
            (std::env::var_os("ORBITE_TEST_PICTURES"), computer.screen().picture())
        {
            let _ = std::fs::create_dir_all(&folder);
            let _ = std::fs::write(
                std::path::Path::new(&folder).join(format!("expected-{colour:?}.png")),
                picture,
            );
        }
        false
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_page_is_opened_typed_in_and_remembered_by_the_browser() {
        let site = MockServer::start().await;
        Mock::given(wiremock::matchers::method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(PAGE, "text/html"))
            .mount(&site)
            .await;
        let address = format!("http://127.0.0.1:{}/", site.address().port());
        let home = tempfile::tempdir().unwrap();
        let profile = home.path().join("profile");

        let Some(mut computer) = started(&profile) else { return };
        computer.open(&address).unwrap();
        assert!(becomes(&computer, (255, 0, 0)), "the page did not show");

        // The field is clicked and typed in, as a person would.
        computer.screen().click(190, 60 + computer.page_top()).unwrap();
        computer.screen().type_text("hello").unwrap();
        assert!(becomes(&computer, (0, 128, 0)), "what was typed did not reach the page");

        // The browser is closed, and another day it still knows.
        computer.close_browser();
        drop(computer);
        let Some(mut computer) = started(&profile) else { return };
        computer.open(&address).unwrap();
        assert!(becomes(&computer, (0, 0, 255)), "the browser forgot what the site kept");
    }
}
