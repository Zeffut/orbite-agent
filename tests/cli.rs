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
