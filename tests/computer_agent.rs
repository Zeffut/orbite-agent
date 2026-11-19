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
        .then_acts([("computer_click", json!({ "x": 190, "y": 147 }))])
        .then_acts([("computer_type", json!({ "text": "tulip-42" }))])
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
