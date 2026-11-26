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
    started: Arc<Mutex<Vec<(String, String, String)>>>,
    removed: Arc<Mutex<Vec<String>>>,
}

impl orbite::Engine for Pretending {
    fn start(&self, id: &str, name: &str, token: &str) -> Result<(), String> {
        if name == "Refused" {
            return Err("no room on this machine".into());
        }
        self.started.lock().unwrap().push((id.into(), name.into(), token.into()));
        Ok(())
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
    let engine = Pretending { at: agent.uri(), started: Arc::default(), removed: Arc::default() };
    let server = orbite::Server::serving(
        home.path(),
        "127.0.0.1:0".parse().unwrap(),
        engine.clone(),
        Vec::new(),
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
