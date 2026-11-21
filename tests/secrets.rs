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

    assert!(
        refused.contains("tell me you have a secret"),
        "the Owner is told where secrets go: {refused}"
    );
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
        assert!(
            owner.hears().await.contains("tell me you have a secret"),
            "{secret} was not recognised"
        );
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
    assert!(owner.hears().await.contains("tell me you have a secret"));
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
    assert!(asked.contains("SHOP_PASSWORD") && asked.contains("never sees"), "{asked}");
    assert_eq!(provider.times_asked(), 0, "naming a secret is not for the model");
    owner.says("correct horse battery staple");
    let kept = owner.hears().await;
    assert!(kept.contains("SHOP_PASSWORD") && kept.contains("is kept"), "{kept}");
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
    // Given up in plain words.
    owner.says("Cancel.");
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
    assert!(asked.contains("SHOP_PASSWORD") && asked.contains("never sees"), "{asked}");
    owner.says("tr0ub4dor&3");
    assert!(owner.hears().await.contains("is kept"));
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
    for refused in ["/settings permission free", "/secrets on", "/reset"] {
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

#[tokio::test]
async fn a_password_written_in_passing_is_kept_by_the_agent_and_removed_from_the_conversation() {
    let home = tempfile::tempdir().unwrap();
    let provider = ScriptedProvider::default();
    provider
        .then_acts([("command", json!({ "command": "/secret DISCORD_PASSWORD Soleil-2024!" }))])
        .then(Ok("I put it away.".into()))
        .then(Ok("Ready to sign in.".into()))
        .then(Ok("Yes.".into()));
    let mut owner = agent_in(home.path(), &provider);

    owner.says("My Discord password is Soleil-2024! and my mail is ada@example.com");
    // What the Agent picks out to keep is the Owner's to agree to: they are
    // shown the name it chose, never the value again.
    let asked = owner.is_asked().await;
    assert!(asked.summary.contains("Keep as the secret DISCORD_PASSWORD"), "{}", asked.summary);
    assert!(!format!("{asked:?}").contains("Soleil"), "{asked:?}");
    owner.allows();
    assert_eq!(owner.hears().await, "I put it away.");
    // Orbite keeps it under the name the Agent chose, and says what it did.
    let kept = owner.hears().await;
    assert!(kept.contains("DISCORD_PASSWORD") && kept.contains("removed"), "{kept}");
    assert!(!kept.contains("Soleil"), "{kept}");
    assert_eq!(owner.hears().await, "Ready to sign in.");
    assert_eq!(
        Secrets::kept_in(home.path()).get("DISCORD_PASSWORD").unwrap().as_deref(),
        Some("Soleil-2024!")
    );
    assert!(owner.saw().iter().any(|shown| shown.contains("erased what held the secret")));

    // From then on the Conversation no longer shows it to a model.
    owner.says("Are you ready?");
    assert_eq!(owner.hears().await, "Yes.");
    let again = format!("{:?}", provider.last_asked());
    assert!(!again.contains("Soleil-2024!"), "{again}");
    assert!(again.contains("ada@example.com"), "the rest of what was said is kept: {again}");
}

#[tokio::test]
async fn the_agent_cannot_keep_as_a_secret_what_its_owner_never_wrote() {
    let home = tempfile::tempdir().unwrap();
    let provider = ScriptedProvider::default();
    provider
        .then_acts([("command", json!({ "command": "/secret DOOR 4921" }))])
        .then(Ok("I could not.".into()));
    let mut owner = agent_in(home.path(), &provider);

    owner.says("Keep my door code.");
    assert_eq!(owner.hears().await, "I could not.");
    assert!(outcome_of_last_action(&provider).contains("not something your Owner wrote"));
    assert_eq!(Secrets::kept_in(home.path()).get("DOOR").unwrap(), None);
}

#[tokio::test]
async fn several_secrets_written_in_one_message_are_each_kept_under_their_name() {
    let home = tempfile::tempdir().unwrap();
    let provider = ScriptedProvider::default();
    provider
        .then_acts([
            ("command", json!({ "command": "/secret SHOP_PASSWORD Tournesol-88!" })),
            ("command", json!({ "command": "/secret SHOP_PIN 4921" })),
        ])
        .then(Ok("Both are put away.".into()))
        // Told that both are kept, it has nothing to add, and says nothing.
        .then(Ok("[silent]".into()))
        .then(Ok("Hello.".into()));
    let mut owner = agent_in(home.path(), &provider);

    owner.says("My shop password is Tournesol-88! and the PIN is 4921.");
    for _ in 0..2 {
        owner.is_asked().await;
        owner.allows();
    }
    assert_eq!(owner.hears().await, "Both are put away.");
    assert!(owner.hears().await.contains("is kept"));
    assert!(owner.hears().await.contains("is kept"));

    let secrets = Secrets::kept_in(home.path());
    assert_eq!(secrets.get("SHOP_PASSWORD").unwrap().as_deref(), Some("Tournesol-88!"));
    assert_eq!(secrets.get("SHOP_PIN").unwrap().as_deref(), Some("4921"));
    // Nothing more was said: the next thing heard answers the next message.
    owner.says("Hello");
    assert_eq!(owner.hears().await, "Hello.");
}

#[tokio::test]
async fn at_the_strict_setting_a_password_is_taken_out_before_any_model_sees_it() {
    let home = tempfile::tempdir().unwrap();
    let provider = ScriptedProvider::default();
    provider
        .then_acts([("command", json!({ "command": "/secret DISCORD_PASSWORD" }))])
        .then(Ok("[silent]".into()))
        .then(Ok("[silent]".into()));
    let mut owner = agent_in(home.path(), &provider);
    owner.says("/secrets strict");
    assert!(owner.hears().await.contains("Strict"));

    owner.says("My Discord password is Soleil-2024! and my mail is ada@example.com");
    // Held back: the Owner is asked, with choices, and no model is.
    let asked = owner.hears().await;
    assert!(asked.contains("not shown it to my AI provider"), "{asked}");
    assert!(asked.contains("/withheld secret") && asked.contains("/withheld send"), "{asked}");
    assert_eq!(provider.times_asked(), 0);

    owner.says("/withheld secret");
    assert!(owner.hears().await.contains("value alone"));
    owner.says("Soleil-2024!");
    // The Agent names it without having seen it, and Orbite keeps it.
    let kept = owner.hears().await;
    assert!(kept.contains("DISCORD_PASSWORD") && kept.contains("is kept"), "{kept}");
    assert!(!kept.contains("saw it"), "no provider saw it: {kept}");

    assert_eq!(
        Secrets::kept_in(home.path()).get("DISCORD_PASSWORD").unwrap().as_deref(),
        Some("Soleil-2024!")
    );
    let shown = shown_to_the_model(&provider);
    assert!(!shown.contains("Soleil-2024!"), "{shown}");
    assert!(shown.contains("My Discord password is [secret]"), "{shown}");
    assert!(shown.contains("ada@example.com"), "{shown}");
}

#[tokio::test]
async fn at_the_strict_setting_a_message_that_only_speaks_of_passwords_is_sent_on_when_said_so() {
    let home = tempfile::tempdir().unwrap();
    let provider = ScriptedProvider::answering(["Use a password manager.", "Hello."]);
    let mut owner = agent_in(home.path(), &provider);
    owner.says("/secrets strict");
    owner.hears().await;

    owner.says("How do I choose a good password?");
    owner.hears().await;
    owner.says("/withheld send");
    assert_eq!(owner.hears().await, "Use a password manager.");
    // What does not speak of one is not held back at all.
    owner.says("Hello");
    assert_eq!(owner.hears().await, "Hello.");

    // And without the strict setting, nothing is held back.
    owner.says("/secrets off");
    owner.hears().await;
    assert!(orbite_settings_say_off(&mut owner).await);
}

async fn orbite_settings_say_off(owner: &mut Owner) -> bool {
    owner.says("/secrets strict");
    let strict = owner.hears().await.contains("Strict");
    owner.says("/secrets off");
    strict && owner.hears().await.contains("turned away again")
}

#[tokio::test]
async fn asked_in_plain_words_the_agent_turns_the_strict_setting_on_but_never_lets_secrets_through()
{
    let home = tempfile::tempdir().unwrap();
    let provider = ScriptedProvider::default();
    provider
        .then_acts([("command", json!({ "command": "/settings secrets strict" }))])
        .then(Ok("[silent]".into()))
        .then_acts([("command", json!({ "command": "/settings secrets on" }))])
        .then(Ok("That one is yours to write.".into()));
    let mut owner = agent_in(home.path(), &provider);

    owner.says("Be strict with my secrets.");
    owner.is_asked().await;
    owner.allows();
    let changed = owner.hears().await;
    assert!(changed.contains("strict"), "Orbite says the setting changed: {changed}");
    assert!(!changed.contains("Permission level"), "in a line, not the whole menu: {changed}");
    // From then on, what speaks of a password is held back.
    owner.says("My password is hunter2");
    assert!(owner.hears().await.contains("not shown it to my AI provider"));
    owner.says("/cancel");
    owner.hears().await;

    owner.says("Now let secrets through.");
    assert_eq!(owner.hears().await, "That one is yours to write.");
    assert!(outcome_of_last_action(&provider).contains("not a command you can give"));
}

/// The texts a model was shown as said by the Owner.
fn said_by_the_owner(provider: &ScriptedProvider) -> Vec<String> {
    provider
        .every_call()
        .into_iter()
        .flat_map(|(_, conversation)| conversation)
        .filter_map(|message| match message {
            Message::Owner(text) => Some(text),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn a_model_cannot_file_a_word_or_a_piece_of_one_as_a_secret() {
    // What a page read by the Agent could talk it into: filing ordinary
    // words as secrets, to have them wiped from the Conversation.
    for (said, asked) in [
        ("Tell me about the weather in Paris please.", "/secret X e"),
        ("Tell me about the weather in Paris please.", "/secret X eath"),
        ("My code is 12.", "/secret X 12"),
    ] {
        let home = tempfile::tempdir().unwrap();
        let provider = ScriptedProvider::default();
        provider
            .then_acts([("command", json!({ "command": asked }))])
            .then(Ok("Done.".into()))
            .then(Ok("Still here.".into()));
        let mut owner = agent_in(home.path(), &provider);

        owner.says(said);
        assert_eq!(owner.hears().await, "Done.");
        assert!(outcome_of_last_action(&provider).contains("not something your Owner wrote"));
        assert_eq!(Secrets::kept_in(home.path()).get("X").unwrap(), None, "{asked}");
        // The Conversation is as it was, and goes on.
        owner.says("Are you there?");
        assert_eq!(owner.hears().await, "Still here.");
        assert!(said_by_the_owner(&provider).contains(&said.to_owned()), "{asked}");
        assert!(!owner.saw().iter().any(|shown| shown.contains("erased")), "{asked}");
    }
}

#[tokio::test]
async fn a_word_the_agent_would_file_as_a_secret_is_the_owners_to_refuse() {
    // What a page read by the Agent could talk it into: filing an ordinary
    // word the Owner wrote, to have it wiped from the Conversation.
    let home = tempfile::tempdir().unwrap();
    let provider = ScriptedProvider::default();
    provider
        .then_acts([("read_file", json!({ "path": "notes.txt" }))])
        .then_acts([("command", json!({ "command": "/secret NOTE file" }))])
        .then(Ok("As you wish.".into()))
        .then(Ok("Still here.".into()));
    std::fs::create_dir_all(home.path().join("workspace")).unwrap();
    std::fs::write(home.path().join("workspace/notes.txt"), "a note").unwrap();
    let mut owner = agent_in(home.path(), &provider);

    owner.says("Please read the file notes.txt");
    let asked = owner.is_asked().await;
    assert!(asked.summary.contains("Keep as the secret NOTE"), "{}", asked.summary);
    owner.denies();
    assert_eq!(owner.hears().await, "As you wish.");

    assert_eq!(Secrets::kept_in(home.path()).get("NOTE").unwrap(), None);
    assert!(!owner.saw().iter().any(|shown| shown.contains("erased")));
    owner.says("Are you there?");
    assert_eq!(owner.hears().await, "Still here.");
    assert!(said_by_the_owner(&provider).contains(&"Please read the file notes.txt".to_owned()));
}

#[tokio::test]
async fn keeping_a_secret_never_rewrites_what_an_action_is_called() {
    // Agreed to, a word that is also part of an Action's name is cleared
    // from what was said, and the Actions stay what they are.
    let home = tempfile::tempdir().unwrap();
    let provider = ScriptedProvider::default();
    provider
        .then_acts([("read_file", json!({ "path": "notes.txt" }))])
        .then_acts([("command", json!({ "command": "/secret NOTE file" }))])
        .then(Ok("[silent]".into()))
        .then(Ok("[silent]".into()))
        .then(Ok("Still here.".into()));
    std::fs::create_dir_all(home.path().join("workspace")).unwrap();
    std::fs::write(home.path().join("workspace/notes.txt"), "a note").unwrap();
    let mut owner = agent_in(home.path(), &provider);

    owner.says("Please read the file notes.txt");
    owner.is_asked().await;
    owner.allows();
    assert!(owner.hears().await.contains("NOTE"));
    owner.says("Are you there?");
    assert_eq!(owner.hears().await, "Still here.");

    let again = provider.last_asked();
    assert!(
        again.iter().any(|message| matches!(message, Message::Agent { actions, .. }
            if actions.iter().any(|action| action.name == "read_file"))),
        "the name of an Action was rewritten: {again:?}"
    );
    assert!(
        again.iter().any(|message| matches!(message, Message::Owner(text)
            if text.contains("[the secret NOTE]") && !text.contains("the file"))),
        "{again:?}"
    );
}

#[tokio::test]
async fn a_short_secret_is_cleared_where_it_stands_alone_and_nowhere_inside_other_words() {
    let home = tempfile::tempdir().unwrap();
    let provider = ScriptedProvider::default();
    provider
        .then_acts([("command", json!({ "command": "/secret DOOR 4921" }))])
        .then(Ok("[silent]".into()))
        .then(Ok("[silent]".into()))
        .then(Ok("Yes.".into()));
    let mut owner = agent_in(home.path(), &provider);

    owner.says("The door code is 4921, and my order number is 849213.");
    owner.is_asked().await;
    owner.allows();
    assert!(owner.hears().await.contains("DOOR"));
    owner.says("Still there?");
    assert_eq!(owner.hears().await, "Yes.");

    assert_eq!(Secrets::kept_in(home.path()).get("DOOR").unwrap().as_deref(), Some("4921"));
    let again = format!("{:?}", provider.last_asked());
    assert!(again.contains("order number is 849213"), "a longer number was cut into: {again}");
    assert!(!again.contains("code is 4921"), "{again}");
}

#[tokio::test]
async fn what_the_agent_asks_for_next_does_not_cancel_the_wait_for_a_secret() {
    let home = tempfile::tempdir().unwrap();
    let provider = ScriptedProvider::default();
    // A secret asked for, then another command right behind it.
    provider
        .then_acts([
            ("command", json!({ "command": "/secret BANK_PASSWORD" })),
            ("command", json!({ "command": "/usage" })),
        ])
        .then(Ok("[silent]".into()))
        .then(Ok("[silent]".into()));
    let mut owner = agent_in(home.path(), &provider);

    owner.says("I want to give you my bank password.");
    assert!(owner.hears().await.contains("BANK_PASSWORD"), "the value is asked for");
    owner.hears().await; // what was used, which the Agent asked to show

    owner.says("correct-horse-battery");
    assert!(owner.hears().await.contains("is kept"));
    assert_eq!(
        Secrets::kept_in(home.path()).get("BANK_PASSWORD").unwrap().as_deref(),
        Some("correct-horse-battery")
    );
    assert!(!shown_to_the_model(&provider).contains("correct-horse"));
}

#[tokio::test]
async fn a_command_the_agent_writes_oddly_is_refused_and_never_taken_for_its_owners_words() {
    let home = tempfile::tempdir().unwrap();
    for odd in ["/mcp  connect notion", "/mcp\tconnect notion", "/usage\n/reset", "/made up"] {
        let provider = ScriptedProvider::default();
        provider
            .then_acts([("command", json!({ "command": odd }))])
            .then(Ok("I could not.".into()))
            .then(Ok("Hello.".into()));
        let mut owner = agent_in(home.path(), &provider);

        owner.says("Do it.");
        assert_eq!(owner.hears().await, "I could not.");
        owner.says("Hello");
        assert_eq!(owner.hears().await, "Hello.", "{odd:?} was carried on with");
        let said = said_by_the_owner(&provider);
        assert!(
            !said.iter().any(|text| text.contains("connect") || text.contains("made up")),
            "{said:?}"
        );
    }
}

#[tokio::test]
async fn a_secret_that_begins_like_a_command_is_still_a_secret() {
    let home = tempfile::tempdir().unwrap();
    let provider = ScriptedProvider::answering(["[silent]"]);
    let mut owner = agent_in(home.path(), &provider);

    owner.says("/secret API_KEY");
    owner.hears().await;
    owner.says("/Xy9+base64/looking==");
    assert!(owner.hears().await.contains("is kept"));

    assert_eq!(
        Secrets::kept_in(home.path()).get("API_KEY").unwrap().as_deref(),
        Some("/Xy9+base64/looking==")
    );
    assert!(!shown_to_the_model(&provider).contains("base64/looking"));
}

#[tokio::test]
async fn a_secret_already_kept_is_replaced_only_with_the_owners_say() {
    let home = tempfile::tempdir().unwrap();
    Secrets::kept_in(home.path()).set("SHOP_PASSWORD", "the-old-one").unwrap();
    let provider = ScriptedProvider::default();
    provider
        .then_acts([("command", json!({ "command": "/secret SHOP_PASSWORD Brand-New-77" }))])
        .then(Ok("Left as it was.".into()));
    let mut owner = agent_in(home.path(), &provider);

    owner.says("My shop password is now Brand-New-77");
    let asked = owner.is_asked().await;
    assert!(asked.summary.contains("Replace the secret SHOP_PASSWORD"), "{}", asked.summary);
    assert!(!format!("{asked:?}").contains("Brand-New"), "the value is not shown again");
    owner.denies();
    assert_eq!(owner.hears().await, "Left as it was.");

    assert_eq!(
        Secrets::kept_in(home.path()).get("SHOP_PASSWORD").unwrap().as_deref(),
        Some("the-old-one")
    );
}

#[tokio::test]
async fn a_secret_asked_for_is_still_awaited_after_orbite_starts_again() {
    let home = tempfile::tempdir().unwrap();
    let database = home.path().join("orbite.db");
    let converse = |provider: &ScriptedProvider| {
        let (mut channel, owner) = memory_channel();
        let (provider, store, folder) =
            (provider.clone(), Store::open(&database).unwrap(), home.path().to_owned());
        let running = tokio::spawn(async move {
            Agent::new(provider, store).keeping_secrets_in(&folder).converse(&mut channel).await
        });
        (owner, running)
    };
    let provider = ScriptedProvider::default();
    let (mut owner, running) = converse(&provider);
    owner.says("/secret DOOR_CODE");
    assert!(owner.hears().await.contains("DOOR_CODE"));
    // Orbite stops before the value comes, as an update or a restart has it do.
    running.abort();
    let _ = running.await;

    let provider = ScriptedProvider::answering(["[silent]"]);
    let (mut owner, _running) = converse(&provider);
    // Back, Orbite says what it is still waiting for.
    let still = owner.hears().await;
    assert!(still.contains("still waiting") && still.contains("DOOR_CODE"), "{still}");
    owner.says("7731-and-more");
    assert!(owner.hears().await.contains("is kept"));

    assert_eq!(
        Secrets::kept_in(home.path()).get("DOOR_CODE").unwrap().as_deref(),
        Some("7731-and-more")
    );
    assert!(!shown_to_the_model(&provider).contains("7731"));
}

#[tokio::test]
async fn at_the_strict_setting_a_password_written_while_the_agent_works_is_held_back_too() {
    let home = tempfile::tempdir().unwrap();
    let provider = ScriptedProvider::default();
    provider.then_never_answers().then(Ok("Here is a bike.".into()));
    let mut owner = agent_in(home.path(), &provider);
    owner.says("/secrets strict");
    owner.hears().await;
    owner.says("Find me a bike.");
    while provider.times_asked() == 0 {
        tokio::task::yield_now().await;
    }

    // Written while the Agent thinks, where nothing looked at it before.
    owner.says("By the way my password is hunter2-and-more");

    // The Agent goes on with what it was asked, without it...
    assert_eq!(owner.hears().await, "Here is a bike.");
    // ...and Orbite then asks what to do with the message it held back.
    assert!(owner.hears().await.contains("not shown it to my AI provider"));
    assert!(!shown_to_the_model(&provider).contains("hunter2"));
}

#[tokio::test]
async fn at_the_strict_setting_the_value_of_a_held_back_message_is_still_awaited_after_a_restart() {
    let home = tempfile::tempdir().unwrap();
    let database = home.path().join("orbite.db");
    let converse = |provider: &ScriptedProvider| {
        let (mut channel, owner) = memory_channel();
        let (provider, store, folder) =
            (provider.clone(), Store::open(&database).unwrap(), home.path().to_owned());
        let running = tokio::spawn(async move {
            Agent::new(provider, store).keeping_secrets_in(&folder).converse(&mut channel).await
        });
        (owner, running)
    };
    let provider = ScriptedProvider::default();
    let (mut owner, running) = converse(&provider);
    owner.says("/secrets strict");
    owner.hears().await;
    owner.says("My bank password is Mimosa-2026-bank");
    owner.hears().await;
    owner.says("/withheld secret");
    assert!(owner.hears().await.contains("value alone"));
    // Orbite stops before the value comes.
    running.abort();
    let _ = running.await;

    let provider = ScriptedProvider::default();
    provider
        .then_acts([("command", json!({ "command": "/secret BANK_PASSWORD" }))])
        .then(Ok("[silent]".into()))
        .then(Ok("[silent]".into()));
    let (mut owner, _running) = converse(&provider);
    assert!(owner.hears().await.contains("still waiting"));
    owner.says("Mimosa-2026-bank");
    assert!(owner.hears().await.contains("is kept"));

    assert_eq!(
        Secrets::kept_in(home.path()).get("BANK_PASSWORD").unwrap().as_deref(),
        Some("Mimosa-2026-bank")
    );
    assert!(!shown_to_the_model(&provider).contains("Mimosa"));
}

#[tokio::test]
async fn a_secret_put_away_is_not_sent_back_to_a_provider_that_wants_its_own_words_again() {
    // Anthropic is sent back exactly what it wrote, the Action that named the
    // secret included: the value must be gone from that too.
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};
    let answering = |content: serde_json::Value| {
        ResponseTemplate::new(200).set_body_json(json!({
            "id": "msg_1", "type": "message", "role": "assistant", "model": "claude-opus-5-5",
            "content": content, "stop_reason": "end_turn",
            "usage": { "input_tokens": 10, "output_tokens": 5 },
        }))
    };
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(answering(json!([
            { "type": "text", "text": "I put it away." },
            { "type": "tool_use", "id": "toolu_1", "name": "command",
              "input": { "command": "/secret DOOR_CODE 7731-abcd" } },
        ])))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(answering(json!([{ "type": "text", "text": "Yes." }])))
        .mount(&server)
        .await;
    let home = tempfile::tempdir().unwrap();
    let (mut channel, mut owner) = memory_channel();
    let (folder, provider) = (
        home.path().to_owned(),
        orbite::Anthropic::new("sk-ant-test", "claude-opus-5-5").at(server.uri()),
    );
    tokio::spawn(async move {
        Agent::new(provider, Store::in_memory())
            .keeping_secrets_in(&folder)
            .converse(&mut channel)
            .await
    });

    owner.says("The door code is 7731-abcd");
    owner.is_asked().await;
    owner.allows();
    // Among what is said next comes Orbite's word that it is kept.
    let mut heard = Vec::new();
    while !heard.iter().any(|said: &String| said.contains("DOOR_CODE")) {
        assert!(heard.len() < 5, "it was never said to be kept: {heard:?}");
        heard.push(owner.hears().await);
    }
    owner.says("Still there?");
    owner.hears().await;
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    assert_eq!(
        Secrets::kept_in(home.path()).get("DOOR_CODE").unwrap().as_deref(),
        Some("7731-abcd")
    );
    // Once kept, nothing sent to the provider holds it any more.
    let sent = server.received_requests().await.unwrap();
    let last = String::from_utf8_lossy(&sent.last().unwrap().body).into_owned();
    assert!(!last.contains("7731-abcd"), "{last}");
    assert!(last.contains("[the secret DOOR_CODE]"), "{last}");
}

#[tokio::test]
async fn what_the_owner_wrote_before_being_asked_for_a_secret_neither_answers_nor_cancels_it() {
    let home = tempfile::tempdir().unwrap();
    let provider = ScriptedProvider::default();
    provider
        .then_acts([("command", json!({ "command": "/secret BANK_PASSWORD" }))])
        .then_never_answers()
        .then(Ok("[silent]".into()))
        .then(Ok("[silent]".into()));
    let mut owner = agent_in(home.path(), &provider);
    owner.says("I want to give you my bank password.");
    // The Agent has asked for the secret and is still at work...
    while provider.times_asked() < 2 {
        tokio::task::yield_now().await;
    }
    // ...when the Owner writes something else, before any question is shown.
    owner.says("/usage");

    let asked = owner.hears().await;
    assert!(asked.contains("BANK_PASSWORD"), "the value is asked for: {asked}");
    let used = owner.hears().await;
    assert!(
        used.contains("seven days") || used.contains("Today"),
        "what was written is carried out: {used}"
    );
    // The question still stands, and what is sent now answers it.
    owner.says("correct-horse-battery");
    assert!(owner.hears().await.contains("is kept"));

    assert_eq!(
        Secrets::kept_in(home.path()).get("BANK_PASSWORD").unwrap().as_deref(),
        Some("correct-horse-battery")
    );
    assert!(!shown_to_the_model(&provider).contains("correct-horse"));
}
