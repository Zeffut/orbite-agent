//! The `orbite` command, run as an Owner would on the Host.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};

fn orbite(home: &Path, arguments: &[&str], typed: &str) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_orbite"))
        .args(arguments)
        .env("ORBITE_HOME", home)
        .env_remove("ORBITE_PROVIDER")
        .env_remove("ORBITE_MODEL")
        .env_remove("ORBITE_API_KEY")
        .env_remove("ORBITE_BASE_URL")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    command.stdin.take().unwrap().write_all(typed.as_bytes()).unwrap();
    command.wait_with_output().unwrap()
}

fn shown(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn a_secret_is_entered_listed_and_removed_on_the_host_without_ever_being_shown() {
    let home = tempfile::tempdir().unwrap();
    let value = "ghp_ExampleOnlyNotARealTokenValue0123456789";

    let set = orbite(home.path(), &["secret", "set", "github_token"], &format!("{value}\n"));
    assert!(set.status.success(), "{}", shown(&set));
    assert!(
        shown(&set).contains("GITHUB_TOKEN") && !shown(&set).contains(value),
        "{}",
        shown(&set)
    );

    let listed = orbite(home.path(), &["secret", "list"], "");
    assert_eq!(String::from_utf8_lossy(&listed.stdout).trim(), "GITHUB_TOKEN");

    assert_eq!(
        orbite::Secrets::kept_in(home.path()).get("GITHUB_TOKEN").unwrap().as_deref(),
        Some(value)
    );

    let removed = orbite(home.path(), &["secret", "remove", "GITHUB_TOKEN"], "");
    assert!(removed.status.success());
    assert!(
        String::from_utf8_lossy(&orbite(home.path(), &["secret", "list"], "").stdout)
            .trim()
            .is_empty()
    );
}

#[test]
fn what_is_wrong_with_a_secret_is_said_and_nothing_is_kept() {
    let home = tempfile::tempdir().unwrap();

    let unnamed = orbite(home.path(), &["secret", "set"], "value\n");
    assert!(!unnamed.status.success());
    assert!(shown(&unnamed).contains("orbite secret set NAME"), "{}", shown(&unnamed));

    let empty = orbite(home.path(), &["secret", "set", "TOKEN"], "\n");
    assert!(!empty.status.success());
    assert!(shown(&empty).contains("empty"), "{}", shown(&empty));

    let unknown = orbite(home.path(), &["secret", "remove", "NOTHING"], "");
    assert!(!unknown.status.success());
    assert!(orbite::Secrets::kept_in(home.path()).names().unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn once_set_up_the_command_needs_no_setting_from_the_environment() {
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};
    let key = format!("key-{}", "from-the-sealed-store");
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .and(header("authorization", format!("Bearer {key}").as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "choices": [{ "message": { "role": "assistant", "content": "Hello from the configured provider." } }]
        })))
        .mount(&server)
        .await;
    let home = tempfile::tempdir().unwrap();
    orbite::Config {
        provider: Some(orbite::ProviderChoice {
            name: "custom".into(),
            model: "some-model".into(),
            address: Some(server.uri()),
            ..Default::default()
        }),
        agent_name: Some("Nova".into()),
        ..orbite::Config::default()
    }
    .save(home.path())
    .unwrap();
    orbite::Secrets::kept_in(home.path()).set("ORBITE_PROVIDER_KEY", &key).unwrap();

    let folder = home.path().to_owned();
    let chatted =
        tokio::task::spawn_blocking(move || orbite(&folder, &["chat"], "Hi\n")).await.unwrap();

    assert!(shown(&chatted).contains("Hello from the configured provider."), "{}", shown(&chatted));
    let asked = server.received_requests().await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&asked[0].body).unwrap();
    assert!(body["messages"][0]["content"].as_str().unwrap().contains("Your name is Nova"));
}

