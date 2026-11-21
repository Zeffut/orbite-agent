//! What an Owner goes through with the web interface: pairing a browser
//! with a code shown on the Host, then holding the Conversation there.

mod support;

use std::path::Path;
use std::time::Duration;

use orbite::{Agent, Interface, Store};
use serde_json::{Value, json};
use support::ScriptedProvider;
use tokio::sync::mpsc;

/// A browser, as far as the interface is concerned.
struct Browser {
    http: reqwest::Client,
    at: String,
    /// What pairing gave it to keep, when it was paired.
    kept: Option<String>,
}

impl Browser {
    fn at(address: std::net::SocketAddr) -> Self {
        Self { http: reqwest::Client::new(), at: format!("http://{address}"), kept: None }
    }

    /// Sends what the interface's own page would.
    async fn posts(&self, path: &str, body: Value) -> reqwest::Response {
        let mut sending =
            self.http.post(format!("{}{path}", self.at)).header("x-orbite", "1").json(&body);
        if let Some(kept) = &self.kept {
            sending = sending.header("cookie", kept);
        }
        sending.send().await.unwrap()
    }

    async fn pairs_with(&mut self, code: &str) -> reqwest::StatusCode {
        let answer = self.posts("/api/pair", json!({ "code": code, "name": "laptop" })).await;
        let status = answer.status();
        if let Some(given) = answer.headers().get("set-cookie") {
            let given = given.to_str().unwrap();
            assert!(given.contains("HttpOnly") && given.contains("SameSite=Strict"), "{given}");
            self.kept = Some(given.split(';').next().unwrap().to_owned());
        }
        status
    }

    async fn says(&self, text: &str) -> reqwest::StatusCode {
        self.posts("/api/say", json!({ "text": text })).await.status()
    }

    /// What the page is shown, from the start or from where it left off.
    async fn watches(&self, after: Option<i64>) -> Result<mpsc::UnboundedReceiver<Value>, u16> {
        let from = after.map(|after| format!("?after={after}")).unwrap_or_default();
        let mut asking = self.http.get(format!("{}/api/events{from}", self.at));
        if let Some(kept) = &self.kept {
            asking = asking.header("cookie", kept);
        }
        let mut answer = asking.send().await.unwrap();
        if !answer.status().is_success() {
            return Err(answer.status().as_u16());
        }
        let (shown, showing) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            let mut read = String::new();
            while let Ok(Some(part)) = answer.chunk().await {
                read.push_str(&String::from_utf8_lossy(&part));
                while let Some(end) = read.find("\n\n") {
                    let event: String = read.drain(..end + 2).collect();
                    let said = event.lines().find_map(|line| line.strip_prefix("data:"));
                    let numbered = event.lines().find_map(|line| line.strip_prefix("id:"));
                    if let Some(mut told) =
                        said.and_then(|said| serde_json::from_str::<Value>(said.trim()).ok())
                    {
                        told["id"] = json!(numbered.and_then(|id| id.trim().parse::<i64>().ok()));
                        if shown.send(told).is_err() {
                            return;
                        }
                    }
                }
            }
        });
        Ok(showing)
    }
}

/// The next thing of `kind` the page is shown.
async fn next(shown: &mut mpsc::UnboundedReceiver<Value>, kind: &str) -> Value {
    loop {
        let told = tokio::time::timeout(Duration::from_secs(10), shown.recv())
            .await
            .unwrap_or_else(|_| panic!("nothing of kind {kind} was shown"))
            .expect("the page was closed");
        if told["kind"] == kind {
            return told;
        }
    }
}

