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
/// key, the first model offered, then a name of their own.
fn first_run() -> String {
    format!("1\ncustom\n{}\n1\nNova\n", api_key())
}

/// What an Owner who wants their Agent in Telegram too does afterwards:
/// that part of setup, and the bot's token. What was shown.
async fn link_telegram(home: &Path, provider: &MockServer, telegram: &MockServer) -> String {
    set_up(home, &format!("{}\n", bot_token()), provider, telegram, Some(Step::Telegram)).await
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

    let mut shown = set_up(home.path(), &first_run(), &provider, &telegram, None).await;
    // Telegram is not asked about: the Agent is talked to in a browser.
    assert!(!shown.contains("Telegram"), "{shown}");
    assert!(shown.contains("Nova is waiting for you"), "{shown}");
    assert_eq!(Config::load(home.path()).unwrap().interface.as_deref(), Some("127.0.0.1:8377"));
    // Whoever wants it there too sets that part up.
    shown.push_str(&link_telegram(home.path(), &provider, &telegram).await);

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
    let said = sent_to_owner(&telegram).await;
    assert!(said.last().unwrap().contains("Linked"), "{said:?}");
}

#[tokio::test]
async fn keys_and_tokens_are_sealed_and_appear_in_no_file_in_the_clear() {
    let home = tempfile::tempdir().unwrap();
    let provider = provider().await;
    let telegram = telegram(vec![message(100, OWNER, CODE)]).await;

    let mut shown = set_up(home.path(), &first_run(), &provider, &telegram, None).await;
    shown.push_str(&link_telegram(home.path(), &provider, &telegram).await);

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
    let typed = format!("1\ncustom\nwrong-key\n{}\n1\nNova\n", api_key());

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
    let typed = format!("not-a-token\n{}\n", bot_token());

    let shown = set_up(home.path(), &typed, &provider, &telegram, Some(Step::Telegram)).await;

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

    link_telegram(home.path(), &provider, &telegram).await;

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

    let shown = set_up(home.path(), "Nova\n", &provider, &telegram, None).await;

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
    link_telegram(home.path(), &provider, &telegram).await;

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
    let typed = format!("2\ncustom\n{}\n1\nNova\n", api_key());

    let shown = set_up(home.path(), &typed, &provider, &telegram, None).await;

    assert_eq!(Config::load(home.path()).unwrap().language.as_deref(), Some("fr"));
    assert!(shown.contains("clé d'API"), "{shown}");
    assert!(shown.contains("Tout est prêt. Nova t'attend"), "{shown}");
}

#[tokio::test]
async fn a_name_can_be_picked_from_the_suggestions_and_a_model_typed_by_hand() {
    let home = tempfile::tempdir().unwrap();
    let provider = provider().await;
    let telegram = telegram(vec![message(100, OWNER, CODE)]).await;
    let typed = format!("1\ncustom\n{}\nmodel-of-my-own\n1\n", api_key());

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

    link_telegram(home.path(), &provider, &telegram).await;

    // Telegram forgets what was delivered once the next one is asked for.
    let asked = telegram.received_requests().await.unwrap();
    let last_look =
        asked.iter().rfind(|request| request.url.path().ends_with("/getUpdates")).unwrap();
    let body: Value = serde_json::from_slice(&last_look.body).unwrap();
    assert_eq!(body["offset"], 102);
}

#[tokio::test]
async fn the_model_is_not_a_question_where_the_provider_has_a_usual_one() {
    let home = tempfile::tempdir().unwrap();
    let provider = provider().await;
    let telegram = telegram(vec![message(100, OWNER, CODE)]).await;

    // English, Groq, its key; then where to talk and a name: no model.
    let typed = format!("1\ngroq\n{}\nNova\n", api_key());
    let shown = set_up(home.path(), &typed, &provider, &telegram, None).await;

    let config = Config::load(home.path()).unwrap();
    let (usual, lighter) = ("qwen/qwen3.8-27b", "openai/gpt-oss-20b");
    assert_eq!(config.provider.unwrap().model, usual, "{shown}");
    // A lighter one for summaries and wake-ups is taken the same way.
    assert_eq!(config.light_model.as_deref(), Some(lighter));
    assert!(shown.contains(usual), "the Owner is told which it is: {shown}");
    assert!(!shown.contains("Which model?"), "{shown}");
    // Nothing is tried out during setup: a word to the key, one to each model.
    let asked = provider.received_requests().await.unwrap().len();
    assert!(asked <= 4, "{asked} requests to the Provider");
    assert!(orbite::rating(home.path(), "groq", "model-large").is_none());
}

