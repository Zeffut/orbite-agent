mod support;

use orbite::{Agent, Store, Terminal};
use support::ScriptedProvider;

#[tokio::test]
async fn a_conversation_typed_in_a_terminal_is_answered_line_by_line() {
    let provider = ScriptedProvider::answering(["Hello.", "Goodbye."]);
    let typed: &[u8] = b"Hi\n\nBye\n";
    let mut shown = Vec::new();

    let mut terminal = Terminal::new(typed, &mut shown);
    Agent::new(provider, Store::in_memory())
        .converse(&mut terminal)
        .await;

    assert_eq!(
        String::from_utf8(shown).unwrap(),
        "> Hello.\n\n> Goodbye.\n\n> "
    );
}
