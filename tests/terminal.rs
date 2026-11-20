mod support;

use orbite::{Agent, Store, Terminal};
use support::ScriptedProvider;

#[tokio::test]
async fn a_conversation_typed_in_a_terminal_is_answered_line_by_line() {
    let provider = ScriptedProvider::answering(["Hello.", "Goodbye."]);
    let typed: &[u8] = b"Hi\n\nBye\n";
    let mut shown = Vec::new();

    let mut terminal = Terminal::new(typed, &mut shown);
    Agent::new(provider, Store::in_memory()).converse(&mut terminal).await;

    assert_eq!(String::from_utf8(shown).unwrap(), "> Hello.\n\n> Goodbye.\n\n> ");
}

#[tokio::test]
async fn an_approval_is_asked_and_answered_in_the_terminal() {
    for (typed, created) in
        [("Create it\ny\n", true), ("Create it\nn\n", false), ("Create it\n", false)]
    {
        let workspace = tempfile::tempdir().unwrap();
        let provider = ScriptedProvider::default();
        provider
            .then_acts([("run_command", serde_json::json!({ "command": "touch made.txt" }))])
            .then(Ok("Done.".into()));
        let mut shown = Vec::new();

        let mut terminal = Terminal::new(typed.as_bytes(), &mut shown);
        Agent::new(provider, Store::in_memory())
            .working_in(workspace.path())
            .converse(&mut terminal)
            .await;

        let shown = String::from_utf8(shown).unwrap();
        assert!(shown.contains("touch made.txt"), "{shown}");
        assert!(shown.contains("cannot be undone"), "{shown}");
        assert_eq!(workspace.path().join("made.txt").exists(), created, "typed {typed:?}");
    }
}

#[tokio::test]
async fn the_prompt_is_shown_once_however_often_the_agent_looks_for_input() {
    use orbite::Channel;
    use tokio::io::AsyncWriteExt;
    let (mut keyboard, typed) = tokio::io::duplex(64);
    let mut shown = Vec::new();
    {
        let mut terminal = Terminal::new(tokio::io::BufReader::new(typed), &mut shown);
        // Looking for input and giving up, as the Agent does when something
        // else needs it first, must not show the prompt again.
        for _ in 0..3 {
            let waited = std::time::Duration::from_millis(10);
            assert!(tokio::time::timeout(waited, terminal.receive()).await.is_err());
        }
        keyboard.write_all(b"Hi\n").await.unwrap();
        assert_eq!(terminal.receive().await.as_deref(), Some("Hi"));
    }

    assert_eq!(String::from_utf8(shown).unwrap(), "> ");
}

#[tokio::test]
async fn something_else_typed_at_an_approval_is_taken_as_a_message() {
    let workspace = tempfile::tempdir().unwrap();
    let provider = ScriptedProvider::default();
    provider
        .then_acts([("run_command", serde_json::json!({ "command": "touch made.txt" }))])
        .then(Ok("As you wish.".into()));
    let asked = provider.clone();
    let mut shown = Vec::new();

    let mut terminal = Terminal::new("Create it\nno, call it other.txt\n".as_bytes(), &mut shown);
    Agent::new(provider, Store::in_memory())
        .working_in(workspace.path())
        .converse(&mut terminal)
        .await;

    assert!(!workspace.path().join("made.txt").exists());
    assert_eq!(
        asked.last_asked().last(),
        Some(&orbite::Message::from_owner("no, call it other.txt"))
    );
}

#[tokio::test]
async fn stop_typed_while_a_command_runs_halts_it_and_what_else_was_typed_waits_its_turn() {
    use tokio::io::AsyncWriteExt;
    let workspace = tempfile::tempdir().unwrap();
    let provider = ScriptedProvider::default();
    provider
        .then_acts([(
            "run_command",
            serde_json::json!({ "command": "touch started.txt; sleep 30; touch finished.txt" }),
        )])
        .then(Ok("Four.".into()));
    let (mut typing, typed) = tokio::io::duplex(1024);
    let (shown, mut reading) = tokio::io::duplex(65536);
    let folder = workspace.path().to_owned();
    let running = tokio::spawn(async move {
        let mut terminal = Terminal::new(tokio::io::BufReader::new(typed), shown);
        Agent::new(provider, Store::in_memory()).working_in(folder).converse(&mut terminal).await;
    });

    // Allowed at once; then, while it runs, a question and the order to stop.
    typing.write_all(b"Run the long job.\ny\n").await.unwrap();
    let started = workspace.path().join("started.txt");
    for _ in 0..200 {
        if started.exists() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert!(started.exists(), "the command did not start");
    let asked = std::time::Instant::now();
    typing.write_all(b"What is 2+2?\nstop\n").await.unwrap();
    drop(typing);
    tokio::time::timeout(std::time::Duration::from_secs(10), running)
        .await
        .expect("the command was not stopped")
        .unwrap();

    assert!(asked.elapsed() < std::time::Duration::from_secs(8));
    assert!(!workspace.path().join("finished.txt").exists());
    let mut shown = String::new();
    tokio::io::AsyncReadExt::read_to_string(&mut reading, &mut shown).await.unwrap();
    // What was typed before the order waited, and was answered after it.
    assert!(shown.contains("Four."), "{shown}");
}