#[tokio::test]
async fn a_provider_without_a_usual_model_has_the_owner_choose_and_no_lighter_one() {
    let home = tempfile::tempdir().unwrap();
    let provider = provider().await;
    let telegram = telegram(vec![message(100, OWNER, CODE)]).await;

    let shown = set_up(home.path(), &first_run(), &provider, &telegram, None).await;

    assert!(shown.contains("Which model?") && shown.contains("1. model-large"), "{shown}");
    assert_eq!(Config::load(home.path()).unwrap().light_model, None);
}

#[tokio::test]
async fn a_model_known_not_to_act_is_chosen_only_after_the_owner_insists() {
    let home = tempfile::tempdir().unwrap();
    let provider = provider().await;
    let telegram = telegram(vec![message(100, OWNER, CODE)]).await;
    // What trying it out on this Host showed, some other day.
    let mute =
        orbite::connect("custom", Some(&api_key()), "model-mute", Some(&provider.uri())).unwrap();
    let verdict = orbite::try_out(&mute).await.unwrap();
    orbite::keep_rating(home.path(), "custom", "model-mute", &verdict).unwrap();

    // The mute model, declined at the warning; then the first one offered.
    let typed = format!("1\ncustom\n{}\nmodel-mute\nno\n1\nNova\n", api_key());
    let shown = set_up(home.path(), &typed, &provider, &telegram, None).await;
    assert!(shown.contains("cannot act") && shown.contains("Strongly discouraged"), "{shown}");
    assert_eq!(Config::load(home.path()).unwrap().provider.unwrap().model, "model-large");

    // Nothing is blocked: the Owner who insists gets it.
    let insisting = format!("custom\n{}\nmodel-mute\nyes\n", api_key());
    set_up(home.path(), &insisting, &provider, &telegram, Some(Step::Provider)).await;
    assert_eq!(Config::load(home.path()).unwrap().provider.unwrap().model, "model-mute");
}

/// What the Owner's browser asked for by following the last link shown.
fn asked_by_the_link(shown: &[u8]) -> std::collections::HashMap<String, String> {
    let shown = String::from_utf8_lossy(shown);
    let link = shown.split_whitespace().rfind(|word| word.contains("/api/accounts/authorize?"));
    url::Url::parse(link.expect("a link was shown")).unwrap().query_pairs().into_owned().collect()
}

/// An Owner who answers what they are shown: each line typed as it is,
/// except `<address>`, the address their browser ended on after the link.
struct Answering {
    lines: std::collections::VecDeque<String>,
    shown: std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
}

impl std::io::Read for Answering {
    fn read(&mut self, into: &mut [u8]) -> std::io::Result<usize> {
        let Some(line) = self.lines.pop_front() else { return Ok(0) };
        let line = if line == "<address>" {
            let asked = asked_by_the_link(&self.shown.lock().unwrap());
            format!(
                "{}?code=code-of-the-plan&state={}&client_id=oaiapp_setup",
                asked["redirect_uri"], asked["state"]
            )
        } else {
            line
        };
        let written = format!("{line}\n");
        into[..written.len()].copy_from_slice(written.as_bytes());
        Ok(written.len())
    }
}

#[derive(Clone)]
struct Shown(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

impl std::io::Write for Shown {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// OpenAI as an Owner with a ChatGPT plan meets it: a sign-in that lets in
/// whoever followed the link last shown, then models answering as them.
async fn openai_with_a_plan(shown: std::sync::Arc<std::sync::Mutex<Vec<u8>>>) -> MockServer {
    use base64::Engine;
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/accounts/oauth/token"))
        .respond_with(move |_: &wiremock::Request| {
            let part = |value: Value| {
                base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(value.to_string())
            };
            let nonce = asked_by_the_link(&shown.lock().unwrap())["nonce"].clone();
            let identity = json!({
                "iss": "https://auth.openai.com", "aud": "oaiapp_setup", "nonce": nonce,
                "exp": 99_999_999_999u64, "email": "owner@example.org",
            });
            ResponseTemplate::new(200).set_body_json(json!({
                "access_token": "access-of-the-plan", "refresh_token": "refresh-of-the-plan",
                "id_token": format!("{}.{}.s", part(json!({})), part(identity)),
                "token_type": "Bearer", "expires_in": 3600,
                "scope": "openid offline_access resource.invoke chatgpt.tokens.use.direct",
            }))
        })
        .mount(&server)
        .await;
    let signed_in = || header("authorization", "Bearer access-of-the-plan");
    Mock::given(method("GET"))
        .and(path("/models"))
        .and(signed_in())
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "models": [
            { "slug": "gpt-of-the-plan", "display_name": "GPT", "visibility": "list" },
        ] })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/responses"))
        .and(signed_in())
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            format!(
                "data: {}\n\ndata: {}\n\n",
                json!({ "type": "response.output_text.delta", "delta": "ok" }),
                json!({ "type": "response.completed", "response": { "usage": {} } })
            ),
            "text/event-stream",
        ))
        .mount(&server)
        .await;
    server
}

