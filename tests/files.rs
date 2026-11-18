//! What an Owner observes when files and images go back and forth.

mod support;

use std::fs;
use std::path::Path;

use orbite::{Agent, Message, ProviderError, Store, attached};
use serde_json::json;
use support::{Owner, ScriptedProvider, memory_channel};

/// The smallest PNG there is: one transparent pixel.
const PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 0x0d, 0x49, 0x48, 0x44, 0x52, 0, 0, 0,
    1, 0, 0, 0, 1, 8, 6, 0, 0, 0, 0x1f, 0x15, 0xc4, 0x89, 0, 0, 0, 0x0d, 0x49, 0x44, 0x41, 0x54,
    0x78, 0x9c, 0x62, 0, 1, 0, 0, 5, 0, 1, 0x0d, 0x0a, 0x2d, 0xb4, 0, 0, 0, 0, 0x49, 0x45, 0x4e,
    0x44, 0xae, 0x42, 0x60, 0x82,
];

fn agent_in(workspace: &Path, provider: &ScriptedProvider) -> Owner {
    let (mut channel, owner) = memory_channel();
    let provider = provider.clone();
    let workspace = workspace.to_owned();
    tokio::spawn(async move {
        Agent::new(provider, Store::in_memory()).working_in(workspace).converse(&mut channel).await
    });
    owner
}

/// What a Channel hands the Agent when the Owner sends `text` with `file`.
fn sent_with(text: &str, file: &Path) -> String {
    format!("{text}\n{}", attached(file))
}

#[tokio::test]
async fn an_image_the_owner_sends_is_shown_to_the_model() {
    let workspace = tempfile::tempdir().unwrap();
    let photo = workspace.path().join("inbox/photo.png");
    fs::create_dir_all(photo.parent().unwrap()).unwrap();
    fs::write(&photo, PNG).unwrap();
    let provider = ScriptedProvider::answering(["A single pixel."]);
    let mut owner = agent_in(workspace.path(), &provider);

    owner.says(&sent_with("What is this?", &photo));

    assert_eq!(owner.hears().await, "A single pixel.");
    assert_eq!(provider.last_images(), [(0, photo.clone())]);
    let [Message::Owner(text)] = &provider.last_asked()[..] else { panic!() };
    assert!(text.contains("What is this?") && text.contains(photo.to_str().unwrap()), "{text}");
}

#[tokio::test]
async fn a_document_the_owner_sends_is_named_to_the_model_which_can_read_it() {
    let workspace = tempfile::tempdir().unwrap();
    let report = workspace.path().join("inbox/report.txt");
    fs::create_dir_all(report.parent().unwrap()).unwrap();
    fs::write(&report, "Sales rose by 4%.").unwrap();
    let provider = ScriptedProvider::default();
    provider
        .then_acts([("read_file", json!({ "path": report.to_str().unwrap() }))])
        .then(Ok("Sales rose by 4%.".into()));
    let mut owner = agent_in(workspace.path(), &provider);

    owner.says(&sent_with("Summarise this.", &report));

    assert_eq!(owner.hears().await, "Sales rose by 4%.");
    assert!(provider.last_images().is_empty(), "a text file is not an image");
    assert!(owner.was_asked_nothing());
}

#[tokio::test]
async fn an_image_is_no_longer_shown_once_the_conversation_has_moved_on() {
    let workspace = tempfile::tempdir().unwrap();
    let photo = workspace.path().join("photo.png");
    fs::write(&photo, PNG).unwrap();
    let provider = ScriptedProvider::answering(["A pixel.", "1", "2", "3", "4"]);
    let mut owner = agent_in(workspace.path(), &provider);
    owner.says(&sent_with("What is this?", &photo));
    owner.hears().await;
    owner.says("And its colour?");
    owner.hears().await;
    assert_eq!(provider.last_images().len(), 1, "still shown just after");

    for question in ["a", "b", "c"] {
        owner.says(question);
        owner.hears().await;
    }

    assert!(provider.last_images().is_empty(), "an image is costly: it is not resent for ever");
    let Message::Owner(text) = &provider.last_asked()[0] else { panic!() };
    assert!(text.contains(photo.to_str().unwrap()), "its path stays, so it can be looked at again");
}

#[tokio::test]
async fn what_is_not_there_or_too_big_is_not_shown() {
    let workspace = tempfile::tempdir().unwrap();
    let huge = workspace.path().join("huge.png");
    fs::write(&huge, vec![0u8; 6_000_000]).unwrap();
    let provider = ScriptedProvider::answering(["Hm."]);
    let mut owner = agent_in(workspace.path(), &provider);

    owner.says(&format!(
        "Look.\n{}\n{}",
        attached(&workspace.path().join("gone.png")),
        attached(&huge)
    ));
    owner.hears().await;

    assert!(provider.last_images().is_empty());
}

#[tokio::test]
async fn a_model_that_cannot_see_is_told_so_and_the_owner_still_gets_an_answer() {
    let workspace = tempfile::tempdir().unwrap();
    let photo = workspace.path().join("photo.png");
    fs::write(&photo, PNG).unwrap();
    let provider = ScriptedProvider::default();
    provider
        .then(Err(ProviderError::Failed("HTTP 400: image input is not supported".into())))
        .then(Ok("I cannot see images with this model.".into()))
        .then(Ok("Still blind.".into()));
    let mut owner = agent_in(workspace.path(), &provider);

    owner.says(&sent_with("What is this?", &photo));

    assert_eq!(owner.hears().await, "I cannot see images with this model.");
    assert!(provider.last_images().is_empty());
    let Some(Message::Owner(text)) = provider.last_asked().pop() else { panic!() };
    assert!(text.contains("cannot see images"), "the model is told why it sees nothing: {text}");

    // It is not tried again at every message.
    owner.says(&sent_with("And this one?", &photo));
    assert_eq!(owner.hears().await, "Still blind.");
    assert_eq!(provider.times_asked(), 3);
}

#[tokio::test]
async fn the_agent_sends_a_file_to_its_owner() {
    let workspace = tempfile::tempdir().unwrap();
    fs::write(workspace.path().join("summary.txt"), "All good.").unwrap();
    let provider = ScriptedProvider::default();
    provider
        .then_acts([("send_file", json!({ "path": "summary.txt", "caption": "Here it is." }))])
        .then(Ok("Sent.".into()));
    let mut owner = agent_in(workspace.path(), &provider);

    owner.says("Send me the summary.");

    assert_eq!(owner.hears().await, "(file summary.txt: All good.) Here it is.");
    assert_eq!(owner.hears().await, "Sent.");
    assert!(owner.was_asked_nothing(), "sending the Owner their own file needs no Approval");
}

#[tokio::test]
async fn sending_a_file_that_does_not_exist_or_is_secret_is_handled() {
    let workspace = tempfile::tempdir().unwrap();
    let provider = ScriptedProvider::default();
    provider.then_acts([("send_file", json!({ "path": "nothing.txt" }))]).then(Ok("Hm.".into()));
    let mut owner = agent_in(workspace.path(), &provider);

    owner.says("Send it.");
    assert_eq!(owner.hears().await, "Hm.");

    let Some(Message::Outcome { text, .. }) = provider.last_asked().pop() else { panic!() };
    assert!(text.contains("nothing.txt"), "{text}");
}
