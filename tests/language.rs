//! Orbite's own words in the Owner's language. What the model says is the
//! model's business; this is what Orbite itself shows and says.

mod support;

use std::time::Duration;

use orbite::{Agent, Approval, Channel, Language, ProviderError, Step, Store, Telegram};
use serde_json::{Value, json};
use support::{ScriptedProvider, memory_channel};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn french_agent(provider: &ScriptedProvider, workspace: &std::path::Path) -> support::Owner {
    let (mut channel, owner) = memory_channel();
    let provider = provider.clone();
    let workspace = workspace.to_owned();
    tokio::spawn(async move {
        Agent::new(provider, Store::in_memory())
            .working_in(workspace)
            .speaking(Language::French)
            .converse(&mut channel)
            .await
    });
    owner
}

#[tokio::test]
async fn what_orbite_says_itself_is_in_the_owners_language() {
    let workspace = tempfile::tempdir().unwrap();
    let provider = ScriptedProvider::default();
    provider.then(Err(ProviderError::Unreachable("connection refused".into())));
    let mut owner = french_agent(&provider, workspace.path());

    owner.says("Bonjour");
    let explained = owner.hears().await;
    assert!(
        explained.contains("Je n'ai pas pu joindre") && explained.contains("connection refused"),
        "{explained}"
    );

    owner.says("/reset");
    assert!(owner.hears().await.contains("Nouveau départ"));

    owner.says(&format!("voici ma clé sk-{}-AbCdEfGhIjKlMnOpQrStUvWxYz0123456789", "proj"));
    assert!(owner.hears().await.contains("ressemble à un secret"));
}

#[tokio::test]
async fn approvals_and_steps_are_in_the_owners_language() {
    let workspace = tempfile::tempdir().unwrap();
    std::fs::write(workspace.path().join("notes.txt"), "lait").unwrap();
    let provider = ScriptedProvider::default();
    provider
        .then_acts([("read_file", json!({ "path": "notes.txt" }))])
        .then_acts([("run_command", json!({ "command": "touch fait.txt" }))])
        .then(Ok("Fait.".into()));
    let mut owner = french_agent(&provider, workspace.path());

    owner.says("Vas-y.");
    let approval = owner.is_asked().await;
    assert_eq!(approval.summary, "Lancer une commande");
    owner.says("stop");
    assert_eq!(owner.hears().await, "Arrêté.");

    assert_eq!(owner.saw()[0], "📄 Lecture de notes.txt");
}

/// What the bot showed, in order: (method, body).
async fn shown(server: &MockServer) -> Vec<(String, Value)> {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|request| {
            let called = request.url.path().rsplit('/').next().unwrap().to_string();
            (called, serde_json::from_slice(&request.body).unwrap_or(Value::Null))
        })
        .filter(|(called, _)| {
            ["sendMessage", "editMessageText", "setMyCommands"].contains(&called.as_str())
        })
        .collect()
}

#[tokio::test]
async fn telegram_shows_buttons_decisions_and_its_menu_in_the_owners_language() {
    let server = MockServer::start().await;
    let ok = |result: Value| {
        ResponseTemplate::new(200).set_body_json(json!({ "ok": true, "result": result }))
    };
    let tap = json!({
        "update_id": 100,
        "callback_query": {
            "id": "tap-1", "from": { "id": 42 },
            "message": { "message_id": 7, "chat": { "id": 42 } }, "data": "allow",
        }
    });
    Mock::given(path("/botTOKEN/getUpdates"))
        .respond_with(ok(json!([tap])))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(path("/botTOKEN/getUpdates"))
        .respond_with(ok(json!([])).set_delay(Duration::from_millis(20)))
        .mount(&server)
        .await;
    Mock::given(path("/botTOKEN/sendMessage"))
        .respond_with(ok(json!({ "message_id": 7 })))
        .mount(&server)
        .await;
    Mock::given(method("POST")).respond_with(ok(json!(true))).mount(&server).await;
    let mut channel = Telegram::new("TOKEN", 42).at(server.uri()).speaking(Language::French);

    channel.echo("depuis le bureau").await;
    channel.doing(&Step { icon: "💻", text: "Commande : ls".into() }).await;
    let approval = Approval {
        summary: "Lancer une commande".into(),
        detail: "rm x".into(),
        irreversible: true,
        ..Approval::default()
    };
    channel.ask(&approval).await.unwrap();

    let shown = shown(&server).await;
    let texts: Vec<String> = shown.iter().map(|(_, body)| body.to_string()).collect();
    let all = texts.join("\n");
    assert!(all.contains("Vous, ailleurs"), "{all}");
    assert!(
        all.contains("Autoriser") && all.contains("Toujours") && all.contains("Refuser"),
        "{all}"
    );
    assert!(all.contains("ne peut pas être annulé"), "{all}");
    assert!(all.contains("Autorisé"), "{all}");
    assert!(all.contains("Arrêter ce que fait l'agent"), "the menu: {all}");
}
