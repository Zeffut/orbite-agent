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
        let (connection, number) = screen.connect().unwrap();
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
    async fn a_computer_left_running_by_an_orbite_that_was_killed_does_not_keep_the_next_one_blind()
    {
        let site = MockServer::start().await;
        Mock::given(wiremock::matchers::method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(PAGE, "text/html"))
            .mount(&site)
            .await;
        let address = format!("http://127.0.0.1:{}/", site.address().port());
        let home = tempfile::tempdir().unwrap();
        let profile = home.path().join("profile");

        let Some(mut first) = started(&profile) else { return };
        first.open(&address).unwrap();
        assert!(becomes(&first, (255, 0, 0)), "the page did not show");
        // The programs it started, as it noted them.
        let left_behind: Vec<String> = std::fs::read_to_string(home.path().join("profile.running"))
            .unwrap()
            .lines()
            .filter(|line| line.parse::<u32>().is_ok())
            .map(str::to_owned)
            .collect();
        assert_eq!(left_behind.len(), 2, "{left_behind:?}");
        // Orbite is killed: nothing of it gets to tidy up.
        std::mem::forget(first);

        let Some(mut second) = started(&profile) else { return };
        second.open(&address).unwrap();

        assert!(
            becomes(&second, (255, 0, 0)),
            "the address went to the browser left behind, on a screen nobody looks at"
        );
        // What was left behind is gone: stopped, whether or not anything
        // has taken note of it yet.
        std::thread::sleep(Duration::from_millis(500));
        let running = |pid: &String| {
            std::fs::read(format!("/proc/{pid}/cmdline")).is_ok_and(|command| !command.is_empty())
        };
        let still: Vec<&String> = left_behind.iter().filter(|pid| running(pid)).collect();
        assert!(still.is_empty(), "left running: {still:?}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn letters_with_accents_and_other_alphabets_are_typed_as_they_are() {
        // A page that turns green once exactly this was typed in its field.
        let wanted = "Pélican à Zürich, naïve façade, 東京";
        let page = PAGE.replace(
            "event.target.value === \"hello\"",
            &format!("event.target.value === \"{wanted}\""),
        );
        let site = MockServer::start().await;
        Mock::given(wiremock::matchers::method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(page, "text/html; charset=utf-8"))
            .mount(&site)
            .await;
        let home = tempfile::tempdir().unwrap();
        let Some(mut computer) = started(&home.path().join("profile")) else { return };
        computer.open(&format!("http://127.0.0.1:{}/", site.address().port())).unwrap();
        assert!(becomes(&computer, (255, 0, 0)), "the page did not show");

        computer.screen().click(190, 60 + computer.page_top()).unwrap();
        computer.screen().type_text(wanted).unwrap();

        assert!(becomes(&computer, (0, 128, 0)), "what was typed is not what was meant");
    }

    #[test]
    fn a_computer_in_use_by_another_orbite_is_left_alone() {
        let home = tempfile::tempdir().unwrap();
        let profile = home.path().join("profile");
        // Another program, alive, noted as the one using this Computer.
        let mut other = std::process::Command::new("sleep").arg("30").spawn().unwrap();
        let stat = std::fs::read_to_string(format!("/proc/{}/stat", other.id())).unwrap();
        let started =
            stat.rsplit(')').next().unwrap().split_whitespace().nth(19).unwrap().to_owned();
        std::fs::write(
            home.path().join("profile.running"),
            format!("owner {} {started}\n{}\n", other.id(), other.id()),
        )
        .unwrap();

        let refused = Computer::start(&profile, 800, 600);

        let alive = matches!(other.try_wait(), Ok(None));
        let _ = other.kill();
        let _ = other.wait();
        let Err(why) = refused else { panic!("a Computer in use was taken over") };
        assert!(why.to_string().contains("in use"), "{why}");
        assert!(alive, "what the other Orbite was running was stopped");
    }

    #[test]
    fn nothing_is_left_of_a_computer_that_ended() {
        let home = tempfile::tempdir().unwrap();
        let Some(computer) = started(&home.path().join("profile")) else { return };
        let key = computer.screen().key_file().unwrap().to_owned();
        assert!(key.exists());
        // Killed: nothing of it tidies up. The next one does.
        std::mem::forget(computer);
        let Some(next) = started(&home.path().join("profile")) else { return };
        assert!(!key.exists(), "the key of the screen left behind is still there");
        drop(next);
        assert!(!home.path().join("profile.running").exists());
    }

    #[test]
    fn the_browser_is_not_given_what_orbite_was_started_with() {
        let home = tempfile::tempdir().unwrap();
        let profile = home.path().join("profile");
        let Some(mut computer) = started(&profile) else { return };
        computer.open("about:blank").unwrap();

        // Every program noted as started for this Computer, the screen aside.
        let noted = std::fs::read_to_string(profile.with_extension("running")).unwrap();
        let mut looked_at = 0;
        for pid in noted.lines().filter_map(|line| line.parse::<u32>().ok()) {
            let command = std::fs::read(format!("/proc/{pid}/cmdline")).unwrap_or_default();
            if command.starts_with(b"Xvfb") {
                continue;
            }
            let given = std::fs::read(format!("/proc/{pid}/environ")).unwrap();
            let given = String::from_utf8_lossy(&given);
            for variable in given.split('\0') {
                let name = variable.split('=').next().unwrap_or_default();
                // Settings and keys of Orbite, and the way to its Owner's session.
                assert!(!name.starts_with("ORBITE_"), "the browser was given {name}");
                assert_ne!(name, "DBUS_SESSION_BUS_ADDRESS");
            }
            looked_at += 1;
        }
        assert!(looked_at > 0, "no browser was noted: {noted}");
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

#[test]
fn the_command_says_whether_the_computer_works_on_this_machine() {
    if screen().is_none() {
        return;
    }
    let home = tempfile::tempdir().unwrap();

    let checked = std::process::Command::new(env!("CARGO_BIN_EXE_orbite"))
        .args(["computer", "check"])
        .env("ORBITE_HOME", home.path())
        .output()
        .unwrap();

    let said = format!(
        "{}{}",
        String::from_utf8_lossy(&checked.stdout),
        String::from_utf8_lossy(&checked.stderr)
    );
    assert!(checked.status.success(), "{said}");
    assert!(said.contains("works"), "{said}");
    // It leaves a picture of what it saw, and nothing running.
    let picture = std::fs::read(home.path().join("computer/check.png")).unwrap();
    assert_eq!(&picture[1..4], b"PNG");
}

#[test]
fn nobody_without_the_screens_own_key_can_look_at_it_or_act_on_it() {
    let Some(screen) = screen() else { return };

    // Another program of the machine, with whatever it has of its own.
    let stranger = x11rb::connect(Some(screen.display()));
    assert!(stranger.is_err(), "the screen let a stranger in");
    let acted = std::process::Command::new("xdotool")
        .args(["mousemove", "10", "10"])
        .env("DISPLAY", screen.display())
        .env_remove("XAUTHORITY")
        .output()
        .unwrap();
    assert!(!acted.status.success(), "a stranger moved the pointer");

    // Orbite itself still sees it and acts on it.
    assert!(screen.picture().is_ok());
    screen.click(20, 20).unwrap();
}

#[test]
fn the_screen_does_not_start_over_each_time_nothing_is_connected_to_it() {
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{AtomEnum, ConnectionExt, PropMode};
    use x11rb::wrapper::ConnectionExt as _;
    let Some(screen) = screen() else { return };

    // Something left on the screen by a program that then goes away, as a
    // browser's window is between two looks at it.
    {
        let (connection, number) = screen.connect().unwrap();
        let root = connection.setup().roots[number].root;
        connection
            .change_property8(PropMode::REPLACE, root, AtomEnum::WM_NAME, AtomEnum::STRING, b"kept")
            .unwrap()
            .check()
            .unwrap();
    }
    std::thread::sleep(std::time::Duration::from_millis(300));

    let (connection, number) = screen.connect().unwrap();
    let root = connection.setup().roots[number].root;
    let read = connection
        .get_property(false, root, AtomEnum::WM_NAME, AtomEnum::STRING, 0, 16)
        .unwrap()
        .reply()
        .unwrap();
    assert_eq!(read.value, b"kept", "the screen started over when its last program left");
}