#[test]
fn the_command_says_which_version_it_is() {
    let home = tempfile::tempdir().unwrap();

    let said = orbite(home.path(), &["version"], "");

    assert_eq!(
        String::from_utf8_lossy(&said.stdout).trim(),
        format!("orbite {}", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn an_owner_turns_automatic_updates_off_and_on_again() {
    let home = tempfile::tempdir().unwrap();

    let off = orbite(home.path(), &["update", "off"], "");
    assert!(off.status.success(), "{}", shown(&off));
    assert_eq!(orbite::Config::load(home.path()).unwrap().automatic_updates, Some(false));
    let left_alone = orbite(home.path(), &["update"], "");
    assert!(shown(&left_alone).contains("off"), "{}", shown(&left_alone));

    let on = orbite(home.path(), &["update", "on"], "");
    assert!(on.status.success(), "{}", shown(&on));
    assert_ne!(orbite::Config::load(home.path()).unwrap().automatic_updates, Some(false));
}

/// An installation thinking with three Providers, in this order.
fn with_three_providers(home: &Path) {
    let provider = |name: &str| serde_json::json!({ "name": name, "model": format!("{name}-model"), "key": format!("ORBITE_KEY_{}", name.to_uppercase()) });
    let config = serde_json::json!({
        "language": "en",
        "light_model": "chatgpt-light",
        "provider": provider("chatgpt"),
        "fallbacks": [provider("groq"), provider("ollama")],
    });
    std::fs::write(home.join("config.json"), config.to_string()).unwrap();
}

fn provider_names(home: &Path) -> Vec<String> {
    let all = orbite::Config::load(home).unwrap().providers();
    all.into_iter().map(|provider| provider.name).collect()
}

#[test]
fn the_owner_sees_their_providers_in_order_and_changes_which_comes_first() {
    let home = tempfile::tempdir().unwrap();
    with_three_providers(home.path());

    let listed = shown(&orbite(home.path(), &["providers"], ""));
    let at = |name: &str| listed.find(name).unwrap_or_else(|| panic!("{name} in:\n{listed}"));
    assert!(at("1. chatgpt") < at("2. groq") && at("2. groq") < at("3. ollama"), "{listed}");

    let moved = orbite(home.path(), &["providers", "first", "groq"], "");
    assert!(moved.status.success(), "{}", shown(&moved));
    assert!(shown(&moved).contains("next start"), "{}", shown(&moved));
    assert_eq!(provider_names(home.path()), ["groq", "chatgpt", "ollama"]);
    // The lighter model was one of the Provider that came first.
    assert_eq!(orbite::Config::load(home.path()).unwrap().light_model, None);
}

#[test]
fn a_provider_is_given_a_daily_limit_and_freed_of_it() {
    let home = tempfile::tempdir().unwrap();
    with_three_providers(home.path());

    let limited = orbite(home.path(), &["providers", "limit", "chatgpt", "200,000"], "");
    assert!(shown(&limited).contains("200,000 tokens a day"), "{}", shown(&limited));
    let all = orbite::Config::load(home.path()).unwrap().providers();
    assert_eq!(all[0].daily_tokens, Some(200_000));
    assert!(
        shown(&orbite(home.path(), &["providers"], "")).contains("at most 200,000 tokens a day")
    );

    orbite(home.path(), &["providers", "limit", "chatgpt", "none"], "");
    assert_eq!(orbite::Config::load(home.path()).unwrap().providers()[0].daily_tokens, None);

    let wrong = orbite(home.path(), &["providers", "limit", "chatgpt", "a lot"], "");
    assert!(!wrong.status.success() && shown(&wrong).contains("number of tokens"));
}

#[test]
fn a_provider_is_removed_with_its_key_but_never_the_last_one() {
    let home = tempfile::tempdir().unwrap();
    with_three_providers(home.path());
    let secrets = orbite::Secrets::kept_in(home.path());
    secrets.set("ORBITE_KEY_GROQ", &format!("key-{}", "of-groq")).unwrap();

    let removed = orbite(home.path(), &["providers", "remove", "groq"], "");
    assert!(removed.status.success(), "{}", shown(&removed));
    assert_eq!(provider_names(home.path()), ["chatgpt", "ollama"]);
    assert_eq!(secrets.get("ORBITE_KEY_GROQ").unwrap(), None);

    let unknown = orbite(home.path(), &["providers", "remove", "groq"], "");
    assert!(!unknown.status.success());
    assert!(shown(&unknown).contains("chatgpt, ollama"), "{}", shown(&unknown));

    orbite(home.path(), &["providers", "remove", "ollama"], "");
    let last = orbite(home.path(), &["providers", "remove", "chatgpt"], "");
    assert!(!last.status.success() && shown(&last).contains("only provider"), "{}", shown(&last));
    assert_eq!(provider_names(home.path()), ["chatgpt"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_next_provider_set_up_answers_with_its_own_key_when_the_first_is_out_of_usage() {
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};
    let (first_key, second_key) =
        (format!("key-{}", "of-the-first"), format!("key-{}", "of-the-second"));
    let spent = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(429).set_body_json(serde_json::json!({
            "error": { "message": "You are out of usage." }
        })))
        .mount(&spent)
        .await;
    let ready = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .and(header("authorization", format!("Bearer {second_key}").as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "choices": [{ "message": { "role": "assistant", "content": "Hello from the second." } }]
        })))
        .mount(&ready)
        .await;
    let home = tempfile::tempdir().unwrap();
    let choice = |name: &str, address: String, key: &str| orbite::ProviderChoice {
        name: name.into(),
        model: "some-model".into(),
        address: Some(address),
        key: Some(key.into()),
        ..Default::default()
    };
    orbite::Config {
        provider: Some(choice("groq", spent.uri(), "ORBITE_KEY_GROQ")),
        fallbacks: vec![choice("custom", ready.uri(), "ORBITE_KEY_CUSTOM")],
        ..orbite::Config::default()
    }
    .save(home.path())
    .unwrap();
    let secrets = orbite::Secrets::kept_in(home.path());
    secrets.set("ORBITE_KEY_GROQ", &first_key).unwrap();
    secrets.set("ORBITE_KEY_CUSTOM", &second_key).unwrap();

    let folder = home.path().to_owned();
    let chatted =
        tokio::task::spawn_blocking(move || orbite(&folder, &["chat"], "Hi\n")).await.unwrap();

    let shown = shown(&chatted);
    assert!(shown.contains("Hello from the second."), "{shown}");
    // The Owner is told who answers when it is not the usual one.
    assert!(shown.contains("custom"), "{shown}");
}
