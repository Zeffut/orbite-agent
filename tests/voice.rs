//! What an Owner observes when they speak to their Agent instead of writing:
//! what they said is written down by a Provider that does that, and answered.

mod support;

use orbite::{Agent, Message, Store, Transcriber};
use serde_json::json;
use support::{ScriptedProvider, memory_channel};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Built from parts: nothing here is a real key.
fn api_key() -> String {
    format!("key-{}", "of-the-provider-that-listens")
}

fn what_the_model_read(provider: &ScriptedProvider) -> String {
    match provider.last_asked().into_iter().next() {
        Some(Message::Owner(text)) => text,
        other => panic!("the model was not given the Owner's words but {other:?}"),
    }
}

#[tokio::test]
async fn a_voice_message_is_written_down_and_answered_as_what_the_owner_said() {
    let listening = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/audio/transcriptions"))
        .and(header("authorization", format!("Bearer {}", api_key()).as_str()))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({ "text": "Remind me to call the dentist tomorrow." })),
        )
        .expect(1)
        .mount(&listening)
        .await;
    let inbox = tempfile::tempdir().unwrap();
    // As Telegram names what was said aloud.
    let voice = inbox.path().join("voice-100.ogg");
    std::fs::write(&voice, b"OggS-not-much-of-a-recording").unwrap();
    let provider = ScriptedProvider::answering(["I will."]);
    let asked = provider.clone();
    let (mut channel, mut owner) = memory_channel();
    let transcriber = Transcriber::new(listening.uri(), api_key(), "model-that-listens");
    tokio::spawn(async move {
        Agent::new(provider, Store::in_memory())
            .transcribing_with(transcriber)
            .converse(&mut channel)
            .await
    });

    owner.says(&orbite::attached(&voice));
    assert_eq!(owner.hears().await, "I will.");

    let read = what_the_model_read(&asked);
    assert!(read.contains("Remind me to call the dentist tomorrow."), "{read}");
    // The model knows it was spoken: a word may have been misheard.
    assert!(read.contains("voice message"), "{read}");
    // What the Provider was sent: the recording, under a name it knows the kind of.
    let sent = listening.received_requests().await.unwrap();
    let body = String::from_utf8_lossy(&sent[0].body).into_owned();
    assert!(body.contains("model-that-listens"), "{body}");
    assert!(body.contains("filename=\"voice-100.ogg\""), "{body}");
    assert!(body.contains("OggS-not-much-of-a-recording"), "{body}");
}

#[tokio::test]
async fn without_a_provider_that_listens_the_agent_is_told_it_cannot_hear_the_message() {
    let inbox = tempfile::tempdir().unwrap();
    let voice = inbox.path().join("voice-100.ogg");
    std::fs::write(&voice, b"OggS").unwrap();
    let provider = ScriptedProvider::answering(["I cannot listen to that yet."]);
    let asked = provider.clone();
    let (mut channel, mut owner) = memory_channel();
    tokio::spawn(
        async move { Agent::new(provider, Store::in_memory()).converse(&mut channel).await },
    );

    owner.says(&orbite::attached(&voice));
    owner.hears().await;

    let read = what_the_model_read(&asked);
    assert!(read.contains("cannot hear"), "{read}");
    assert!(read.contains("orbite providers add"), "{read}");
}

#[tokio::test]
async fn a_recording_that_could_not_be_written_down_is_said_so_and_not_guessed() {
    let listening = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "error": { "message": "The audio could not be decoded." }
        })))
        .mount(&listening)
        .await;
    let inbox = tempfile::tempdir().unwrap();
    let voice = inbox.path().join("voice-100.oga");
    std::fs::write(&voice, b"OggS").unwrap();
    let provider = ScriptedProvider::answering(["Could you write it?"]);
    let asked = provider.clone();
    let (mut channel, mut owner) = memory_channel();
    let transcriber = Transcriber::new(listening.uri(), api_key(), "model-that-listens");
    tokio::spawn(async move {
        Agent::new(provider, Store::in_memory())
            .transcribing_with(transcriber)
            .converse(&mut channel)
            .await
    });

    owner.says(&format!("Listen to this.\n{}", orbite::attached(&voice)));
    owner.hears().await;

    let read = what_the_model_read(&asked);
    assert!(read.starts_with("Listen to this."), "{read}");
    assert!(read.contains("could not be written down"), "{read}");
    assert!(read.contains("The audio could not be decoded."), "{read}");
    // An .oga recording is sent under the name its kind is known by.
    let sent = listening.received_requests().await.unwrap();
    assert!(String::from_utf8_lossy(&sent[0].body).contains("filename=\"voice-100.ogg\""));
}
