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
    // Nor one that claims to be where the interface is served encrypted.
    const ELSEWHERE: &[(&str, &str)] = &[
        ("x-orbite", "1"),
        ("origin", "https://elsewhere.example"),
        ("x-forwarded-host", "host.tail1234.ts.net:8377"),
    ];
    assert_eq!(say(ELSEWHERE).await, 401);
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(provider.times_asked(), 0);

    // The page itself, reached through what serves it encrypted, is heard.
    const THROUGH: &[(&str, &str)] = &[
        ("x-orbite", "1"),
        ("origin", "https://host.tail1234.ts.net:8377"),
        ("x-forwarded-host", "host.tail1234.ts.net:8377"),
    ];
    assert_eq!(say(THROUGH).await, 200);
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

impl Browser {
    /// Sends a file ahead of the message it goes with, as the page does.
    async fn uploads(&self, name: &str, content: &'static [u8]) -> reqwest::Response {
        let mut sending = self
            .http
            .post(format!("{}/api/upload?name={}", self.at, name.replace('/', "%2F")))
            .header("x-orbite", "1")
            .body(content);
        if let Some(kept) = &self.kept {
            sending = sending.header("cookie", kept);
        }
        sending.send().await.unwrap()
    }
}

#[tokio::test]
async fn a_file_sent_from_the_page_reaches_the_agent_with_the_words_it_goes_with() {
    let home = tempfile::tempdir().unwrap();
    let provider = ScriptedProvider::answering(["Read."]);
    let browser = agent_with_a_paired_browser(home.path(), &provider).await;
    let mut shown = browser.watches(None).await.unwrap();

    let taken = browser.uploads("../../notes.txt", b"buy milk").await;
    assert_eq!(taken.status(), 200);
    let file = taken.json::<Value>().await.unwrap()["file"].clone();
    let sent = browser.posts("/api/say", json!({ "text": "Read this", "files": [file] })).await;
    assert_eq!(sent.status(), 200);

    // Shown with its name, and fetched by a name nobody can guess.
    let said = next(&mut shown, "owner").await;
    assert_eq!(said["text"], "Read this");
    assert_eq!(said["files"][0]["name"], "notes.txt");
    assert_eq!(next(&mut shown, "agent").await["text"], "Read.");
    // Saved inside the inbox whatever its name tried, and named to the Agent.
    let saved = home.path().join("workspace/inbox/notes.txt");
    assert_eq!(std::fs::read(&saved).unwrap(), b"buy milk");
    let asked = format!("{:?}", provider.last_asked());
    assert!(asked.contains(&format!("Read this\\n[attached: {}]", saved.display())), "{asked}");

    // A file is sent once: its name is no longer good for another message.
    let again = browser.posts("/api/say", json!({ "text": "Again", "files": [file] })).await;
    assert_eq!(again.status(), 410);
}

#[tokio::test]
async fn a_file_taken_back_or_sent_by_a_stranger_is_not_kept() {
    let home = tempfile::tempdir().unwrap();
    let provider = ScriptedProvider::default();
    let browser = agent_with_a_paired_browser(home.path(), &provider).await;
    let stranger = Browser::at(browser.at.trim_start_matches("http://").parse().unwrap());

    assert_eq!(stranger.uploads("virus.sh", b"rm -rf").await.status(), 401);
    assert_eq!(browser.uploads("empty.txt", b"").await.status(), 400);

    let taken = browser.uploads("draft.txt", b"not this one").await;
    let file = taken.json::<Value>().await.unwrap()["file"].as_str().unwrap().to_owned();
    let mut giving_up = browser
        .http
        .delete(format!("{}/api/upload?file={file}", browser.at))
        .header("x-orbite", "1");
    giving_up = giving_up.header("cookie", browser.kept.as_ref().unwrap());
    assert_eq!(giving_up.send().await.unwrap().status(), 200);

    // Nothing of it is left, in the inbox or among what the page can fetch.
    let left = |folder: &str| {
        std::fs::read_dir(home.path().join(folder)).map(|all| all.count()).unwrap_or(0)
    };
    assert_eq!(left("workspace/inbox"), 0);
    assert_eq!(left("interface-files"), 0);
}

