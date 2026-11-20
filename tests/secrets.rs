//! What an Owner observes of secrets: kept sealed on the Host, used by the
//! Agent, never shown to the model.

mod support;

use std::fs;
use std::path::Path;

use orbite::{Agent, Message, Secrets, Store};
use serde_json::json;
use support::{Owner, ScriptedProvider, memory_channel};

/// Looks like a token and is none.
const TOKEN: &str = "ghp_ExampleOnlyNotARealTokenValue0123456789";

fn agent_in(home: &Path, provider: &ScriptedProvider) -> Owner {
    let (mut channel, owner) = memory_channel();
    let provider = provider.clone();
    let home = home.to_owned();
    tokio::spawn(async move {
        Agent::new(provider, Store::in_memory())
            .working_in(home.join("workspace"))
            .keeping_secrets_in(&home)
            .converse(&mut channel)
            .await
    });
    owner
}

fn outcome(provider: &ScriptedProvider) -> String {
    match provider.last_asked().pop() {
        Some(Message::Outcome { text, .. }) => text,
        other => panic!("the model was not given an Outcome but {other:?}"),
    }
}

// --- The store ---

#[test]
fn a_secret_is_kept_and_given_back_and_survives_reopening() {
    let home = tempfile::tempdir().unwrap();
    Secrets::kept_in(home.path()).set("GITHUB_TOKEN", TOKEN).unwrap();

    let secrets = Secrets::kept_in(home.path());
    assert_eq!(secrets.get("GITHUB_TOKEN").unwrap().as_deref(), Some(TOKEN));
    assert_eq!(secrets.names().unwrap(), ["GITHUB_TOKEN"]);
    assert_eq!(secrets.get("OTHER").unwrap(), None);
}

#[test]
fn nothing_written_to_disk_shows_the_secret() {
    let home = tempfile::tempdir().unwrap();
    Secrets::kept_in(home.path()).set("GITHUB_TOKEN", TOKEN).unwrap();

    for entry in fs::read_dir(home.path()).unwrap() {
        let path = entry.unwrap().path();
        let written = fs::read(&path).unwrap();
        let shows = written.windows(TOKEN.len()).any(|window| window == TOKEN.as_bytes());
        assert!(!shows, "{} shows the secret", path.display());
    }
}

#[cfg(unix)]
#[test]
fn what_seals_the_secrets_is_readable_by_the_agents_account_only() {
    use std::os::unix::fs::PermissionsExt;
    let home = tempfile::tempdir().unwrap();
    Secrets::kept_in(home.path()).set("GITHUB_TOKEN", TOKEN).unwrap();

    for entry in fs::read_dir(home.path()).unwrap() {
        let path = entry.unwrap().path();
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "{} is readable by others", path.display());
    }
}

#[test]
fn a_secret_can_be_replaced_and_removed() {
    let home = tempfile::tempdir().unwrap();
    let secrets = Secrets::kept_in(home.path());
    secrets.set("GITHUB_TOKEN", "first-value-000000000000").unwrap();
    secrets.set("GITHUB_TOKEN", TOKEN).unwrap();
    assert_eq!(secrets.get("GITHUB_TOKEN").unwrap().as_deref(), Some(TOKEN));

    assert!(secrets.remove("GITHUB_TOKEN").unwrap());
    assert!(!secrets.remove("GITHUB_TOKEN").unwrap());
    assert!(secrets.names().unwrap().is_empty());
}

#[test]
fn a_secret_is_named_like_an_environment_variable() {
    let home = tempfile::tempdir().unwrap();
    let secrets = Secrets::kept_in(home.path());

    for bad in ["", "github token", "1TOKEN", "TOKEN-A", "../x"] {
        assert!(secrets.set(bad, "value").is_err(), "{bad:?} was accepted");
    }
    assert!(secrets.set("github_token", TOKEN).is_ok(), "names are put in capitals");
    assert_eq!(secrets.names().unwrap(), ["GITHUB_TOKEN"]);
}