/// An Agent whose only Channel is the interface, and a browser paired with it.
async fn agent_with_a_paired_browser(home: &Path, provider: &ScriptedProvider) -> Browser {
    let mut interface =
        Interface::serving(home, "127.0.0.1:0".parse().unwrap(), "en").await.unwrap();
    let mut browser = Browser::at(interface.address());
    let (provider, workspace) = (provider.clone(), home.join("workspace"));
    tokio::spawn(async move {
        Agent::new(provider, Store::in_memory())
            .working_in(workspace)
            .converse(&mut interface)
            .await
    });
    let code = orbite::pairing_code(home).unwrap();
    assert_eq!(browser.pairs_with(&code).await, 200);
    browser
}

#[tokio::test]
async fn an_owner_pairs_a_browser_and_holds_the_conversation_there() {
    let home = tempfile::tempdir().unwrap();
    let provider = ScriptedProvider::answering(["Hello, I am here."]);
    let browser = agent_with_a_paired_browser(home.path(), &provider).await;
    let mut shown = browser.watches(None).await.unwrap();

    assert_eq!(browser.says("Hi").await, 200);

    assert_eq!(next(&mut shown, "owner").await["text"], "Hi");
    assert_eq!(next(&mut shown, "agent").await["text"], "Hello, I am here.");
}

#[tokio::test]
async fn a_device_that_was_not_paired_sees_nothing_and_sends_nothing() {
    let home = tempfile::tempdir().unwrap();
    let provider = ScriptedProvider::answering(["Never said."]);
    let _paired = agent_with_a_paired_browser(home.path(), &provider).await;
    let mut stranger = Browser::at(_paired.at.trim_start_matches("http://").parse().unwrap());

    assert_eq!(stranger.watches(None).await.err(), Some(401));
    assert_eq!(stranger.says("Let me in").await, 401);
    assert_eq!(stranger.pairs_with("AAAA-AAAA").await, 401);
    // What a device keeps is its own: made up, it lets nobody in.
    stranger.kept = Some("orbite_device=made-up".into());
    assert_eq!(stranger.says("Let me in").await, 401);
    assert_eq!(provider.times_asked(), 0);
}

#[tokio::test]
async fn a_pairing_code_works_once_and_trying_many_voids_them_all() {
    let home = tempfile::tempdir().unwrap();
    let provider = ScriptedProvider::default();
    let first = agent_with_a_paired_browser(home.path(), &provider).await;
    let address = first.at.trim_start_matches("http://").parse().unwrap();

    // Used once: the same code lets no second device in.
    let code = orbite::pairing_code(home.path()).unwrap();
    assert_eq!(Browser::at(address).pairs_with(&code).await, 200);
    assert_eq!(Browser::at(address).pairs_with(&code).await, 401);

    // Guessing is given up on: after a few wrong ones, even the right one is void.
    let code = orbite::pairing_code(home.path()).unwrap();
    for _ in 0..8 {
        assert_eq!(Browser::at(address).pairs_with("ZZZZ-ZZZZ").await, 401);
    }
    assert_eq!(Browser::at(address).pairs_with(&code).await, 401);
    // A fresh code, asked for on the Host, works again.
    let code = orbite::pairing_code(home.path()).unwrap();
    assert_eq!(Browser::at(address).pairs_with(&code).await, 200);
    let names: Vec<String> =
        orbite::interface_devices(home.path()).unwrap().into_iter().map(|one| one.name).collect();
    assert_eq!(names.len(), 3, "{names:?}");
}

#[tokio::test]
async fn an_approval_is_answered_with_a_button_in_the_page() {
    let home = tempfile::tempdir().unwrap();
    let provider = ScriptedProvider::default();
    provider
        .then_acts([("run_command", json!({ "command": "touch made.txt" }))])
        .then(Ok("Made.".into()));
    let browser = agent_with_a_paired_browser(home.path(), &provider).await;
    let mut shown = browser.watches(None).await.unwrap();

    browser.says("Make the file.").await;
    let asked = next(&mut shown, "ask").await;
    assert!(asked["detail"].as_str().unwrap().contains("touch made.txt"), "{asked}");
    assert!(!home.path().join("workspace/made.txt").exists(), "done before the Owner answered");
    let answered =
        browser.posts("/api/answer", json!({ "ask": asked["ask"], "answer": "allow" })).await;
    assert_eq!(answered.status(), 200);

    assert_eq!(next(&mut shown, "settled").await["answer"], "allow");
    assert_eq!(next(&mut shown, "agent").await["text"], "Made.");
    assert!(home.path().join("workspace/made.txt").exists());
}