#[tokio::test]
async fn asked_in_plain_words_the_agent_lets_a_device_in_once_its_owner_agrees() {
    let home = tempfile::tempdir().unwrap();
    let interface =
        Interface::serving(home.path(), "127.0.0.1:0".parse().unwrap(), "en").await.unwrap();
    let address = interface.address();
    orbite::Config { interface: Some(address.to_string()), ..Default::default() }
        .save(home.path())
        .unwrap();
    let provider = ScriptedProvider::default();
    provider.then_acts([("command", json!({ "command": "/pair" }))]).then(Ok("[silent]".into()));
    let (mut channel, mut owner) = support::memory_channel();
    let (model, folder) = (provider.clone(), home.path().to_owned());
    tokio::spawn(async move {
        Agent::new(model, Store::in_memory()).configured_in(&folder).converse(&mut channel).await
    });

    owner.says("Add my phone.");
    // Who may open the conversation is asked at every level, with a button.
    let asked = owner.is_asked().await;
    assert!(asked.summary.contains("Let a new device in"), "{}", asked.summary);
    assert!(asked.critical);
    owner.allows();
    let told = owner.hears().await;
    let link = format!("http://{address}/#");
    let code: String =
        told[told.find(&link).expect("no link given") + link.len()..].chars().take(9).collect();

    // The code works, and no model was shown it.
    assert_eq!(Browser::at(address).pairs_with(&code).await, 200);
    let shown = format!("{:?}", provider.every_call());
    assert!(!shown.contains(&code), "{shown}");
}

#[tokio::test]
async fn a_device_turned_away_is_told_nothing_more_even_on_a_page_left_open() {
    let home = tempfile::tempdir().unwrap();
    let provider = ScriptedProvider::answering(["Still here."]);
    let first = agent_with_a_paired_browser(home.path(), &provider).await;
    let mut seen_by_first = first.watches(None).await.unwrap();
    let mut other = Browser::at(first.at.trim_start_matches("http://").parse().unwrap());
    let code = orbite::pairing_code(home.path()).unwrap();
    assert_eq!(other.pairs_with(&code).await, 200);

    assert!(orbite::turn_away(home.path(), "laptop").unwrap());
    // The page of the device turned away is closed at the next thing said.
    assert_eq!(first.says("Am I still in?").await, 401);
    // Another device, still let in, says something: the first is not shown it.
    assert_eq!(other.says("Hello").await, 200);
    let told = tokio::time::timeout(Duration::from_secs(2), seen_by_first.recv()).await;
    assert!(matches!(told, Ok(None) | Err(_)), "{told:?}");
}

/// Where the stand-in for `orbite interface on` writes, in the test below.
static SWITCHED_IN: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();

/// Does what `orbite interface on|off` does where Tailscale serves it.
fn switch(on: bool) -> Result<(), String> {
    let home = SWITCHED_IN.get().unwrap();
    let mut config = orbite::Config::load(home).unwrap();
    config.interface = on.then(|| "127.0.0.1:8377".to_owned());
    config.interface_reached_at = on.then(|| "https://host.tail1234.ts.net:8377".to_owned());
    config.save(home).map_err(|error| error.to_string())
}

#[tokio::test]
async fn asked_in_plain_words_the_agent_turns_the_interface_on_then_lets_a_device_in() {
    let home = tempfile::tempdir().unwrap();
    SWITCHED_IN.set(home.path().to_owned()).unwrap();
    let provider = ScriptedProvider::default();
    provider
        .then_acts([("command", json!({ "command": "/interface on" }))])
        .then(Ok("[silent]".into()))
        .then_acts([("command", json!({ "command": "/pair" }))])
        .then(Ok("[silent]".into()));
    let (mut channel, mut owner) = support::memory_channel();
    let (model, folder) = (provider.clone(), home.path().to_owned());
    tokio::spawn(async move {
        Agent::new(model, Store::in_memory())
            .configured_in(&folder)
            .switching_the_interface_with(switch)
            .converse(&mut channel)
            .await
    });

    owner.says("I want to talk to you from my phone's browser.");
    let asked = owner.is_asked().await;
    assert!(asked.summary.contains("Turn the web interface on"), "{}", asked.summary);
    owner.allows();
    let told = owner.hears().await;
    assert!(told.contains("https://host.tail1234.ts.net:8377"), "{told}");
    assert!(told.contains("encrypted"), "{told}");

    owner.says("Add my phone.");
    owner.is_asked().await;
    owner.allows();
    let told = owner.hears().await;
    assert!(told.contains("https://host.tail1234.ts.net:8377/#"), "{told}");
}

