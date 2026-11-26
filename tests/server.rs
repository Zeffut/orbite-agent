//! What an Owner observes of a server that holds several Agents: devices
//! are let in to it once, Agents are made and removed there, and what a
//! device asks of one Agent reaches that Agent and no other.

use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use wiremock::matchers::{header_exists, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// An engine that runs nothing: every Agent it "starts" is one stand-in,
/// which answers what an Agent's interface would.
#[derive(Clone)]
struct Pretending {
    at: String,
    /// Where the Agents were told the server is.
    server_at: Arc<Mutex<String>>,
    started: Arc<Mutex<Vec<(String, String, String)>>>,
    removed: Arc<Mutex<Vec<String>>>,
}

impl orbite::Engine for Pretending {
    fn start(&self, id: &str, given: &[(String, String)]) -> Result<(), String> {
        let of = |name: &str| {
            given
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.clone())
                .unwrap_or_default()
        };
        if of("ORBITE_AGENT_NAME") == "Refused" {
            return Err("no room on this machine".into());
        }
        *self.server_at.lock().unwrap() = of("ORBITE_SERVER_AT");
        self.started.lock().unwrap().push((
            id.into(),
            of("ORBITE_AGENT_NAME"),
            of("ORBITE_SERVER_TOKEN"),
        ));
        Ok(())
    }
    fn seen_from_agents(&self) -> Option<std::net::IpAddr> {
        Some([127, 0, 0, 1].into())
    }
    fn reached_at(&self, _id: &str) -> Result<String, String> {
        Ok(self.at.clone())
    }
    fn remove(&self, id: &str) -> Result<(), String> {
        self.removed.lock().unwrap().push(id.into());
        Ok(())
    }
}

