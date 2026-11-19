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
