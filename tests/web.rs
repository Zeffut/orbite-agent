//! What an Owner observes when the Agent reads and searches the web.

mod support;

use orbite::{Agent, Message, Store};
use serde_json::json;
use support::{Owner, ScriptedProvider, memory_channel};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn agent_searching_at(search: &str, provider: &ScriptedProvider) -> Owner {
    let (mut channel, owner) = memory_channel();
    let provider = provider.clone();
    let search = search.to_owned();
    tokio::spawn(async move {
        Agent::new(provider, Store::in_memory()).searching_at(search).converse(&mut channel).await
    });
    owner
}

fn outcome(provider: &ScriptedProvider) -> String {
    match provider.last_asked().pop() {
        Some(Message::Outcome { text, .. }) => text,
        other => panic!("the model was not given an Outcome but {other:?}"),
    }
}

async fn serving(route: &str, response: ResponseTemplate) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path(route)).respond_with(response).mount(&server).await;
    server
}

fn html(body: &str) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_raw(body.to_owned(), "text/html; charset=utf-8")
}

/// Has the Agent read `url`, on the Owner's own network, with their Approval.
async fn read(url: &str) -> String {
    let provider = ScriptedProvider::default();
    provider.then_acts([("read_page", json!({ "url": url }))]).then(Ok("Read.".into()));
    let mut owner = agent_searching_at("http://127.0.0.1:1", &provider);
    owner.says("Read it.");
    owner.is_asked().await;
    owner.allows();
    owner.hears().await;
    outcome(&provider)
}

const ARTICLE: &str = r#"<!doctype html>
<html><head><title>Tides explained</title>
<style>body { color: red }</style>
<script>window.tracking = "do-not-show";</script></head>
<body>
<nav><a href="/">Home</a> <a href="/about">About us</a></nav>
<header>Site banner</header>
<main>
  <h1>Why the sea rises twice a day</h1>
  <p>The Moon pulls on the oceans, and the Earth turns under the bulge.</p>
  <p>See the <a href="https://example.org/moon">Moon page</a> for more.</p>
  <ul><li>High tide</li><li>Low tide</li></ul>
</main>
<aside>Subscribe to our newsletter</aside>
<footer>Copyright notice</footer>
</body></html>"#;

#[tokio::test]
async fn a_page_is_given_to_the_model_as_its_readable_content() {
    let server = serving("/tides", html(ARTICLE)).await;

    let page = read(&format!("{}/tides", server.uri())).await;

    assert!(page.contains("Tides explained"), "{page}");
    assert!(page.contains("Why the sea rises twice a day"), "{page}");
    assert!(
        page.contains("The Moon pulls on the oceans, and the Earth turns under the bulge."),
        "{page}"
    );
    assert!(page.contains("High tide") && page.contains("Low tide"), "{page}");
    assert!(page.contains("https://example.org/moon"), "links are kept: {page}");
    for noise in ["do-not-show", "color: red", "About us", "Site banner", "newsletter", "Copyright"]
    {
        assert!(!page.contains(noise), "{noise:?} should have been left out: {page}");
    }
}

#[tokio::test]
async fn a_page_without_a_main_part_is_still_read() {
    let server = serving(
        "/plain",
        html("<html><body><p>Just one paragraph of text here.</p></body></html>"),
    )
    .await;

    let page = read(&format!("{}/plain", server.uri())).await;

    assert!(page.contains("Just one paragraph of text here."), "{page}");
}