#[test]
fn secrets_sealed_with_another_key_are_reported_as_unreadable_not_as_absent() {
    let home = tempfile::tempdir().unwrap();
    Secrets::kept_in(home.path()).set("GITHUB_TOKEN", TOKEN).unwrap();
    let key = fs::read_dir(home.path())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.to_string_lossy().ends_with(".key"))
        .expect("the sealing key is a file of its own");
    fs::write(&key, [7u8; 32]).unwrap();

    let failure = Secrets::kept_in(home.path()).get("GITHUB_TOKEN").unwrap_err();

    assert!(failure.to_string().contains("unseal"), "{failure}");
}

// --- In the Conversation ---

#[tokio::test]
async fn a_secret_pasted_in_the_conversation_is_neither_kept_nor_shown_to_the_model() {
    let home = tempfile::tempdir().unwrap();
    let provider = ScriptedProvider::answering(["Hello."]);
    let mut owner = agent_in(home.path(), &provider);

    owner.says(&format!("Here is my token: {TOKEN}"));
    let refused = owner.hears().await;

    assert!(refused.contains("/secret NAME"), "the Owner is told where secrets go: {refused}");
    assert!(!refused.contains(TOKEN));
    assert_eq!(provider.times_asked(), 0, "the model was shown the secret");
    assert!(
        owner.saw().contains(&"(erased what the Owner wrote)".to_string()),
        "it stays in the Channel"
    );

    owner.says("Hi");
    owner.hears().await;
    assert_eq!(
        provider.last_asked(),
        [Message::from_owner("Hi")],
        "the secret was kept in the Conversation"
    );
}

#[tokio::test]
async fn the_usual_kinds_of_secrets_are_recognised_and_ordinary_words_are_not() {
    let home = tempfile::tempdir().unwrap();
    // Put together from parts, so that nothing here is itself a secret of
    // the shape scanners look for.
    let tail = "AbCdEfGhIjKlMnOpQrStUvWxYz0123456789";
    for secret in [
        format!("sk-ant-{}-{tail}", "api03"),
        format!("sk-{}-{tail}", "proj"),
        format!("github_{}_11ABCDEFG0{tail}", "pat"),
        format!("xox{}-1234567890-1234567890123-{tail}", "b"),
        format!("AKIA{}", "IOSFODNN7EXAMPLE"),
        format!("AIza{}", "SyA-1234567890abcdefghijklmnopqrstuv"),
        format!("{}:{}", "1234567890", "AAExampleOnlyNotARealBotToken0123456"),
        format!(
            "-----BEGIN OPENSSH {} KEY-----\nb3BlbnNzaC1rZXktdjEAAAAA\n-----END OPENSSH {} KEY-----",
            "PRIVATE", "PRIVATE"
        ),
    ] {
        let provider = ScriptedProvider::default();
        let mut owner = agent_in(home.path(), &provider);
        owner.says(&format!("use this {secret} please"));
        assert!(owner.hears().await.contains("/secret NAME"), "{secret} was not recognised");
        assert_eq!(provider.times_asked(), 0);
    }
    for ordinary in [
        "Ask-me-anything about disk-usage and task-management, sk-wise.",
        "The commit is 3f2a9c1e8b7d6a5f4e3d2c1b0a9f8e7d6c5b4a39 on main.",
        "Call me at 10:30, room 4217:B.",
    ] {
        let provider = ScriptedProvider::answering(["Fine."]);
        let mut owner = agent_in(home.path(), &provider);
        owner.says(ordinary);
        assert_eq!(owner.hears().await, "Fine.", "{ordinary:?} was taken for a secret");
    }
}

#[tokio::test]
async fn an_owner_who_says_so_may_give_secrets_in_the_conversation() {
    let home = tempfile::tempdir().unwrap();
    let provider = ScriptedProvider::answering(["Got it.", "Hello."]);
    let mut owner = agent_in(home.path(), &provider);

    owner.says("/secrets on");
    assert!(owner.hears().await.contains("on"));
    owner.says(&format!("Here is my token: {TOKEN}"));
    assert_eq!(owner.hears().await, "Got it.");

    owner.says("/secrets off");
    assert!(owner.hears().await.contains("turned away again"));
    owner.says(&format!("Again: {TOKEN}"));
    assert!(owner.hears().await.contains("/secret NAME"));
    assert_eq!(provider.times_asked(), 1, "/secrets is not sent to the model");
}

