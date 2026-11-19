//! What an Owner goes through to have speech written down on their own
//! machine: what they are told before choosing a model, and what is
//! refused.

use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Output, Stdio};

/// Runs `orbite transcription` on a machine said to have `memory` MB and
/// `cores` cores, where only the programs in `<home>/bin` exist beside the
/// system's own.
fn transcription(
    home: &Path,
    memory: u32,
    cores: u32,
    arguments: &[&str],
    more: &[(&str, &str)],
) -> Output {
    let bin = home.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    Command::new(env!("CARGO_BIN_EXE_orbite"))
        .arg("transcription")
        .args(arguments)
        .env("ORBITE_HOME", home)
        .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
        .env("ORBITE_MACHINE_MEMORY_MB", memory.to_string())
        .env("ORBITE_MACHINE_CORES", cores.to_string())
        .envs(more.iter().copied())
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

fn shown(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

/// Puts a program called `name` on the machine, doing `body`.
fn stand_in(home: &Path, name: &str, body: &str) {
    let file = home.join("bin").join(name);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// The lines said about the model called `name`: its own, which begins
/// with a mark and its name, and the warnings under it.
fn about(listed: &str, name: &str) -> String {
    let mut lines = listed.lines().skip_while(|line| {
        let mut words = line.split_whitespace();
        !(words.next().is_some_and(|mark| !mark.is_ascii()) && words.next() == Some(name))
    });
    let own = lines.next().unwrap_or_else(|| panic!("{name} is not listed in:\n{listed}"));
    let warnings: Vec<&str> = lines.take_while(|line| line.starts_with("       ")).collect();
    format!("{own}\n{}", warnings.join("\n"))
}

#[test]
fn a_small_machine_is_told_which_models_it_would_carry_badly() {
    let home = tempfile::tempdir().unwrap();

    let listed = shown(&transcription(home.path(), 1500, 2, &[], &[]));

    assert!(listed.contains("1500 MB of memory, 2 processor core(s)"), "{listed}");
    // Too small to listen well, whatever the machine.
    assert!(about(&listed, "tiny").contains("many mistakes"), "{listed}");
    // Fits, and listens well.
    let small = about(&listed, "small");
    assert!(!small.contains("will not run") && !small.contains("many mistakes"), "{small}");
    // More than half of the machine's memory, and slow on two cores.
    assert!(small.contains("852 MB") && small.contains("2 processor core(s)"), "{small}");
    // Does not fit at all.
    let large = about(&listed, "large-v3");
    assert!(large.contains("3900 MB") && large.contains("will not run"), "{large}");
    // Where the project gives no figure, the file's size is the least it takes.
    assert!(about(&listed, "large-v3-turbo").contains("at least 1624 MB"), "{listed}");
}

#[test]
fn a_large_machine_is_warned_of_nothing_it_can_carry() {
    let home = tempfile::tempdir().unwrap();

    let listed = shown(&transcription(home.path(), 32000, 12, &[], &[]));

    assert!(listed.contains("✅ small"), "{listed}");
    assert!(listed.contains("✅ large-v3 "), "{listed}");
    assert!(listed.contains("⚠️ tiny"), "small models stay small on any machine: {listed}");
}

#[test]
fn whisper_cpp_is_not_chosen_where_it_is_not_installed_and_the_owner_is_told_how() {
    let home = tempfile::tempdir().unwrap();

    let refused = transcription(home.path(), 8000, 4, &["whisper.cpp", "small"], &[]);

    assert!(!refused.status.success());
    assert!(shown(&refused).contains("apt install whisper.cpp"), "{}", shown(&refused));
    assert_eq!(orbite::Config::load(home.path()).unwrap().transcription, None);
}

#[test]
fn a_model_the_machine_cannot_carry_is_refused_unless_the_owner_insists() {
    let home = tempfile::tempdir().unwrap();
    stand_in(home.path(), "whisper-cli", "exit 0");

    let refused = transcription(home.path(), 2000, 2, &["whisper.cpp", "large-v3"], &[]);

    assert!(!refused.status.success(), "{}", shown(&refused));
    assert!(shown(&refused).contains("will not run"), "{}", shown(&refused));
    assert!(shown(&refused).contains("--anyway"), "{}", shown(&refused));
    assert!(!shown(&refused).contains("Downloading"), "nothing is fetched for nothing");
    assert_eq!(orbite::Config::load(home.path()).unwrap().transcription, None);
}

#[test]
fn a_model_file_of_the_owners_is_tried_on_the_machine_and_its_speed_is_said() {
    let home = tempfile::tempdir().unwrap();
    stand_in(home.path(), "whisper-cli", "echo ''");
    let model = home.path().join("my-model.bin");
    std::fs::write(&model, b"a model").unwrap();

    let chosen =
        transcription(home.path(), 8000, 4, &["whisper.cpp", model.to_str().unwrap()], &[]);

    assert!(chosen.status.success(), "{}", shown(&chosen));
    assert!(shown(&chosen).contains("to listen to 10 seconds of sound"), "{}", shown(&chosen));
    // No converter on this pretend machine: the Owner is told what that costs.
    let kept = orbite::Config::load(home.path()).unwrap().transcription.unwrap();
    assert_eq!(kept.engine, "whisper.cpp");
    assert_eq!(kept.model.as_deref(), model.to_str());
}

#[test]
fn a_machine_slower_than_speech_is_told_so() {
    let home = tempfile::tempdir().unwrap();
    // The clock is not waited for: the program says how long it took by taking it.
    stand_in(home.path(), "whisper-cli", "sleep 11");
    let model = home.path().join("my-model.bin");
    std::fs::write(&model, b"a model").unwrap();

    let chosen =
        transcription(home.path(), 8000, 4, &["whisper.cpp", model.to_str().unwrap()], &[]);

    assert!(chosen.status.success(), "{}", shown(&chosen));
    assert!(shown(&chosen).contains("slower than speech itself"), "{}", shown(&chosen));
}

#[test]
fn a_model_that_does_not_run_is_not_chosen() {
    let home = tempfile::tempdir().unwrap();
    stand_in(home.path(), "whisper-cli", "echo 'error: failed to load model' >&2; exit 3");
    let model = home.path().join("my-model.bin");
    std::fs::write(&model, b"not a model").unwrap();

    let refused =
        transcription(home.path(), 8000, 4, &["whisper.cpp", model.to_str().unwrap()], &[]);

    assert!(!refused.status.success());
    assert!(shown(&refused).contains("failed to load model"), "{}", shown(&refused));
    assert_eq!(orbite::Config::load(home.path()).unwrap().transcription, None);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_download_that_is_not_the_published_model_is_not_kept() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};
    let source = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/ggml-tiny.bin"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"something else".to_vec()))
        .mount(&source)
        .await;
    let home = tempfile::tempdir().unwrap();
    stand_in(home.path(), "whisper-cli", "exit 0");
    let folder = home.path().to_owned();
    let from = source.uri();

    let refused = tokio::task::spawn_blocking(move || {
        transcription(&folder, 8000, 4, &["whisper.cpp", "tiny"], &[("ORBITE_MODELS_URL", &from)])
    })
    .await
    .unwrap();

    assert!(!refused.status.success(), "{}", shown(&refused));
    assert!(shown(&refused).contains("does not match"), "{}", shown(&refused));
    let models = home.path().join("models");
    let left: Vec<_> = std::fs::read_dir(&models).into_iter().flatten().flatten().collect();
    assert!(left.is_empty(), "{left:?}");
    assert_eq!(orbite::Config::load(home.path()).unwrap().transcription, None);
}

#[test]
fn other_ways_are_chosen_in_a_word_and_a_command_must_say_where_the_recording_goes() {
    let home = tempfile::tempdir().unwrap();
    let load = || orbite::Config::load(home.path()).unwrap().transcription.unwrap();

    assert!(
        transcription(
            home.path(),
            8000,
            4,
            &["server", "http://127.0.0.1:8000/v1", "Systran/faster-whisper-small"],
            &[]
        )
        .status
        .success()
    );
    assert_eq!(
        (load().engine.as_str(), load().address.as_deref()),
        ("server", Some("http://127.0.0.1:8000/v1"))
    );

    let refused = transcription(home.path(), 8000, 4, &["command", "my-engine --quiet"], &[]);
    assert!(!refused.status.success() && shown(&refused).contains("{file}"), "{}", shown(&refused));
    assert!(
        transcription(home.path(), 8000, 4, &["command", "my-engine {file}"], &[]).status.success()
    );
    assert_eq!(load().command.as_deref(), Some("my-engine {file}"));

    assert!(transcription(home.path(), 8000, 4, &["off"], &[]).status.success());
    assert_eq!(load().engine, "off");
    assert!(shown(&transcription(home.path(), 8000, 4, &[], &[])).contains("written down by: off"));
}

#[tokio::test(flavor = "multi_thread")]
async fn what_the_owner_says_aloud_is_written_down_by_whisper_cpp_once_chosen() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};
    let thinking = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "choices": [{ "message": { "role": "assistant", "content": "I will remind you." } }]
        })))
        .mount(&thinking)
        .await;
    let home = tempfile::tempdir().unwrap();
    stand_in(home.path(), "whisper-cli", "echo \"Water the plants tonight. [$*]\"");
    stand_in(home.path(), "ffmpeg", "for last; do :; done; echo wav > \"$last\"");
    let model = home.path().join("my-model.bin");
    std::fs::write(&model, b"a model").unwrap();
    let folder = home.path().to_owned();
    let (chosen, spoken) = tokio::task::spawn_blocking(move || {
        let model = folder.join("my-model.bin");
        (
            transcription(&folder, 8000, 4, &["whisper.cpp", model.to_str().unwrap()], &[]),
            transcription(&folder, 8000, 4, &["language", "en"], &[]),
        )
    })
    .await
    .unwrap();
    assert!(chosen.status.success() && spoken.status.success(), "{}", shown(&chosen));
    let voice = home.path().join("voice-1.ogg");
    std::fs::write(&voice, b"OggS").unwrap();
    let mut config = orbite::Config::load(home.path()).unwrap();
    config.provider = Some(orbite::ProviderChoice {
        name: "ollama".into(),
        model: "some-model".into(),
        address: Some(thinking.uri()),
        ..Default::default()
    });
    config.save(home.path()).unwrap();

    let folder = home.path().to_owned();
    let said = format!("{}\n", orbite::attached(&voice));
    let chatted = tokio::task::spawn_blocking(move || {
        let mut chat = Command::new(env!("CARGO_BIN_EXE_orbite"))
            .arg("chat")
            .env("ORBITE_HOME", &folder)
            .env("PATH", format!("{}:/usr/bin:/bin", folder.join("bin").display()))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        chat.stdin.take().unwrap().write_all(said.as_bytes()).unwrap();
        chat.wait_with_output().unwrap()
    })
    .await
    .unwrap();

    assert!(shown(&chatted).contains("I will remind you."), "{}", shown(&chatted));
    let asked = thinking.received_requests().await.unwrap();
    let read = String::from_utf8_lossy(&asked[0].body).into_owned();
    // The model read what was said, heard in the language the Owner named.
    assert!(read.contains("Water the plants tonight."), "{read}");
    assert!(read.contains("-l en"), "{read}");
}