#[tokio::test]
async fn the_agents_screen_is_shown_to_the_devices_let_in_and_to_nobody_else() {
    use orbite::Channel;
    let home = tempfile::tempdir().unwrap();
    let mut interface =
        Interface::serving(home.path(), "127.0.0.1:0".parse().unwrap(), "en").await.unwrap();
    let mut browser = Browser::at(interface.address());
    let code = orbite::pairing_code(home.path()).unwrap();
    assert_eq!(browser.pairs_with(&code).await, 200);
    let fetch = |kept: Option<String>| {
        let mut asking = browser.http.get(format!("{}/api/screen", browser.at));
        if let Some(kept) = kept {
            asking = asking.header("cookie", kept);
        }
        async move { asking.send().await.unwrap() }
    };
    // Nothing to see before the Agent has a screen.
    assert_eq!(fetch(browser.kept.clone()).await.status(), 404);
    let mut shown = browser.watches(None).await.unwrap();

    let picture = home.path().join("seen.png");
    std::fs::write(&picture, b"\x89PNG as the screen is").unwrap();
    interface.screen(&picture).await;

    assert_eq!(next(&mut shown, "screen").await["seen"], 1);
    let seen = fetch(browser.kept.clone()).await;
    assert_eq!(seen.headers()["content-type"], "image/png");
    assert_eq!(seen.bytes().await.unwrap().as_ref(), b"\x89PNG as the screen is");
    assert_eq!(fetch(None).await.status(), 401);
}

#[tokio::test]
async fn a_page_that_reaches_the_interface_under_a_name_of_its_own_is_answered_nothing() {
    let home = tempfile::tempdir().unwrap();
    let provider = ScriptedProvider::default();
    let browser = agent_with_a_paired_browser(home.path(), &provider).await;
    let code = orbite::pairing_code(home.path()).unwrap();
    let asking = |host: &'static str| {
        let sending = browser
            .http
            .post(format!("{}/api/pair", browser.at))
            .header("x-orbite", "1")
            .header("host", host)
            .header("origin", format!("http://{host}"))
            .json(&json!({ "code": "ZZZZ-ZZZZ", "name": "elsewhere" }));
        async move { sending.send().await.unwrap().status().as_u16() }
    };

    // Another site's name, made to lead here: refused before anything is looked at.
    for _ in 0..10 {
        assert_eq!(asking("evil.example:8377").await, 421);
    }
    // It could not even use up the tries: the Owner's code still works.
    assert_eq!(
        Browser::at(browser.at.trim_start_matches("http://").parse().unwrap())
            .pairs_with(&code)
            .await,
        200
    );
    // The Host's own names are answered.
    assert_eq!(asking("localhost:8377").await, 401);
}

#[tokio::test]
async fn a_device_is_named_with_what_its_owner_can_read_and_type() {
    let home = tempfile::tempdir().unwrap();
    let interface =
        Interface::serving(home.path(), "127.0.0.1:0".parse().unwrap(), "en").await.unwrap();
    let pair = |name: &'static str| {
        let browser = Browser::at(interface.address());
        let code = orbite::pairing_code(home.path()).unwrap();
        async move { browser.posts("/api/pair", json!({ "code": code, "name": name })).await.status() }
    };

    assert_eq!(pair("Mac · Safari").await, 200);
    assert_eq!(pair("\u{200b}\u{202e}").await, 200);
    assert_eq!(pair("  Mac  ·  Safari ").await, 200);
    assert_eq!(pair("<b>Mac</b>").await, 200);

    let mut names: Vec<String> =
        orbite::interface_devices(home.path()).unwrap().into_iter().map(|one| one.name).collect();
    names.sort();
    assert_eq!(names, ["Mac · Safari", "Mac · Safari 2", "bMacb", "browser"]);
    // One turned away leaves the other, and its name is free again for one device.
    assert!(orbite::turn_away(home.path(), "Mac · Safari").unwrap());
    assert_eq!(pair("Mac · Safari").await, 200);
    assert_eq!(orbite::interface_devices(home.path()).unwrap().len(), 4);
}