// --- Used by the Agent ---

#[tokio::test]
async fn the_agent_knows_which_secrets_exist_but_not_what_they_are() {
    let home = tempfile::tempdir().unwrap();
    Secrets::kept_in(home.path()).set("GITHUB_TOKEN", TOKEN).unwrap();
    let provider = ScriptedProvider::default();
    provider.then_acts([("list_secrets", json!({}))]).then(Ok("One.".into()));
    let mut owner = agent_in(home.path(), &provider);

    owner.says("Which secrets do you have?");
    owner.hears().await;

    let listed = outcome(&provider);
    assert!(listed.contains("GITHUB_TOKEN"), "{listed}");
    assert!(!listed.contains(TOKEN));
    assert!(owner.was_asked_nothing());
}

#[tokio::test]
async fn a_command_uses_a_secret_by_its_name_and_its_value_never_reaches_the_model() {
    let home = tempfile::tempdir().unwrap();
    Secrets::kept_in(home.path()).set("GITHUB_TOKEN", TOKEN).unwrap();
    let provider = ScriptedProvider::default();
    provider
        .then_acts([(
            "run_command",
            json!({ "command": "echo \"token=$GITHUB_TOKEN\" | tee seen.txt" }),
        )])
        .then(Ok("Done.".into()));
    let mut owner = agent_in(home.path(), &provider);

    owner.says("Use my token.");
    owner.is_asked().await;
    owner.allows();
    owner.hears().await;

    assert_eq!(
        fs::read_to_string(home.path().join("workspace/seen.txt")).unwrap().trim(),
        format!("token={TOKEN}"),
        "the command really had the secret"
    );
    let told = outcome(&provider);
    assert!(!told.contains(TOKEN), "the model was shown the secret: {told}");
    assert!(told.contains("[secret GITHUB_TOKEN]"), "{told}");
}

#[tokio::test]
async fn a_secret_is_hidden_from_the_model_wherever_it_turns_up() {
    let home = tempfile::tempdir().unwrap();
    Secrets::kept_in(home.path()).set("GITHUB_TOKEN", TOKEN).unwrap();
    fs::create_dir_all(home.path().join("workspace")).unwrap();
    fs::write(home.path().join("workspace/notes.txt"), format!("remember {TOKEN} for later"))
        .unwrap();
    let provider = ScriptedProvider::default();
    provider.then_acts([("read_file", json!({ "path": "notes.txt" }))]).then(Ok("Read.".into()));
    let mut owner = agent_in(home.path(), &provider);

    owner.says("Read my notes.");
    owner.hears().await;

    assert_eq!(outcome(&provider), "remember [secret GITHUB_TOKEN] for later");
}

#[tokio::test]
async fn a_command_that_does_not_name_a_secret_is_not_given_it() {
    let home = tempfile::tempdir().unwrap();
    Secrets::kept_in(home.path()).set("GITHUB_TOKEN", TOKEN).unwrap();
    let provider = ScriptedProvider::default();
    provider
        .then_acts([("run_command", json!({ "command": "env > env.txt" }))])
        .then(Ok("Done.".into()));
    let mut owner = agent_in(home.path(), &provider);

    owner.says("Dump the environment.");
    owner.is_asked().await;
    owner.allows();
    owner.hears().await;

    let environment = fs::read_to_string(home.path().join("workspace/env.txt")).unwrap();
    assert!(!environment.contains("GITHUB_TOKEN") && !environment.contains(TOKEN));
}

