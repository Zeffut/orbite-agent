//! What a future Owner goes through when setting Orbite up in the terminal.

use std::path::Path;
use std::time::Duration;

use orbite::{Config, Secrets, Setup, SetupStep as Step};
use serde_json::{Value, json};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const OWNER: i64 = 4242;
const CODE: &str = "731905";

/// Built from parts: nothing here is a real key or token.
fn api_key() -> String {
    format!("key-{}", "for-the-fake-provider")
}

fn bot_token() -> String {
    format!("{}:{}", "1234567890", "ExampleOnlyNotARealBotToken")
}

fn ok(result: Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({ "ok": true, "result": result }))
}

/// A Provider speaking OpenAI's protocol that accepts `api_key()` only.
async fn provider() -> MockServer {
    let server = MockServer::start().await;
    let authorised = || header("authorization", format!("Bearer {}", api_key()).as_str());
    Mock::given(method("GET"))
        .and(path("/models"))
        .and(authorised())
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [{ "id": "model-large" }, { "id": "model-small" }]
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .and(authorised())
        .respond_with(able_model)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({
            "error": { "message": "Incorrect API key provided." }
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET")).respond_with(ResponseTemplate::new(401)).mount(&server).await;
    server
}

/// Answers as a model that does all a tryout asks of it; "model-mute" is
/// one that only ever talks.
fn able_model(request: &wiremock::Request) -> ResponseTemplate {
    let asked: Value = serde_json::from_slice(&request.body).unwrap();
    let says = |text: &str| {
        ResponseTemplate::new(200).set_body_json(
            json!({ "choices": [{ "message": { "role": "assistant", "content": text } }] }),
        )
    };
    let acts = |name: &str, input: Value| {
        ResponseTemplate::new(200).set_body_json(json!({ "choices": [{ "message": {
            "role": "assistant", "content": null,
            "tool_calls": [{ "id": "call-1", "type": "function", "function": { "name": name, "arguments": input.to_string() } }],
        } }] }))
    };
    let last = asked["messages"]
        .as_array()
        .and_then(|messages| messages.last())
        .cloned()
        .unwrap_or_default();
    let said = last["content"].to_string();
    if asked["model"] == "model-mute" {
        return says("ok");
    }
    if last["role"] == "tool" {
        return says("That makes 4443.");
    }
    if last["content"].is_array() {
        return says("Red.");
    }
    if said.contains("1847") {
        acts("add", json!({ "a": 1847, "b": 2596 }))
    } else if said.contains("Ada") {
        acts("set_reminder", json!({ "text": "call Ada", "at": { "day": "tuesday", "hour": 9 } }))
    } else if said.contains("forum.example") {
        acts("fill_form", json!({ "site": "forum.example", "fields": {} }))
    } else if said.contains("Sam") {
        acts("send_message", json!({ "to": "Sam", "text": "See you at 8" }))
    } else if said.contains("at the very start") {
        says("PELICAN-42")
    } else {
        says("ok")
    }
}

fn message(update_id: i64, sender: i64, text: &str) -> Value {
    json!({
        "update_id": update_id,
        "message": {
            "message_id": update_id,
            "from": { "id": sender, "is_bot": false, "first_name": "Someone" },
            "chat": { "id": sender, "type": "private" },
            "text": text,
        }
    })
}

/// Telegram, knowing the bot `bot_token()` and delivering `updates` to it.
async fn telegram(updates: Vec<Value>) -> MockServer {
    let server = MockServer::start().await;
    let bot = format!("/bot{}", bot_token());
    Mock::given(path(format!("{bot}/getMe")))
        .respond_with(ok(json!({ "id": 1, "is_bot": true, "username": "my_orbite_bot" })))
        .mount(&server)
        .await;
    Mock::given(path(format!("{bot}/getUpdates")))
        .respond_with(ok(json!(updates)))
        .mount(&server)
        .await;
    Mock::given(path(format!("{bot}/sendMessage")))
        .respond_with(ok(json!({ "message_id": 9 })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({
            "ok": false, "error_code": 401, "description": "Unauthorized"
        })))
        .mount(&server)
        .await;
    server
}

/// Runs setup in `home` with `typed` at the keyboard; what was shown.
async fn set_up(
    home: &Path,
    typed: &str,
    provider: &MockServer,
    telegram: &MockServer,
    only: Option<Step>,
) -> String {
    let mut shown = Vec::new();
    Setup::in_home(home, typed.as_bytes(), &mut shown)
        .reaching_providers_at(provider.uri())
        .reaching_telegram_at(telegram.uri())
        .linking_with_code(CODE)
        .waiting_between_looks(Duration::from_millis(10))
        .giving_up_linking_after(Duration::from_millis(400))
        .run(only)
        .await
        .unwrap();
    String::from_utf8(shown).unwrap()
}

/// What an Owner types on a first run: English, the Provider "custom", its
/// key, the first model offered, no lighter model, the bot's token, then a
/// name of their own.
fn first_run() -> String {
    format!("1\ncustom\n{}\n1\n\n{}\nNova\n", api_key(), bot_token())
}

async fn sent_to_owner(telegram: &MockServer) -> Vec<String> {
    telegram
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|request| request.url.path().ends_with("/sendMessage"))
        .map(|request| {
            let body: Value = serde_json::from_slice(&request.body).unwrap();
            assert_eq!(body["chat_id"], OWNER);
            body["text"].as_str().unwrap().to_owned()
        })
        .collect()
}

#[tokio::test]
async fn a_first_setup_ends_with_everything_kept_and_a_first_message_from_the_named_agent() {
    let home = tempfile::tempdir().unwrap();
    let provider = provider().await;
    let telegram = telegram(vec![message(100, OWNER, CODE)]).await;

    let shown = set_up(home.path(), &first_run(), &provider, &telegram, None).await;

    let config = Config::load(home.path()).unwrap();
    assert_eq!(config.language.as_deref(), Some("en"));
    let chosen = config.provider.expect("the Provider is kept");
    assert_eq!((chosen.name.as_str(), chosen.model.as_str()), ("custom", "model-large"));
    assert_eq!(config.telegram_owner, Some(OWNER));
    assert_eq!(config.agent_name.as_deref(), Some("Nova"));
    assert!(
        shown.contains("@my_orbite_bot"),
        "the Owner is shown which bot was recognised: {shown}"
    );
    assert!(shown.contains(CODE), "{shown}");
    let greetings = sent_to_owner(&telegram).await;
    assert!(greetings.last().unwrap().contains("Nova"), "{greetings:?}");
}

#[tokio::test]
async fn keys_and_tokens_are_sealed_and_appear_in_no_file_in_the_clear() {
    let home = tempfile::tempdir().unwrap();
    let provider = provider().await;
    let telegram = telegram(vec![message(100, OWNER, CODE)]).await;

    let shown = set_up(home.path(), &first_run(), &provider, &telegram, None).await;

    let secrets = Secrets::kept_in(home.path());
    assert_eq!(secrets.get("ORBITE_PROVIDER_KEY").unwrap(), Some(api_key()));
    assert_eq!(secrets.get("ORBITE_TELEGRAM_TOKEN").unwrap(), Some(bot_token()));
    for entry in std::fs::read_dir(home.path()).unwrap() {
        let file = entry.unwrap().path();
        let written = String::from_utf8_lossy(&std::fs::read(&file).unwrap()).into_owned();
        assert!(
            !written.contains(&api_key()) && !written.contains(&bot_token()),
            "{} shows a secret",
            file.display()
        );
    }
    assert!(!shown.contains(&api_key()) && !shown.contains(&bot_token()), "a secret was echoed");
}

#[tokio::test]
async fn a_wrong_api_key_is_explained_and_asked_for_again() {
    let home = tempfile::tempdir().unwrap();
    let provider = provider().await;
    let telegram = telegram(vec![message(100, OWNER, CODE)]).await;
    let typed = format!("1\ncustom\nwrong-key\n{}\n1\n\n{}\nNova\n", api_key(), bot_token());

    let shown = set_up(home.path(), &typed, &provider, &telegram, None).await;

    assert!(
        shown.contains("Incorrect API key provided."),
        "the Provider's own words are shown: {shown}"
    );
    assert_eq!(Secrets::kept_in(home.path()).get("ORBITE_PROVIDER_KEY").unwrap(), Some(api_key()));
}

#[tokio::test]
async fn a_wrong_bot_token_is_explained_and_asked_for_again() {
    let home = tempfile::tempdir().unwrap();
    let provider = provider().await;
    let telegram = telegram(vec![message(100, OWNER, CODE)]).await;
    let typed = format!("1\ncustom\n{}\n1\n\nnot-a-token\n{}\nNova\n", api_key(), bot_token());

    let shown = set_up(home.path(), &typed, &provider, &telegram, None).await;

    assert!(shown.contains("BotFather"), "the Owner is told where a token comes from: {shown}");
    assert_eq!(
        Secrets::kept_in(home.path()).get("ORBITE_TELEGRAM_TOKEN").unwrap(),
        Some(bot_token())
    );
}

#[tokio::test]
async fn the_owner_is_whoever_sends_the_code_and_nobody_else() {
    let home = tempfile::tempdir().unwrap();
    let provider = provider().await;
    let telegram = telegram(vec![
        message(100, 666, "hello"),
        message(101, 667, "000000"),
        message(102, OWNER, &format!(" {CODE} ")),
        message(103, 668, CODE),
    ])
    .await;

    set_up(home.path(), &first_run(), &provider, &telegram, None).await;

    assert_eq!(Config::load(home.path()).unwrap().telegram_owner, Some(OWNER));
}

#[tokio::test]
async fn a_setup_cut_short_resumes_where_it_stopped() {
    let home = tempfile::tempdir().unwrap();
    let provider = provider().await;
    let telegram = telegram(vec![message(100, OWNER, CODE)]).await;
    // The keyboard goes away after the Provider is set.
    let mut shown = Vec::new();
    let cut = format!("1\ncustom\n{}\n1\n", api_key());
    let outcome = Setup::in_home(home.path(), cut.as_bytes(), &mut shown)
        .reaching_providers_at(provider.uri())
        .reaching_telegram_at(telegram.uri())
        .linking_with_code(CODE)
        .run(None)
        .await;
    assert!(outcome.is_err(), "setup cannot finish without the rest");
    let asked_before = provider.received_requests().await.unwrap().len();

    let shown =
        set_up(home.path(), &format!("{}\nNova\n", bot_token()), &provider, &telegram, None).await;

    assert_eq!(
        provider.received_requests().await.unwrap().len(),
        asked_before,
        "the Provider was set already"
    );
    assert!(!shown.contains("API key"), "{shown}");
    assert_eq!(Config::load(home.path()).unwrap().agent_name.as_deref(), Some("Nova"));
}

#[tokio::test]
async fn one_part_can_be_done_again_without_touching_the_rest() {
    let home = tempfile::tempdir().unwrap();
    let provider = provider().await;
    let telegram = telegram(vec![message(100, OWNER, CODE)]).await;
    set_up(home.path(), &first_run(), &provider, &telegram, None).await;

    set_up(home.path(), "Sol\n", &provider, &telegram, Some(Step::Name)).await;

    let config = Config::load(home.path()).unwrap();
    assert_eq!(config.agent_name.as_deref(), Some("Sol"));
    assert_eq!(config.telegram_owner, Some(OWNER));
    assert_eq!(config.provider.unwrap().model, "model-large");
}

#[tokio::test]
async fn setup_speaks_the_language_the_owner_chooses() {
    let home = tempfile::tempdir().unwrap();
    let provider = provider().await;
    let telegram = telegram(vec![message(100, OWNER, CODE)]).await;
    let typed = format!("2\ncustom\n{}\n1\n\n{}\nNova\n", api_key(), bot_token());

    let shown = set_up(home.path(), &typed, &provider, &telegram, None).await;

    assert_eq!(Config::load(home.path()).unwrap().language.as_deref(), Some("fr"));
    assert!(shown.contains("Clé d'API"), "{shown}");
    assert!(sent_to_owner(&telegram).await.last().unwrap().contains("Bonjour"));
}

#[tokio::test]
async fn a_name_can_be_picked_from_the_suggestions_and_a_model_typed_by_hand() {
    let home = tempfile::tempdir().unwrap();
    let provider = provider().await;
    let telegram = telegram(vec![message(100, OWNER, CODE)]).await;
    let typed = format!("1\ncustom\n{}\nmodel-of-my-own\n\n{}\n1\n", api_key(), bot_token());

    let shown = set_up(home.path(), &typed, &provider, &telegram, None).await;

    let config = Config::load(home.path()).unwrap();
    assert_eq!(config.provider.unwrap().model, "model-of-my-own");
    let name = config.agent_name.unwrap();
    assert!(shown.contains(&format!("1. {name}")), "the first suggestion was taken: {shown}");
}

#[tokio::test]
async fn the_code_is_not_left_for_the_agent_to_take_for_a_message() {
    let home = tempfile::tempdir().unwrap();
    let provider = provider().await;
    let telegram = telegram(vec![message(100, 666, "hello"), message(101, OWNER, CODE)]).await;

    set_up(home.path(), &first_run(), &provider, &telegram, None).await;

    // Telegram forgets what was delivered once the next one is asked for.
    let asked = telegram.received_requests().await.unwrap();
    let last_look =
        asked.iter().rfind(|request| request.url.path().ends_with("/getUpdates")).unwrap();
    let body: Value = serde_json::from_slice(&last_look.body).unwrap();
    assert_eq!(body["offset"], 102);
}

#[tokio::test]
async fn a_model_nobody_rated_is_tried_out_and_the_owner_sees_what_to_expect() {
    let home = tempfile::tempdir().unwrap();
    let provider = provider().await;
    let telegram = telegram(vec![message(100, OWNER, CODE)]).await;

    let shown = set_up(home.path(), &first_run(), &provider, &telegram, None).await;

    assert!(
        shown.contains("Actions: reliable") && shown.contains("Images: it sees them"),
        "{shown}"
    );
    // Kept, so that it is not tried again and shows in the settings.
    let kept = orbite::rating(home.path(), "custom", "model-large").expect("the verdict is kept");
    assert_eq!(kept.tools, orbite::Tools::Reliable);

    // Setting it up again shows what is known beside the model, without trying it again.
    let asked_before = provider.received_requests().await.unwrap().len();
    let again = format!("custom\n{}\n1\n", api_key());
    let shown = set_up(home.path(), &again, &provider, &telegram, Some(Step::Provider)).await;
    assert!(shown.contains("1. model-large") && shown.contains("acts reliably"), "{shown}");
    let asked_since = provider.received_requests().await.unwrap().len() - asked_before;
    assert!(asked_since <= 4, "the model was tried out again: {asked_since} requests");
}

#[tokio::test]
async fn a_model_that_cannot_act_is_chosen_only_after_the_owner_insists() {
    let home = tempfile::tempdir().unwrap();
    let provider = provider().await;
    let telegram = telegram(vec![message(100, OWNER, CODE)]).await;

    // The mute model, declined at the warning; then the first one offered.
    let typed = format!("1\ncustom\n{}\nmodel-mute\nno\n1\n\n{}\nNova\n", api_key(), bot_token());
    let shown = set_up(home.path(), &typed, &provider, &telegram, None).await;
    assert!(shown.contains("cannot act") && shown.contains("Strongly discouraged"), "{shown}");
    assert_eq!(Config::load(home.path()).unwrap().provider.unwrap().model, "model-large");

    // Nothing is blocked: the Owner who insists gets it.
    let insisting = format!("custom\n{}\nmodel-mute\nyes\n", api_key());
    set_up(home.path(), &insisting, &provider, &telegram, Some(Step::Provider)).await;
    assert_eq!(Config::load(home.path()).unwrap().provider.unwrap().model, "model-mute");
}

#[tokio::test]
async fn a_lighter_model_can_be_chosen_for_simple_work_or_passed_over() {
    let home = tempfile::tempdir().unwrap();
    let provider = provider().await;
    let telegram = telegram(vec![message(100, OWNER, CODE)]).await;

    // Passed over on the first run.
    let shown = set_up(home.path(), &first_run(), &provider, &telegram, None).await;
    assert!(shown.contains("lighter model"), "{shown}");
    assert_eq!(Config::load(home.path()).unwrap().light_model, None);

    // Chosen later, by its number in the list.
    let again = format!("custom\n{}\n1\n2\n", api_key());
    set_up(home.path(), &again, &provider, &telegram, Some(Step::Provider)).await;
    assert_eq!(Config::load(home.path()).unwrap().light_model.as_deref(), Some("model-small"));

    // And given up again by passing over it.
    let without = format!("custom\n{}\n1\n\n", api_key());
    set_up(home.path(), &without, &provider, &telegram, Some(Step::Provider)).await;
    assert_eq!(Config::load(home.path()).unwrap().light_model, None);
}