#[tokio::test]
async fn text_that_is_not_a_web_page_is_given_as_it_is() {
    let server = serving(
        "/data.json",
        ResponseTemplate::new(200).set_body_raw(r#"{"temperature": 21}"#, "application/json"),
    )
    .await;

    let page = read(&format!("{}/data.json", server.uri())).await;

    assert!(page.contains(r#"{"temperature": 21}"#), "{page}");
}

#[tokio::test]
async fn something_that_is_not_text_is_named_rather_than_dumped() {
    let server = serving(
        "/photo.png",
        ResponseTemplate::new(200)
            .set_body_raw(vec![0x89, b'P', b'N', b'G', 0, 1, 2, 3], "image/png"),
    )
    .await;

    let page = read(&format!("{}/photo.png", server.uri())).await;

    assert!(page.contains("image/png"), "{page}");
    assert!(page.len() < 300, "{page}");
}

#[tokio::test]
async fn a_very_long_page_is_cut_and_the_model_is_told() {
    let long = format!(
        "<html><body><main>{}</main></body></html>",
        "<p>A paragraph of the article.</p>".repeat(5_000)
    );
    let server = serving("/long", html(&long)).await;

    let page = read(&format!("{}/long", server.uri())).await;

    assert!(page.len() < 40_000, "{} bytes reached the model", page.len());
    assert!(page.contains("cut"), "the model is told the page goes on");
}

#[tokio::test]
async fn a_page_that_is_not_found_is_reported_as_such() {
    let server = serving("/gone", ResponseTemplate::new(404).set_body_string("Not here")).await;

    let page = read(&format!("{}/gone", server.uri())).await;

    assert!(page.contains("404"), "{page}");
}

#[tokio::test]
async fn a_page_built_by_scripts_is_reported_as_needing_a_browser() {
    let shell = r#"<html><head><title>App</title></head><body><div id="root"></div>
        <script src="/app.js"></script><script>boot()</script></body></html>"#;
    let server = serving("/app", html(shell)).await;

    let page = read(&format!("{}/app", server.uri())).await;

    assert!(page.contains("browser"), "{page}");
}

#[tokio::test]
async fn addresses_on_the_owners_own_network_wait_for_an_approval() {
    for url in [
        "http://localhost:8080/admin",
        "http://127.0.0.1/",
        "http://10.0.0.1/",
        "http://192.168.1.1/",
        "http://172.16.5.4/",
        "http://100.100.1.1/",
        "http://169.254.169.254/latest/meta-data",
        "http://[::1]/",
        "http://[fd00::1]/",
        "http://nas.local/",
        "http://router/",
        "http://0.0.0.0/",
    ] {
        let provider = ScriptedProvider::default();
        provider.then_acts([("read_page", json!({ "url": url }))]).then(Ok("Ok.".into()));
        let mut owner = agent_searching_at("http://127.0.0.1:1", &provider);

        owner.says("Read it.");
        let approval = tokio::time::timeout(std::time::Duration::from_secs(2), owner.is_asked())
            .await
            .unwrap_or_else(|_| panic!("{url} was read without asking"));
        owner.denies();
        owner.hears().await;

        assert!(approval.detail.contains(url), "{approval:?}");
        assert!(outcome(&provider).contains("denied"));
    }
}

#[tokio::test]
async fn public_addresses_are_read_without_asking() {
    let provider = ScriptedProvider::default();
    provider
        .then_acts([("read_page", json!({ "url": "https://orbite-test.invalid/page" }))])
        .then(Ok("Unreachable.".into()));
    let mut owner = agent_searching_at("http://127.0.0.1:1", &provider);

    owner.says("Read it.");
    owner.hears().await;

    assert!(owner.was_asked_nothing());
    assert!(outcome(&provider).contains("Could not"), "{}", outcome(&provider));
}

#[tokio::test]
async fn what_is_not_a_web_address_is_refused() {
    for url in ["file:///etc/passwd", "ftp://example.org/x", "not a url"] {
        let provider = ScriptedProvider::default();
        provider.then_acts([("read_page", json!({ "url": url }))]).then(Ok("Ok.".into()));
        let mut owner = agent_searching_at("http://127.0.0.1:1", &provider);

        owner.says("Read it.");
        owner.hears().await;

        assert!(owner.was_asked_nothing());
        assert!(outcome(&provider).contains("web address"), "{}", outcome(&provider));
    }
}

/// A results page shaped like the search engine's.
const RESULTS: &str = r#"<html><body><table>
<tr><td valign="top">1.&nbsp;</td><td>
  <a rel="nofollow" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Frust%2Dlang.org%2F&amp;rut=abc" class='result-link'>Rust Programming Language</a></td></tr>
<tr><td>&nbsp;</td><td class='result-snippet'><b>Rust</b> is a fast, reliable &amp; productive language.</td></tr>
<tr><td>&nbsp;</td><td><span class='link-text'>rust-lang.org</span></td></tr>
<tr><td valign="top">2.&nbsp;</td><td>
  <a rel="nofollow" href="https://en.wikipedia.org/wiki/Rust_(programming_language)" class='result-link'>Rust - Wikipedia</a></td></tr>
<tr><td>&nbsp;</td><td class='result-snippet'>A general-purpose language.</td></tr>
</table></body></html>"#;

async fn search(server: &MockServer, query: &str) -> String {
    let provider = ScriptedProvider::default();
    provider.then_acts([("search_web", json!({ "query": query }))]).then(Ok("Found.".into()));
    let mut owner = agent_searching_at(&server.uri(), &provider);
    owner.says("Search.");
    owner.hears().await;
    assert!(owner.was_asked_nothing(), "searching needs no Approval");
    outcome(&provider)
}

#[tokio::test]
async fn a_search_gives_titles_addresses_and_excerpts() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(query_param("q", "rust language"))
        .respond_with(html(RESULTS))
        .mount(&server)
        .await;

    let results = search(&server, "rust language").await;

    assert_eq!(
        results,
        "1. Rust Programming Language\n   https://rust-lang.org/\n   Rust is a fast, reliable & productive language.\n\
         2. Rust - Wikipedia\n   https://en.wikipedia.org/wiki/Rust_(programming_language)\n   A general-purpose language."
    );
}

#[tokio::test]
async fn a_search_that_finds_nothing_says_so() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(html("<html><body>No results.</body></html>"))
        .mount(&server)
        .await;

    let results = search(&server, "zzzzqqqq").await;

    assert!(results.contains("No result"), "{results}");
}

#[tokio::test]
async fn a_search_engine_that_fails_is_explained() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).respond_with(ResponseTemplate::new(503)).mount(&server).await;

    let results = search(&server, "anything").await;

    assert!(results.contains("search") && results.contains("503"), "{results}");
}