#[tokio::test]
async fn an_owner_with_a_chatgpt_plan_signs_in_by_a_link_instead_of_giving_a_key() {
    let home = tempfile::tempdir().unwrap();
    let shown = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let openai = openai_with_a_plan(shown.clone()).await;
    let telegram = telegram(vec![message(100, OWNER, CODE)]).await;
    // English, the ChatGPT plan, a wrong paste then the right address,
    // Telegram, the bot's token, a name: the plan's own model is taken.
    let typed = ["1", "chatgpt", "nonsense", "<address>", "Nova"];
    let owner = Answering {
        lines: typed.iter().map(|line| line.to_string()).collect(),
        shown: shown.clone(),
    };

    Setup::in_home(home.path(), std::io::BufReader::new(owner), Shown(shown.clone()))
        .reaching_providers_at(openai.uri())
        .reaching_telegram_at(telegram.uri())
        .linking_with_code(CODE)
        .waiting_between_looks(Duration::from_millis(10))
        .giving_up_linking_after(Duration::from_millis(400))
        .run(None)
        .await
        .unwrap();

    let shown = String::from_utf8(shown.lock().unwrap().clone()).unwrap();
    // The words OpenAI asks for where its sign-in is offered and used.
    assert!(shown.contains("Continue with ChatGPT"), "{shown}");
    assert!(shown.contains("You're using your ChatGPT plan (owner@example.org)"), "{shown}");
    assert!(shown.contains("Manage usage: https://chatgpt.com/settings/usage"), "{shown}");
    assert!(shown.contains("OpenAI may change it or end it"), "{shown}");
    assert!(!shown.contains("API key (not shown)"), "no key is asked for:\n{shown}");
    assert!(shown.contains("That did not work: that is not an address"), "{shown}");
    let chosen = Config::load(home.path()).unwrap().provider.unwrap();
    assert_eq!((chosen.name.as_str(), chosen.model.as_str()), ("chatgpt", "gpt-of-the-plan"));
    let secrets = Secrets::kept_in(home.path());
    assert!(secrets.get("ORBITE_CHATGPT").unwrap().unwrap().contains("refresh-of-the-plan"));
    assert!(secrets.get(orbite::PROVIDER_KEY).unwrap().is_none());
    assert!(shown.contains("Nova is waiting for you"), "{shown}");
}

#[tokio::test]
async fn another_provider_is_added_after_those_set_up_each_with_a_key_of_its_own() {
    let home = tempfile::tempdir().unwrap();
    let provider = provider().await;
    let telegram = telegram(vec![message(100, OWNER, CODE)]).await;
    set_up(home.path(), &first_run(), &provider, &telegram, None).await;

    // Groq and its key: its usual model is taken.
    let typed = format!("groq\n{}\n", api_key());
    let shown =
        set_up(home.path(), &typed, &provider, &telegram, Some(Step::AnotherProvider)).await;

    let all = Config::load(home.path()).unwrap().providers();
    let names: Vec<&str> = all.iter().map(|provider| provider.name.as_str()).collect();
    assert_eq!(names, ["custom", "groq"], "{shown}");
    assert_eq!(all[1].model, "qwen/qwen3.8-27b");
    let secrets = Secrets::kept_in(home.path());
    for provider in &all {
        let key = provider.key.as_deref().expect("each has a key of its own");
        assert_eq!(secrets.get(key).unwrap(), Some(api_key()), "{key}");
    }
    assert_ne!(all[0].key, all[1].key);
    // Under its own name only: the name that said just "the Provider" is let go of.
    assert_eq!(secrets.get(orbite::PROVIDER_KEY).unwrap(), None);
    assert!(shown.contains("place 2"), "{shown}");

    // Set up again, it takes the place it had rather than a second one.
    set_up(home.path(), &typed, &provider, &telegram, Some(Step::AnotherProvider)).await;
    assert_eq!(Config::load(home.path()).unwrap().providers().len(), 2);
    // And the first, set up again this way, stays first.
    let again = format!("custom\n{}\n1\n", api_key());
    set_up(home.path(), &again, &provider, &telegram, Some(Step::AnotherProvider)).await;
    let names: Vec<String> =
        Config::load(home.path()).unwrap().providers().into_iter().map(|one| one.name).collect();
    assert_eq!(names, ["custom", "groq"]);
}