/// With the real whisper.cpp, where a machine has it: ORBITE_TESTS_WITH_WHISPER
/// names a spoken recording and a word said in it, as `file:word`.
#[tokio::test(flavor = "multi_thread")]
async fn the_real_whisper_cpp_fetches_its_published_model_and_writes_a_recording_down() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};
    let Ok(spoken) = std::env::var("ORBITE_TESTS_WITH_WHISPER") else { return };
    let (recording, word) = spoken.rsplit_once(':').expect("file:word");
    let thinking = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "choices": [{ "message": { "role": "assistant", "content": "Heard." } }]
        })))
        .mount(&thinking)
        .await;
    let home = tempfile::tempdir().unwrap();
    orbite::Config {
        provider: Some(orbite::ProviderChoice {
            name: "ollama".into(),
            model: "some-model".into(),
            address: Some(thinking.uri()),
            ..Default::default()
        }),
        ..orbite::Config::default()
    }
    .save(home.path())
    .unwrap();
    let folder = home.path().to_owned();
    let said = format!("[attached: {recording}]\n");
    let (chosen, chatted) = tokio::task::spawn_blocking(move || {
        // The machine's own programs, and the model as its project publishes it.
        let chosen = Command::new(env!("CARGO_BIN_EXE_orbite"))
            .args(["transcription", "whisper.cpp", "tiny"])
            .env("ORBITE_HOME", &folder)
            .output()
            .unwrap();
        let mut chat = Command::new(env!("CARGO_BIN_EXE_orbite"))
            .arg("chat")
            .env("ORBITE_HOME", &folder)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        chat.stdin.take().unwrap().write_all(said.as_bytes()).unwrap();
        (chosen, chat.wait_with_output().unwrap())
    })
    .await
    .unwrap();

    assert!(chosen.status.success(), "{}", shown(&chosen));
    println!("{}", shown(&chosen));
    assert!(shown(&chosen).contains("to listen to 10 seconds of sound"), "{}", shown(&chosen));
    assert!(shown(&chatted).contains("Heard."), "{}", shown(&chatted));
    let asked = thinking.received_requests().await.unwrap();
    let read = String::from_utf8_lossy(&asked[0].body).to_lowercase();
    assert!(read.contains(&word.to_lowercase()), "the model read: {read}");
}
