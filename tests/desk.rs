//! This computer: the machine's own desktop, lent to the Agent by the
//! account that opened it. A screen started here stands for that desktop.

mod support;

use std::process::Command;
use std::time::Duration;

use orbite::{Agent, NotOffered, Screen, Store, offer_this_computer};
use serde_json::json;
use support::{Owner, ScriptedProvider, memory_channel};

/// True where a screen can be started; panics where one was promised.
fn has_a_screen() -> bool {
    match Screen::start(320, 200) {
        Ok(_) => true,
        Err(error) if std::env::var_os("ORBITE_TESTS_NEED_A_SCREEN").is_some() => {
            panic!("no screen can be started here: {error}")
        }
        Err(error) => {
            eprintln!("skipped: no screen can be started here ({error})");
            false
        }
    }
}

/// The account these tests run as, by its number.
fn me() -> u32 {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata("/proc/self").unwrap().uid()
}

/// A name for the socket of one test, which no other test listens on.
fn socket(of: &str) -> String {
    format!("orbite-desk-test-{}-{of}", std::process::id())
}

async fn heard(owner: &mut Owner) -> String {
    tokio::time::timeout(Duration::from_secs(90), owner.hears_whenever())
        .await
        .expect("the Agent stayed silent")
}

/// The size of a PNG file, read from its first bytes.
fn size_of(picture: &std::path::Path) -> (u32, u32) {
    let bytes = std::fs::read(picture).unwrap();
    let number = |at: usize| u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap());
    (number(16), number(20))
}

/// Where the pointer is on `screen`.
fn pointer_on(screen: &Screen) -> String {
    let mut asking = Command::new("xdotool");
    asking.arg("getmouselocation").env("DISPLAY", screen.display());
    if let Some(key) = screen.key_file() {
        asking.env("XAUTHORITY", key);
    }
    String::from_utf8_lossy(&asking.output().unwrap().stdout).into_owned()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_desktop_offered_by_its_account_is_the_agents_computer_until_its_owner_says_otherwise() {
    if !has_a_screen() {
        return;
    }
    let home = std::sync::Arc::new(tempfile::tempdir().unwrap());
    // The desktop of the machine: wider than a Computer of the Agent's own.
    let desktop = std::sync::Arc::new(Screen::start(1000, 500).unwrap());
    let name = socket("lent");
    // Orbite's own page is served on this machine.
    orbite::Config { interface: Some("127.0.0.1:8377".into()), ..Default::default() }
        .save(home.path())
        .unwrap();
    let provider = ScriptedProvider::default();
    provider
        .then_acts([(
            "computer_open",
            json!({ "url": "http://127.0.0.1:8377/#ABCD-EFGH", "purpose": "look" }),
        )])
        .then(Ok("Refused.".into()))
        .then_acts([("computer", json!({ "do": "click", "x": 426, "y": 213 }))])
        .then(Ok("Clicked.".into()))
        .then_acts([("computer", json!({ "do": "look" }))])
        .then(Ok("Looked.".into()));
    let (mut channel, mut owner) = memory_channel();
    let (model, held, listening) = (provider.clone(), home.clone(), name.clone());
    let running = tokio::spawn(async move {
        let folder = held.path().to_owned();
        Agent::new(model, Store::in_memory())
            .working_in(folder.join("workspace"))
            .using_a_computer_in(folder.join("computer"))
            .lent_this_computer_by(me(), &listening)
            .converse(&mut channel)
            .await;
        drop(held);
    });
    // The helper, as it runs in the session of the account that opened the desktop.
    let (offered, offering) = (desktop.clone(), name.clone());
    let helping = std::thread::spawn(move || {
        // It waits for the Agent's service, as `orbite desk` does.
        while let Err(NotOffered::NobodyListening) =
            offer_this_computer(&offering, me(), "ada", "x11", &offered)
        {
            std::thread::sleep(Duration::from_millis(100));
        }
    });
    tokio::time::sleep(Duration::from_millis(1500)).await;

    // The page its Owner talks to it in is refused outright, on this
    // computer as on its own: not put to the Owner under a wider question.
    owner.says("Open our own page.");
    assert_eq!(heard(&mut owner).await, "Refused.");
    assert!(owner.was_asked_nothing());
    assert!(format!("{:?}", provider.last_asked()).contains("theirs alone to open"));

    owner.says("Click in the middle of the screen.");
    // On their own desktop a gesture can do whatever they can: taking its
    // mouse and keyboard is theirs to allow.
    let asked = tokio::time::timeout(Duration::from_secs(60), owner.is_asked_whenever())
        .await
        .expect("the desktop was taken without asking");
    assert!(asked.summary.contains("Use this computer"), "{}", asked.summary);
    assert!(asked.detail.contains("in your session"), "{}", asked.detail);
    // Asked every time it comes back to it, never allowed once and for all.
    assert!(asked.critical);
    owner.allows();
    assert_eq!(heard(&mut owner).await, "Clicked.");
    // The Agent knows whose desktop it is on, and acted on that one: the
    // point it aimed at on the picture is the middle of the real screen.
    let told = provider.last_instructions();
    assert!(told.contains("this machine's own desktop, opened by ada"), "{told}");
    assert!(pointer_on(&desktop).contains("x:499 y:249"), "{}", pointer_on(&desktop));
    // The picture it was shown keeps the desktop's shape.
    let pictures = home.path().join("computer/pictures");
    let shown = std::fs::read_dir(&pictures)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|file| file.to_string_lossy().ends_with(".shown.png"))
        .expect("no picture was kept");
    assert_eq!(size_of(&shown), (853, 427));

    // Told to keep to a computer of its own, it leaves the desktop alone.
    owner.says("/settings computer own");
    heard(&mut owner).await;
    owner.says("Look at your screen.");
    assert_eq!(heard(&mut owner).await, "Looked.");
    let told = provider.last_instructions();
    assert!(told.contains("Your Computer is your own"), "{told}");
    assert!(!told.contains("this machine's own desktop"), "{told}");

    // Once Orbite stops, the helper has nothing to serve and ends with it.
    drop(owner);
    tokio::time::timeout(Duration::from_secs(20), running).await.unwrap().unwrap();
    tokio::task::spawn_blocking(move || helping.join().unwrap()).await.unwrap();
}

