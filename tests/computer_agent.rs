//! What an Owner observes of the Agent using its Computer: it asks before
//! going to a site, acts there as a person would, and the model is shown
//! the screen.

mod support;

use std::io::Read;
use std::path::Path;
use std::time::Duration;

use orbite::{Agent, Message, Secrets, Store};
use serde_json::json;
use support::{Owner, ScriptedProvider, memory_channel};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// A page that is red, and turns green once "tulip-42" was typed in its field.
const PAGE: &str = r#"<!doctype html><html><body style="margin:0;background:rgb(255,0,0)">
<input id="field" autofocus style="position:absolute;left:40px;top:40px;width:300px;height:40px;font-size:24px">
<script>
  document.getElementById("field").addEventListener("input", event => {
    if (event.target.value === "tulip-42") document.body.style.background = "rgb(0,128,0)";
  });
</script></body></html>"#;

/// True where a screen and a browser can be started; panics where they were promised.
fn has_a_computer() -> bool {
    let profile = tempfile::tempdir().unwrap();
    match orbite::Computer::start(profile.path(), 320, 200) {
        Ok(_) => true,
        Err(error) if std::env::var_os("ORBITE_TESTS_NEED_A_SCREEN").is_some() => {
            panic!("no Computer can be started here: {error}")
        }
        Err(error) => {
            eprintln!("skipped: no Computer can be started here ({error})");
            false
        }
    }
}

async fn site() -> (MockServer, String) {
    let site = MockServer::start().await;
    Mock::given(wiremock::matchers::method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(PAGE, "text/html"))
        .mount(&site)
        .await;
    let address = format!("http://127.0.0.1:{}/", site.address().port());
    (site, address)
}

fn agent_in(home: &Path, provider: &ScriptedProvider) -> Owner {
    let (mut channel, owner) = memory_channel();
    let provider = provider.clone();
    let home = home.to_owned();
    tokio::spawn(async move {
        Agent::new(provider, Store::in_memory())
            .keeping_secrets_in(&home)
            .using_a_computer_in(home.join("computer"))
            .converse(&mut channel)
            .await
    });
    owner
}