#[tokio::test]
async fn the_keys_orbite_itself_runs_on_are_not_the_agents_to_list_or_use_but_stay_hidden() {
    let home = tempfile::tempdir().unwrap();
    let own = format!("provider-key-{}", "0123456789abcdef");
    Secrets::kept_in(home.path()).set("ORBITE_PROVIDER_KEY", &own).unwrap();
    fs::create_dir_all(home.path().join("workspace")).unwrap();
    fs::write(home.path().join("workspace/leak.txt"), format!("key is {own}")).unwrap();
    let provider = ScriptedProvider::default();
    provider
        .then_acts([("list_secrets", json!({}))])
        .then_acts([(
            "run_command",
            json!({ "command": "echo \"k=$ORBITE_PROVIDER_KEY\" > seen.txt" }),
        )])
        .then_acts([("read_file", json!({ "path": "leak.txt" }))])
        .then(Ok("Done.".into()));
    let mut owner = agent_in(home.path(), &provider);

    owner.says("Go.");
    owner.is_asked().await;
    owner.allows();
    owner.hears().await;

    let outcomes: Vec<String> = provider
        .last_asked()
        .into_iter()
        .filter_map(|message| match message {
            Message::Outcome { text, .. } => Some(text),
            _ => None,
        })
        .collect();
    assert!(!outcomes[0].contains("ORBITE_PROVIDER_KEY"), "it was listed: {}", outcomes[0]);
    assert_eq!(fs::read_to_string(home.path().join("workspace/seen.txt")).unwrap().trim(), "k=");
    assert_eq!(outcomes[2], "key is [secret ORBITE_PROVIDER_KEY]");
}