#[tokio::test]
async fn the_conversation_is_found_again_and_carried_on_from_where_the_page_left_off() {
    let home = tempfile::tempdir().unwrap();
    let provider = ScriptedProvider::answering(["One.", "Two."]);
    let browser = agent_with_a_paired_browser(home.path(), &provider).await;
    let mut shown = browser.watches(None).await.unwrap();
    browser.says("First").await;
    let seen = next(&mut shown, "agent").await;
    assert_eq!(seen["text"], "One.");
    drop(shown);

    // The page is closed; the Conversation goes on.
    browser.says("Second").await;
    tokio::time::sleep(Duration::from_millis(300)).await;

    // Opened again from nothing, everything is there, in order.
    let mut again = browser.watches(None).await.unwrap();
    assert_eq!(next(&mut again, "owner").await["text"], "First");
    assert_eq!(next(&mut again, "agent").await["text"], "One.");
    assert_eq!(next(&mut again, "owner").await["text"], "Second");
    assert_eq!(next(&mut again, "agent").await["text"], "Two.");
    // Reconnecting where it left off, only what it missed is told.
    let mut after = browser.watches(seen["id"].as_i64()).await.unwrap();
    assert_eq!(next(&mut after, "owner").await["text"], "Second");
}

#[tokio::test]
async fn what_another_site_has_a_browser_send_is_refused() {
    let home = tempfile::tempdir().unwrap();
    let provider = ScriptedProvider::answering(["Never said."]);
    let browser = agent_with_a_paired_browser(home.path(), &provider).await;
    let say = |with: &'static [(&'static str, &'static str)]| {
        let mut sending = browser
            .http
            .post(format!("{}/api/say", browser.at))
            .header("cookie", browser.kept.clone().unwrap())
            .json(&json!({ "text": "Do something" }));
        for (name, value) in with {
            sending = sending.header(*name, *value);
        }
        async move { sending.send().await.unwrap().status().as_u16() }
    };

    // Without what only the interface's own page adds.
    assert_eq!(say(&[]).await, 401);
    // From a page of another place, though it adds it.
    assert_eq!(say(&[("x-orbite", "1"), ("origin", "http://elsewhere.example")]).await, 401);
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(provider.times_asked(), 0);
}

#[tokio::test]
async fn a_device_turned_away_is_let_in_no_more() {
    let home = tempfile::tempdir().unwrap();
    let provider = ScriptedProvider::answering(["Hello."]);
    let browser = agent_with_a_paired_browser(home.path(), &provider).await;
    assert_eq!(browser.says("Hi").await, 200);

    assert!(orbite::turn_away(home.path(), "laptop").unwrap());

    assert_eq!(browser.says("Still there?").await, 401);
    assert_eq!(browser.watches(None).await.err(), Some(401));
    assert!(orbite::interface_devices(home.path()).unwrap().is_empty());
}

#[test]
fn only_the_host_itself_and_networks_of_the_owners_own_count_as_private() {
    for (address, private) in [
        ("127.0.0.1:80", true),
        ("192.168.1.20:80", true),
        ("10.0.0.5:80", true),
        // Tailscale and its like.
        ("100.101.102.103:80", true),
        ("[::1]:80", true),
        ("[fd7a:115c:a1e0::1]:80", true),
        ("0.0.0.0:80", false),
        ("203.0.113.9:80", false),
        ("100.200.1.1:80", false),
        ("[2001:db8::1]:80", false),
    ] {
        assert_eq!(orbite::is_private(&address.parse().unwrap()), private, "{address}");
    }
}