#[tokio::test]
async fn an_owner_who_chooses_the_browser_needs_no_telegram_and_ends_with_a_link_that_lets_them_in()
{
    let home = tempfile::tempdir().unwrap();
    let provider = provider().await;
    let telegram = telegram(vec![]).await;
    // English, the Provider, its key and model, no lighter model, the browser, a name.
    let typed = format!("1\ncustom\n{}\n1\nNova\n", api_key());

    let shown = set_up(home.path(), &typed, &provider, &telegram, None).await;

    let config = Config::load(home.path()).unwrap();
    assert_eq!(config.interface.as_deref(), Some("127.0.0.1:8377"));
    assert_eq!(config.telegram_owner, None);
    assert!(!shown.contains("BotFather"), "nothing of Telegram is asked: {shown}");
    assert!(telegram.received_requests().await.unwrap().is_empty());
    // The link carries a code that lets a first device in.
    let link = "http://127.0.0.1:8377/#";
    let code: String =
        shown[shown.find(link).expect("no link shown") + link.len()..].chars().take(9).collect();
    let interface = orbite::Interface::serving(home.path(), "127.0.0.1:0".parse().unwrap(), "en")
        .await
        .unwrap();
    let paired = reqwest::Client::new()
        .post(format!("http://{}/api/pair", interface.address()))
        .header("x-orbite", "1")
        .json(&json!({ "code": code, "name": "phone" }))
        .send()
        .await
        .unwrap();
    assert_eq!(paired.status(), 200);
    // The Agent's first words wait there, under its name.
    let kept =
        paired.headers()["set-cookie"].to_str().unwrap().split(';').next().unwrap().to_owned();
    let mut events = reqwest::Client::new()
        .get(format!("http://{}/api/events", interface.address()))
        .header("cookie", kept)
        .send()
        .await
        .unwrap();
    let first = String::from_utf8_lossy(&events.chunk().await.unwrap().unwrap()).into_owned();
    assert!(first.contains("I'm Nova"), "{first}");
}

#[tokio::test]
async fn telegram_is_added_afterwards_by_whoever_wants_it_and_the_browser_stays() {
    let home = tempfile::tempdir().unwrap();
    let provider = provider().await;
    let telegram = telegram(vec![message(100, OWNER, CODE)]).await;
    let shown = set_up(home.path(), &first_run(), &provider, &telegram, None).await;
    assert!(shown.contains("http://127.0.0.1:8377/#"), "{shown}");
    assert_eq!(Config::load(home.path()).unwrap().telegram_owner, None);

    let linking = link_telegram(home.path(), &provider, &telegram).await;

    let config = Config::load(home.path()).unwrap();
    assert_eq!(config.interface.as_deref(), Some("127.0.0.1:8377"));
    assert_eq!(config.telegram_owner, Some(OWNER));
    assert!(linking.contains("Your Telegram account is linked"), "{linking}");
    assert!(sent_to_owner(&telegram).await.last().unwrap().contains("Linked"));
}

#[tokio::test]
async fn the_interface_can_be_turned_on_later_without_going_through_setup_again() {
    let home = tempfile::tempdir().unwrap();
    let provider = provider().await;
    let telegram = telegram(vec![message(100, OWNER, CODE)]).await;
    // An Agent set up before the browser was the place to talk to it.
    link_telegram(home.path(), &provider, &telegram).await;
    assert_eq!(Config::load(home.path()).unwrap().interface, None);

    let shown = set_up(home.path(), "", &provider, &telegram, Some(Step::Interface)).await;

    assert!(shown.contains("http://127.0.0.1:8377"), "{shown}");
    assert_eq!(Config::load(home.path()).unwrap().interface.as_deref(), Some("127.0.0.1:8377"));
}