#[test]
fn only_the_account_orbite_was_installed_by_may_offer_its_desktop() {
    if !has_a_screen() {
        return;
    }
    let desktop = Screen::start(320, 200).unwrap();
    let name = socket("refused");
    // Orbite here takes the desktop of another account than the one offering.
    let listening = std::os::unix::net::UnixListener::bind_addr(
        &<std::os::unix::net::SocketAddr as std::os::linux::net::SocketAddrExt>::from_abstract_name(&name)
            .unwrap(),
    );
    drop(listening);
    let lent = orbite::lending_this_computer(&name, me() + 1).unwrap();

    let refused = offer_this_computer(&name, me(), "mallory", "x11", &desktop);

    assert!(matches!(refused, Err(NotOffered::NotTheOwner)), "{refused:?}");
    assert!(!lent(), "nothing of it was kept");
    // And a helper never obeys what is not the Agent's own service.
    let stranger = offer_this_computer(&name, me() + 1, "ada", "x11", &desktop);
    assert!(matches!(stranger, Err(NotOffered::NotOrbite)), "{stranger:?}");
}

#[test]
fn a_wayland_session_is_told_it_cannot_be_lent_yet() {
    if !has_a_screen() {
        return;
    }
    let desktop = Screen::start(320, 200).unwrap();
    let name = socket("wayland");
    let lent = orbite::lending_this_computer(&name, me()).unwrap();

    let refused = offer_this_computer(&name, me(), "ada", "wayland", &desktop);

    assert!(matches!(refused, Err(NotOffered::Wayland)), "{refused:?}");
    assert!(!lent());
}