#[tokio::test]
async fn a_large_part_of_a_secret_is_hidden_from_the_model_like_the_whole() {
    let home = tempfile::tempdir().unwrap();
    let token = format!("tok_{}{}", "Qm7xLp2VzR9aKd4W", "sN8bTc3YhG6eJf1U");
    Secrets::kept_in(home.path()).set("SERVICE_TOKEN", &token).unwrap();
    fs::create_dir_all(home.path().join("workspace")).unwrap();
    // The beginning, the end, and a few letters only.
    let written = format!(
        "start {} end {} short {} ok",
        &token[..20],
        &token[token.len() - 14..],
        &token[..6]
    );
    fs::write(home.path().join("workspace/parts.txt"), written).unwrap();
    let provider = ScriptedProvider::default();
    provider.then_acts([("read_file", json!({ "path": "parts.txt" }))]).then(Ok("Done.".into()));
    let mut owner = agent_in(home.path(), &provider);

    owner.says("Go.");
    owner.hears().await;

    let told = provider
        .last_asked()
        .into_iter()
        .find_map(|message| match message {
            Message::Outcome { text, .. } => Some(text),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        told,
        "start [part of secret SERVICE_TOKEN] end [part of secret SERVICE_TOKEN] short tok_Qm ok"
    );
}

/// Everything of the Owner's that a model was shown, in one text.
fn shown_to_the_model(provider: &ScriptedProvider) -> String {
    provider
        .every_call()
        .into_iter()
        .flat_map(|(_, conversation)| conversation)
        .map(|message| format!("{message:?}"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[tokio::test]
async fn a_secret_given_with_the_secret_command_is_kept_and_never_shown_to_the_model() {
    let home = tempfile::tempdir().unwrap();
    let provider = ScriptedProvider::answering(["Noted.", "Noted again."]);
    let mut owner = agent_in(home.path(), &provider);

    // Named first, then given in the next message, spaces and all.
    owner.says("/secret shop_password");
    let asked = owner.hears().await;
    assert!(asked.contains("SHOP_PASSWORD") && asked.contains("never show"), "{asked}");
    assert_eq!(provider.times_asked(), 0, "naming a secret is not for the model");
    owner.says("correct horse battery staple");
    let kept = owner.hears().await;
    assert!(kept.contains("SHOP_PASSWORD") && kept.contains("sealed"), "{kept}");
    assert!(!kept.contains("horse"), "{kept}");
    assert_eq!(owner.hears().await, "Noted.");

    // Name and value in one message work as well.
    owner.says("/secret BANK_PIN 4921");
    assert!(owner.hears().await.contains("BANK_PIN"));
    assert_eq!(owner.hears().await, "Noted again.");

    let secrets = Secrets::kept_in(home.path());
    assert_eq!(
        secrets.get("SHOP_PASSWORD").unwrap().as_deref(),
        Some("correct horse battery staple")
    );
    assert_eq!(secrets.get("BANK_PIN").unwrap().as_deref(), Some("4921"));
    // The model learnt the names, and nothing of the values.
    let shown = shown_to_the_model(&provider);
    assert!(shown.contains("SHOP_PASSWORD") && shown.contains("BANK_PIN"), "{shown}");
    assert!(!shown.contains("horse") && !shown.contains("4921"), "{shown}");
}

#[tokio::test]
async fn a_secret_named_wrongly_or_given_up_is_not_kept() {
    let home = tempfile::tempdir().unwrap();
    let provider = ScriptedProvider::answering(["Hello."]);
    let mut owner = agent_in(home.path(), &provider);

    owner.says("/secret 9lives");
    assert!(owner.hears().await.contains("/secret SHOP_PASSWORD"), "how to name one is said");
    owner.says("/secret ORBITE_TELEGRAM_TOKEN");
    assert!(
        owner.hears().await.contains("ORBITE_"),
        "Orbite's own are not the Owner's to set here"
    );

    owner.says("/secret DOOR_CODE");
    owner.hears().await;
    owner.says("/cancel");
    assert!(owner.hears().await.contains("Nothing was kept"));
    // What comes after is an ordinary message again.
    owner.says("Hello");
    assert_eq!(owner.hears().await, "Hello.");
    assert_eq!(Secrets::kept_in(home.path()).get("DOOR_CODE").unwrap(), None);
}

#[tokio::test]
async fn asked_in_plain_words_the_agent_has_its_owner_send_a_secret_it_never_sees() {
    let home = tempfile::tempdir().unwrap();
    let provider = ScriptedProvider::default();
    provider
        .then_acts([("command", json!({ "command": "/secret SHOP_PASSWORD" }))])
        .then(Ok("Go ahead.".into()))
        .then(Ok("Thanks, I have it.".into()));
    let mut owner = agent_in(home.path(), &provider);

    owner.says("I need to give you the password of my shop account.");
    assert_eq!(owner.hears().await, "Go ahead.");
    // Orbite itself then asks for the value, as if the Owner had written the command.
    let asked = owner.hears().await;
    assert!(asked.contains("SHOP_PASSWORD") && asked.contains("never show"), "{asked}");
    owner.says("tr0ub4dor&3");
    assert!(owner.hears().await.contains("sealed"));
    assert_eq!(owner.hears().await, "Thanks, I have it.");

    assert_eq!(
        Secrets::kept_in(home.path()).get("SHOP_PASSWORD").unwrap().as_deref(),
        Some("tr0ub4dor&3")
    );
    assert!(!shown_to_the_model(&provider).contains("tr0ub4dor"));
    assert!(owner.was_asked_nothing(), "asking for a secret needs no approval");
}

#[tokio::test]
async fn the_agent_cannot_loosen_itself_nor_carry_a_secrets_value_by_giving_a_command() {
    let home = tempfile::tempdir().unwrap();
    for refused in ["/settings permission free", "/secrets on", "/secret DOOR 4921", "/reset"] {
        let provider = ScriptedProvider::default();
        provider
            .then_acts([("command", json!({ "command": refused }))])
            .then(Ok("I cannot do that myself.".into()))
            .then(Ok("Hello.".into()));
        let mut owner = agent_in(home.path(), &provider);

        owner.says("Do it for me.");
        assert_eq!(owner.hears().await, "I cannot do that myself.");
        assert!(
            outcome_of_last_action(&provider).contains("not a command you can give"),
            "{refused}"
        );
        // Nothing followed: the next thing heard answers the next message.
        owner.says("Hello");
        assert_eq!(owner.hears().await, "Hello.", "{refused} was carried out");
        assert!(owner.was_asked_nothing());
    }
    assert_eq!(Secrets::kept_in(home.path()).get("DOOR").unwrap(), None);
}

/// The Outcome of the last Action the model was told of.
fn outcome_of_last_action(provider: &ScriptedProvider) -> String {
    provider
        .every_call()
        .into_iter()
        .flat_map(|(_, conversation)| conversation)
        .filter_map(|message| match message {
            Message::Outcome { text, .. } => Some(text),
            _ => None,
        })
        .next_back()
        .unwrap_or_default()
}
