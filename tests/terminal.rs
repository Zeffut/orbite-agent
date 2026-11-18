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