#[tokio::test]
async fn the_owner_says_how_their_devices_reach_the_agent_and_nothing_else_is_touched() {
    let provider = provider().await;
    let telegram = telegram(vec![]).await;
    // Each time: English, the Provider, its key and model, the browser,
    // how it is reached, a name. On a machine that is on Tailscale and
    // carries the public address 203.0.113.7.
    for (answer, through_tailscale) in [("1", true), ("2", false)] {
        let home = tempfile::tempdir().unwrap();
        let asked = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let typed = format!("1\ncustom\n{}\n1\n{answer}\nNova\n", api_key());
        let mut shown = Vec::new();
        Setup::in_home(home.path(), typed.as_bytes(), &mut shown)
            .reaching_providers_at(provider.uri())
            .reaching_telegram_at(telegram.uri())
            .where_tailscale_is(true)
            .where_the_public_address_is(Some("203.0.113.7".parse().unwrap()))
            .turning_the_interface_on_with({
                let (asked, home) = (asked.clone(), home.path().to_owned());
                move || {
                    asked.store(true, std::sync::atomic::Ordering::SeqCst);
                    let mut config = Config::load(&home).unwrap();
                    config.interface = Some("127.0.0.1:8377".into());
                    config.interface_reached_at = Some("https://host.example.ts.net:8377".into());
                    config.save(&home).map_err(|error| error.to_string())
                }
            })
            .run(None)
            .await
            .unwrap();
        let shown = String::from_utf8(shown).unwrap();

        assert!(shown.contains("How will your devices reach your agent?"), "{shown}");
        // Tailscale is asked to serve the page only when it was chosen.
        assert_eq!(asked.load(std::sync::atomic::Ordering::SeqCst), through_tailscale);
        let config = Config::load(home.path()).unwrap();
        // Either way the interface itself stays on the machine, behind what encrypts.
        assert_eq!(config.interface.as_deref(), Some("127.0.0.1:8377"));
        if through_tailscale {
            assert_eq!(config.interface_public, None);
            assert!(shown.contains("https://host.example.ts.net:8377/#"), "{shown}");
        } else {
            assert_eq!(config.interface_public.as_deref(), Some("203.0.113.7"));
            assert!(shown.contains("https://203.0.113.7/#"), "{shown}");
            assert!(shown.contains("Ports 80 and 443"), "{shown}");
        }
    }
}

#[tokio::test]
async fn a_server_behind_anothers_address_is_asked_for_its_public_one() {
    let home = tempfile::tempdir().unwrap();
    let provider = provider().await;
    let telegram = telegram(vec![]).await;
    // The public way, an address of a private network, a word, then a real one.
    let typed = format!("1\ncustom\n{}\n1\n2\n10.0.0.4\nhere\n198.51.100.20\nNova\n", api_key());
    let mut shown = Vec::new();
    Setup::in_home(home.path(), typed.as_bytes(), &mut shown)
        .reaching_providers_at(provider.uri())
        .reaching_telegram_at(telegram.uri())
        .turning_the_interface_on_with(|| Ok(()))
        .run(None)
        .await
        .unwrap();
    let shown = String::from_utf8(shown).unwrap();

    assert!(shown.contains("private network") && shown.contains("not an address"), "{shown}");
    let config = Config::load(home.path()).unwrap();
    assert_eq!(config.interface_public.as_deref(), Some("198.51.100.20"));
    assert_eq!(config.interface_reached_at.as_deref(), Some("https://198.51.100.20"));
}

#[tokio::test]
async fn a_telegram_that_cannot_be_reached_is_said_without_the_token() {
    let home = tempfile::tempdir().unwrap();
    let provider = provider().await;
    // Telegram at an address nothing listens on, then the Owner leaves.
    let typed = format!("{}\n", bot_token());
    let mut shown = Vec::new();
    let ended = Setup::in_home(home.path(), typed.as_bytes(), &mut shown)
        .reaching_providers_at(provider.uri())
        .reaching_telegram_at("http://127.0.0.1:9")
        .run(Some(Step::Telegram))
        .await;

    assert!(ended.is_err(), "setup went on without Telegram");
    let shown = String::from_utf8(shown).unwrap();
    assert!(shown.contains("Telegram does not know this token"), "{shown}");
    assert!(!shown.contains(&bot_token()), "the token was shown:\n{shown}");
}