/// The colour of one point of a picture written by Orbite.
fn point(file: &Path, x: usize, y: usize) -> (u8, u8, u8) {
    let png = std::fs::read(file).unwrap();
    let width = u32::from_be_bytes(png[16..20].try_into().unwrap()) as usize;
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

/// What the Owner is asked next, waiting as long as a browser takes to start.
async fn asked(owner: &mut Owner) -> orbite::Approval {
    tokio::time::timeout(Duration::from_secs(60), owner.is_asked_whenever())
        .await
        .expect("nothing was asked")
}

async fn heard(owner: &mut Owner) -> String {
    tokio::time::timeout(Duration::from_secs(90), owner.hears_whenever())
        .await
        .expect("the Agent stayed silent")
}

#[tokio::test(flavor = "multi_thread")]
async fn the_agent_goes_to_a_site_once_allowed_acts_there_and_the_model_sees_the_screen() {
    if !has_a_computer() {
        return;
    }
    let (_site, address) = site().await;
    let home = tempfile::tempdir().unwrap();
    let provider = ScriptedProvider::default();
    provider
        .then_acts([("computer_open", json!({ "url": address, "purpose": "fill in the form" }))])
        // As the model is offered it: one Action, done in several ways.
        .then_acts([("computer", json!({ "do": "click", "x": 190, "y": 147 }))])
        .then_acts([("computer", json!({ "do": "type", "text": "tulip-42" }))])
        .then(Ok("The form is filled in.".into()));
    let mut owner = agent_in(home.path(), &provider);

    owner.says("Fill in the form on that site.");
    let approval = asked(&mut owner).await;
    assert!(
        approval.detail.contains("127.0.0.1") && approval.detail.contains("fill in the form"),
        "{approval:?}"
    );
    owner.allows();
    assert_eq!(heard(&mut owner).await, "The form is filled in.");

    let offered = provider.last_offered();
    assert!(
        offered.contains(&"computer".to_owned()) && !offered.contains(&"computer_click".to_owned()),
        "{offered:?}"
    );
    // Clicking and typing on the site that was allowed asked nothing more.
    assert!(owner.was_asked_nothing());
    // The model was shown the screen as it was after typing: the page turned green.
    let shown = provider.last_images();
    let (_, picture) = shown.last().expect("the model was shown no picture");
    assert_eq!(point(picture, 900, 600), (0, 128, 0));
    // Only the latest pictures are sent again, however many steps there were.
    assert!(shown.len() <= 3, "{} pictures were sent at once", shown.len());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_secret_is_typed_by_name_after_asking_and_its_value_never_reaches_the_model() {
    if !has_a_computer() {
        return;
    }
    let (_site, address) = site().await;
    let home = tempfile::tempdir().unwrap();
    Secrets::kept_in(home.path()).set("FORM_CODE", "tulip-42").unwrap();
    let provider = ScriptedProvider::default();
    provider
        .then_acts([("computer_open", json!({ "url": address, "purpose": "enter the code" }))])
        .then_acts([("computer_click", json!({ "x": 190, "y": 147 }))])
        .then_acts([("computer_type", json!({ "text": "{{FORM_CODE}}" }))])
        .then(Ok("Entered.".into()));
    let mut owner = agent_in(home.path(), &provider);

    owner.says("Enter my code on that site.");
    asked(&mut owner).await;
    owner.allows();
    let typing = asked(&mut owner).await;
    assert!(typing.critical && typing.detail.contains("FORM_CODE"), "{typing:?}");
    owner.allows();
    assert_eq!(heard(&mut owner).await, "Entered.");

    let (_, picture) = provider.last_images().pop().expect("the model was shown no picture");
    assert_eq!(point(&picture, 900, 600), (0, 128, 0), "the secret was not typed");
    let told = provider.last_asked();
    let leaked = told.iter().any(|message| match message {
        Message::Outcome { text, .. } | Message::Notice(text) => text.contains("tulip-42"),
        _ => false,
    });
    assert!(!leaked, "the secret's value was told to the model in words");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_owner_is_sent_a_picture_of_the_screen_when_they_ask_for_one() {
    if !has_a_computer() {
        return;
    }
    let (_site, address) = site().await;
    let home = tempfile::tempdir().unwrap();
    let picture = home.path().join("computer/pictures/screen-0.png");
    let provider = ScriptedProvider::default();
    provider
        .then_acts([(
            "computer",
            json!({ "do": "open", "url": address, "purpose": "show the page" }),
        )])
        .then_acts([("send_file", json!({ "path": picture, "caption": "The page." }))])
        .then(Ok("Sent.".into()));
    let mut owner = agent_in(home.path(), &provider);

    owner.says("Open that site and send me a picture of it.");
    asked(&mut owner).await;
    owner.allows();
    let sent = heard(&mut owner).await;

    let outcomes: Vec<String> = provider
        .every_call()
        .into_iter()
        .flat_map(|(_, asked)| asked)
        .filter_map(|message| match message {
            Message::Outcome { text, .. } => Some(text),
            _ => None,
        })
        .collect();
    assert!(
        sent.starts_with("(file screen-0.png: ") && sent.ends_with("The page."),
        "{sent}\nwhat the model was told: {outcomes:?}"
    );
    // What was pictured is the page, not the screen before the browser showed itself.
    assert_eq!(point(&picture, 900, 600), (255, 0, 0));
    assert_eq!(heard(&mut owner).await, "Sent.");
    // The model was told where the picture is, so that it can send it.
    let told = provider.every_call().into_iter().flat_map(|(_, asked)| asked).any(|message| {
        matches!(message, Message::Outcome { text, .. } if text.contains(picture.to_str().unwrap()))
    });
    assert!(told, "the model was never told where the picture of the screen is");
}

/// True while a program of that number runs.
fn alive(pid: &str) -> bool {
    std::fs::read(format!("/proc/{pid}/cmdline")).is_ok_and(|command| !command.is_empty())
}

#[tokio::test(flavor = "multi_thread")]
async fn stopping_orbite_closes_its_browser_and_its_screen() {
    if !has_a_computer() {
        return;
    }
    let (_site, address) = site().await;
    // A Provider whose model opens the site, then takes its time.
    let provider = MockServer::start().await;
    let opening = json!({ "choices": [{ "message": { "role": "assistant", "content": null, "tool_calls": [{
        "id": "call-1", "type": "function",
        "function": { "name": "computer", "arguments": json!({ "do": "open", "url": address, "purpose": "look" }).to_string() },
    }] } }] });
    Mock::given(wiremock::matchers::method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(opening))
        .up_to_n_times(1)
        .mount(&provider)
        .await;
    Mock::given(wiremock::matchers::method("POST"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(120)))
        .mount(&provider)
        .await;
    let home = tempfile::tempdir().unwrap();
    let mut orbite = tokio::process::Command::new(env!("CARGO_BIN_EXE_orbite"))
        .arg("chat")
        .env("ORBITE_HOME", home.path())
        .env("ORBITE_PROVIDER", "custom")
        .env("ORBITE_BASE_URL", provider.uri())
        .env("ORBITE_MODEL", "pretend")
        .env("ORBITE_API_KEY", "none")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    use tokio::io::AsyncWriteExt;
    let mut typed = orbite.stdin.take().unwrap();
    // The request, then yes to going to the site.
    typed.write_all(b"Open the site.\ny\n").await.unwrap();
    typed.flush().await.unwrap();

    // Its screen and its browser are running.
    let noted = home.path().join("computer/profile.running");
    let mut running: Vec<String> = Vec::new();
    for _ in 0..120 {
        running = std::fs::read_to_string(&noted)
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect();
        if running.len() >= 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert_eq!(running.len(), 2, "the Computer did not start: {running:?}");
    assert!(running.iter().all(|pid| alive(pid)));

    // Orbite is asked to stop, as its service does.
    // SAFETY: a signal sent to the program this test started.
    unsafe { libc::kill(orbite.id().unwrap() as libc::pid_t, libc::SIGTERM) };
    let ended = tokio::time::timeout(Duration::from_secs(20), orbite.wait()).await;
    assert!(ended.is_ok(), "Orbite did not stop when asked to");

    for _ in 0..40 {
        if !running.iter().any(|pid| alive(pid)) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    let left: Vec<&String> = running.iter().filter(|pid| alive(pid)).collect();
    // What is left is stopped here, whatever the verdict, so that nothing outlives the test.
    for pid in &left {
        // SAFETY: as above.
        unsafe { libc::kill(pid.parse().unwrap(), libc::SIGKILL) };
    }
    assert!(left.is_empty(), "left running after Orbite stopped: {left:?}");
    drop(typed);
}