#[tokio::test]
async fn devices_are_let_in_once_and_agents_are_made_reached_and_removed_there() {
    let agent = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/me"))
        .and(header_exists("x-orbite-server"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "agent": "Echo" })))
        .mount(&agent)
        .await;
    let home = tempfile::tempdir().unwrap();
    // The Owner's Providers, as the server holds them: one that says what
    // it was asked, and what that cost.
    let thinks: orbite::Thinks = Arc::new(|asked: orbite::Asked| {
        Box::pin(async move {
            Ok(orbite::Reply {
                text: format!("asked: {}", asked.instructions),
                actions: Vec::new(),
                kept: None,
                usage: Some(orbite::Usage {
                    by: "anthropic".into(),
                    model: "a-model".into(),
                    input: 12,
                    cached: 0,
                    output: 3,
                }),
            })
        })
    });
    let engine = Pretending {
        at: agent.uri(),
        server_at: Arc::default(),
        started: Arc::default(),
        removed: Arc::default(),
    };
    let server = orbite::Server::serving(
        home.path(),
        "127.0.0.1:0".parse().unwrap(),
        engine.clone(),
        Vec::new(),
        Some(thinks),
        0,
    )
    .await
    .unwrap();
    let at = format!("http://{}", server.address());
    let http = reqwest::Client::new();

    // A stranger sees no Agent, makes none, and reaches none.
    assert_eq!(http.get(format!("{at}/api/agents")).send().await.unwrap().status(), 401);
    let made = http
        .post(format!("{at}/api/agents"))
        .header("x-orbite", "1")
        .json(&json!({ "name": "Echo" }))
        .send()
        .await
        .unwrap();
    assert_eq!(made.status(), 401);
    assert_eq!(http.get(format!("{at}/a/anything/api/me")).send().await.unwrap().status(), 401);

    // A device is let in to the server, once.
    let code = orbite::pairing_code(home.path()).unwrap();
    let paired = http
        .post(format!("{at}/api/pair"))
        .header("x-orbite", "1")
        .json(&json!({ "code": code, "name": "phone" }))
        .send()
        .await
        .unwrap();
    let kept =
        paired.headers()["set-cookie"].to_str().unwrap().split(';').next().unwrap().to_owned();

    // It makes two Agents, each its own.
    let make = |name: &'static str| {
        let (http, at, kept) = (http.clone(), at.clone(), kept.clone());
        async move {
            http.post(format!("{at}/api/agents"))
                .header("x-orbite", "1")
                .header("cookie", &kept)
                .json(&json!({ "name": name }))
                .send()
                .await
                .unwrap()
        }
    };
    let echo: Value = make("Echo").await.json().await.unwrap();
    let nova: Value = make("Nova").await.json().await.unwrap();
    assert_ne!(echo["id"], nova["id"]);
    let listed: Value = http
        .get(format!("{at}/api/agents"))
        .header("cookie", &kept)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let names: Vec<&str> = listed["agents"]
        .as_array()
        .unwrap()
        .iter()
        .map(|agent| agent["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["Echo", "Nova"]);
    // Each was started with something of its own to know the server by.
    let started = engine.started.lock().unwrap().clone();
    assert_eq!(started.len(), 2);
    assert_ne!(started[0].2, started[1].2);
    // What an Agent knows the server by is never told to a device.
    assert!(!listed.to_string().contains(&started[0].2));

    // An Agent holds no key: it thinks through the server, which knows it
    // by what it was given, asks the Owner's Providers, and counts the cost.
    {
        use orbite::Provider;
        let server_at = engine.server_at.lock().unwrap().clone();
        let asking = |token: &str| orbite::Served::at(server_at.clone(), token.to_owned());
        let conversation = [orbite::Message::from_owner("Hello")];
        let request = orbite::Request {
            instructions: "be brief",
            conversation: &conversation,
            actions: &[],
            images: &[],
            light: false,
        };
        let reply = asking(&started[0].2).reply(request).await.unwrap();
        assert_eq!(reply.text, "asked: be brief");
        assert_eq!(reply.usage.unwrap().by, "anthropic");
        // Whoever is not one of the server's Agents is asked nothing for.
        assert!(asking("not-an-agent-of-this-server-at-all").reply(request).await.is_err());
        let spent = orbite::Store::open(home.path().join("orbite.db")).unwrap();
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        assert_eq!(spent.tokens_used_on(&today).unwrap(), [("anthropic".to_owned(), 15)]);
    }

    // One that does not start leaves nothing behind.
    assert_eq!(make("Refused").await.status(), 502);
    assert_eq!(engine.removed.lock().unwrap().len(), 1);

    // What the device asks of one Agent is carried to it, with who asks.
    let id = echo["id"].as_str().unwrap();
    let me: Value = http
        .get(format!("{at}/a/{id}/api/me"))
        .header("cookie", &kept)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(me["agent"], "Echo");
    let carried = &agent.received_requests().await.unwrap()[0];
    assert_eq!(carried.headers["x-orbite-server"].to_str().unwrap(), started[0].2);
    assert_eq!(carried.headers["x-orbite-device"], "phone");
    // What the device is known by stays with the server.
    assert!(carried.headers.get("cookie").is_none());

    // Removed, an Agent is gone with what it kept, and reached no more.
    let gone = http
        .delete(format!("{at}/api/agents/{id}"))
        .header("x-orbite", "1")
        .header("cookie", &kept)
        .send()
        .await
        .unwrap();
    assert_eq!(gone.status(), 200);
    assert!(engine.removed.lock().unwrap().contains(&id.to_owned()));
    let after =
        http.get(format!("{at}/a/{id}/api/me")).header("cookie", &kept).send().await.unwrap();
    assert_eq!(after.status(), 404);
}

fn pretending(at: String) -> Pretending {
    Pretending { at, server_at: Arc::default(), started: Arc::default(), removed: Arc::default() }
}

#[tokio::test]
async fn the_server_serves_the_page_and_starts_again_the_agents_it_holds() {
    let agent = MockServer::start().await;
    // Coming back from a service: the Agent says how it went, in so many words.
    Mock::given(method("GET"))
        .and(path("/signed-in"))
        .and(header_exists("x-orbite-server"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({ "well": true, "said": "Notion <script>alert(1)</script> is connected." }),
        ))
        .mount(&agent)
        .await;
    let home = tempfile::tempdir().unwrap();
    std::fs::write(
        home.path().join("config.json"),
        r#"{ "language": "fr", "interface_reached_at": "https://mine.example:8377" }"#,
    )
    .unwrap();
    // An Agent held from before: the server starts it again as it starts.
    let held = orbite::Held::new("Echo", &[]).unwrap();
    orbite::keep_agents(home.path(), std::slice::from_ref(&held)).unwrap();
    let engine = pretending(agent.uri());
    let server = orbite::Server::serving(
        home.path(),
        "127.0.0.1:0".parse().unwrap(),
        engine.clone(),
        Vec::new(),
        None,
        0,
    )
    .await
    .unwrap();
    let at = format!("http://{}", server.address());
    let http = reqwest::Client::new();

    for _ in 0..100 {
        if !engine.started.lock().unwrap().is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let started = engine.started.lock().unwrap().clone();
    assert_eq!(started.len(), 1, "{started:?}");
    assert_eq!((started[0].0.as_str(), started[0].1.as_str()), (held.id.as_str(), "Echo"));
    assert_eq!(started[0].2, held.token());

    // The interface's own page, and what it is made of.
    let page = http.get(&at).send().await.unwrap();
    assert_eq!(page.status(), 200);
    assert!(page.text().await.unwrap().contains("/app.js"));
    assert_eq!(http.get(format!("{at}/desk.js")).send().await.unwrap().status(), 200);
    // A stranger is told this is a server, in its language, and nothing else.
    let me = http.get(format!("{at}/api/me")).send().await.unwrap();
    assert_eq!(me.status(), 401);
    let told: Value = me.json().await.unwrap();
    assert_eq!((told["server"].as_bool(), told["language"].as_str()), (Some(true), Some("fr")));
    // Under a name that is not its own, nothing is answered.
    let elsewhere = http.get(&at).header("host", "elsewhere.example").send().await.unwrap();
    assert_eq!(elsewhere.status(), 421);

    // Back from a service, with no device to show: the page is the
    // server's own, and what the Agent said is shown as words.
    let back =
        http.get(format!("{at}/a/{}/signed-in?code=abc&state=xyz", held.id)).send().await.unwrap();
    assert_eq!(back.status(), 200);
    assert!(back.headers()["content-type"].to_str().unwrap().starts_with("text/html"));
    let shown = back.text().await.unwrap();
    assert!(shown.contains("✓") && shown.contains("Tu peux revenir"), "{shown}");
    assert!(!shown.contains("<script>") && shown.contains("&lt;script&gt;"), "{shown}");
    // For an Agent that is not this server's, it did not go through.
    let none = http.get(format!("{at}/a/nobody/signed-in?code=abc")).send().await.unwrap();
    assert!(none.text().await.unwrap().contains("✗"));
}

#[test]
fn an_installation_is_told_apart_into_what_is_its_agents_and_what_is_the_servers() {
    let home = tempfile::tempdir().unwrap();
    let home = home.path();
    std::fs::write(
        home.join("config.json"),
        r#"{ "language": "fr", "provider": { "name": "openai", "model": "a-model" },
             "light_model": "a-light-one", "agent_name": "Echo", "agent_colour": "red",
             "interface": "127.0.0.1:8377", "interface_reached_at": "https://mine.example:8377" }"#,
    )
    .unwrap();
    let secrets = orbite::Secrets::kept_in(home);
    secrets.set(orbite::PROVIDER_KEY, "the-provider-key").unwrap();
    secrets.set("ORBITE_CHATGPT", "the-plan-sign-in").unwrap();
    secrets.set("WEBMAIL_PASSWORD", "the-agents-own").unwrap();
    std::fs::create_dir_all(home.join("memory")).unwrap();
    std::fs::write(home.join("memory/a-note.md"), "likes tea").unwrap();
    std::fs::write(home.join("mcp.json"), "{}").unwrap();
    // A device let in, and something said.
    let code = orbite::pairing_code(home).unwrap();
    assert_eq!(code.len(), 9);

    // The Agent's copy: everything of its own, and no key to think with.
    let copy = tempfile::tempdir().unwrap();
    let agent = copy.path().join("agent");
    let saved = copy.path().join("saved.orbite");
    orbite::export(home, &saved).unwrap();
    orbite::restore(&saved, &agent).unwrap();
    orbite::as_the_agent(&agent).unwrap();
    let its = orbite::Secrets::kept_in(&agent);
    assert_eq!(its.names().unwrap(), ["WEBMAIL_PASSWORD"]);
    assert_eq!(its.get("WEBMAIL_PASSWORD").unwrap().as_deref(), Some("the-agents-own"));
    let config = orbite::Config::load(&agent).unwrap();
    assert!(config.provider.is_none() && config.light_model.is_none());
    assert!(config.interface.is_none() && config.interface_reached_at.is_none());
    assert_eq!(config.agent_name.as_deref(), Some("Echo"));
    assert_eq!(config.automatic_updates, Some(false));
    assert_eq!(std::fs::read_to_string(agent.join("memory/a-note.md")).unwrap(), "likes tea");

    // The server's: who the Agents think with and how it is reached, and
    // nothing an Agent kept.
    orbite::as_the_server(home).unwrap();
    let mut left = orbite::Secrets::kept_in(home).names().unwrap();
    left.sort();
    assert_eq!(left, ["ORBITE_CHATGPT", orbite::PROVIDER_KEY]);
    let config = orbite::Config::load(home).unwrap();
    assert_eq!(config.provider.unwrap().model, "a-model");
    assert_eq!(config.interface_reached_at.as_deref(), Some("https://mine.example:8377"));
    assert!(config.agent_name.is_none() && config.agent_colour.is_none());
    assert!(!home.join("memory").exists() && !home.join("mcp.json").exists());
    assert!(!home.join("orbite.db").exists());
    assert!(home.join("interface.db").exists() && home.join("secrets.key").exists());
}

#[tokio::test]
async fn the_model_is_the_servers_and_is_chosen_from_the_settings_of_any_agent() {
    let agent = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/settings"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "settings": [{ "key": "name", "value": "Echo", "choices": [] },
                         { "key": "model", "value": "none", "choices": [] }],
            "models": [], "usual": null,
        })))
        .mount(&agent)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/settings"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "asked": "the agent" })))
        .mount(&agent)
        .await;
    let home = tempfile::tempdir().unwrap();
    std::fs::write(
        home.path().join("config.json"),
        r#"{ "provider": { "name": "openai", "model": "a-model" } }"#,
    )
    .unwrap();
    let held = orbite::Held::new("Echo", &[]).unwrap();
    orbite::keep_agents(home.path(), std::slice::from_ref(&held)).unwrap();
    let server = orbite::Server::serving(
        home.path(),
        "127.0.0.1:0".parse().unwrap(),
        pretending(agent.uri()),
        Vec::new(),
        None,
        0,
    )
    .await
    .unwrap();
    let at = format!("http://{}/a/{}/api/settings", server.address(), held.id);
    let http = reqwest::Client::new();
    let code = orbite::pairing_code(home.path()).unwrap();
    let paired = http
        .post(format!("http://{}/api/pair", server.address()))
        .header("x-orbite", "1")
        .json(&json!({ "code": code, "name": "phone" }))
        .send()
        .await
        .unwrap();
    let kept =
        paired.headers()["set-cookie"].to_str().unwrap().split(';').next().unwrap().to_owned();

    // The Agent's settings, with the server's model in force and those to choose from.
    let told: Value =
        http.get(&at).header("cookie", &kept).send().await.unwrap().json().await.unwrap();
    let of = |told: &Value, key: &str| {
        told["settings"].as_array().unwrap().iter().find(|one| one["key"] == key).unwrap()["value"]
            .clone()
    };
    assert_eq!((of(&told, "name"), of(&told, "model")), (json!("Echo"), json!("a-model")));
    assert_eq!(of(&told, "light"), json!("off"));
    assert!(told["models"].as_array().unwrap().iter().any(|model| model["value"] == "a-model"));

    // Chosen there, it is the server's that changes; the rest is the Agent's.
    let set = |key: &'static str, value: &'static str| {
        let (http, at, kept) = (http.clone(), at.clone(), kept.clone());
        async move {
            http.post(&at)
                .header("cookie", kept)
                .header("x-orbite", "1")
                .json(&json!({ "key": key, "value": value }))
                .send()
                .await
                .unwrap()
        }
    };
    let changed = set("model", "another-model").await;
    assert_eq!(changed.status(), 200);
    assert_eq!(changed.json::<Value>().await.unwrap()["set"], true);
    let config = orbite::Config::load(home.path()).unwrap();
    assert_eq!(config.provider.unwrap().model, "another-model");
    let named = set("name", "Nova").await;
    assert_eq!(named.json::<Value>().await.unwrap()["asked"], "the agent");
    // Not by a stranger, and not by another site's page.
    let stranger = http
        .post(&at)
        .header("x-orbite", "1")
        .json(&json!({ "key": "model", "value": "theirs" }))
        .send()
        .await
        .unwrap();
    assert_eq!(stranger.status(), 401);
    let elsewhere = http
        .post(&at)
        .header("cookie", &kept)
        .json(&json!({ "key": "model", "value": "theirs" }))
        .send()
        .await
        .unwrap();
    assert_eq!(elsewhere.status(), 403);
    assert_eq!(orbite::Config::load(home.path()).unwrap().provider.unwrap().model, "another-model");
}
