//! Reading and searching the web without a browser: fast, and enough for
//! most pages. Pages that only exist once their scripts have run are
//! reported as such, for the Computer to handle.

use std::net::IpAddr;
use std::time::Duration;

use scraper::{ElementRef, Html, Node, Selector};
use url::{Host, Url};

/// How much of a page is downloaded.
const DOWNLOAD_LIMIT: usize = 3_000_000;
/// How much of a page's text is passed on to the model.
const PAGE_KEPT: usize = 24_000;
/// How many search results are passed on to the model.
const RESULTS_KEPT: usize = 8;
/// Parts of a page that are not its content.
const NOISE: &[&str] = &[
    "script", "style", "noscript", "template", "svg", "iframe", "nav", "header", "footer", "aside",
    "form", "button",
];

pub(crate) struct Web {
    http: reqwest::Client,
    /// The same, speaking the older way of the web: a few sites cut off
    /// the newer one when it is not a browser that speaks it.
    older: reqwest::Client,
    /// Address of the search engine's results page.
    pub(crate) search_at: String,
}

impl Web {
    pub(crate) fn new() -> Self {
        let client = |the_older_way: bool| {
            let building = reqwest::Client::builder()
                .user_agent(
                    "Mozilla/5.0 (X11; Linux x86_64; rv:130.0) Gecko/20100101 Firefox/130.0",
                )
                .timeout(Duration::from_secs(30))
                .connect_timeout(Duration::from_secs(10))
                // A public page must not be able to send the Agent to an address
                // on the Owner's own network behind their back.
                .redirect(reqwest::redirect::Policy::custom(|attempt| {
                    let came_from_outside =
                        attempt.previous().first().is_some_and(|first| !is_local(first));
                    if attempt.previous().len() >= 10 {
                        attempt.error("too many redirects")
                    } else if came_from_outside && is_local(attempt.url()) {
                        attempt.error("the page redirects to an address on the local network")
                    } else {
                        attempt.follow()
                    }
                }));
            let building = if the_older_way { building.http1_only() } else { building };
            building.build().expect("the HTTP client can be built")
        };
        let (http, older) = (client(false), client(true));
        Self { http, older, search_at: "https://lite.duckduckgo.com/lite/".into() }
    }

    /// The readable content of the page at `url`.
    pub(crate) async fn read(&self, url: &Url) -> String {
        let (kind, body) = match self.download(url).await {
            Ok(downloaded) => downloaded,
            Err(problem) => return format!("Could not read {url}: {problem}."),
        };
        if kind.contains("html") {
            let page = String::from_utf8_lossy(&body);
            // A page of YouTube is a program, not a text: what it is about
            // is taken from what it carries, and a channel's latest videos
            // from the feed YouTube publishes of them.
            if is_youtube(url)
                && let Some(told) = self.youtube(url, &page).await
            {
                return told;
            }
            return readable(url, &page);
        }
        let textual = kind.starts_with("text/")
            || ["json", "xml", "javascript", "yaml", "csv"].iter().any(|mark| kind.contains(mark));
        match String::from_utf8(body) {
            Ok(text) if textual || kind.is_empty() => kept(&text),
            Ok(_) | Err(_) => format!("{url} is not text but {kind}; it was not read."),
        }
    }

    /// What a page of YouTube is about: a video, with when it came out and
    /// how often it was watched; or a channel, with its latest videos.
    async fn youtube(&self, url: &Url, page: &str) -> Option<String> {
        let between = |text: &str, before: &str, after: char| -> Option<String> {
            let from = text.find(before)? + before.len();
            let rest = &text[from..];
            Some(rest[..rest.find(after)?].to_owned())
        };
        // A video.
        if url.path().starts_with("/watch") || url.path().starts_with("/shorts/") {
            let details = &page[page.find("\"videoDetails\":")?..];
            let title = between(details, "\"title\":\"", '"')?;
            let views = between(details, "\"viewCount\":\"", '"').unwrap_or_default();
            let channel = between(details, "\"author\":\"", '"').unwrap_or_default();
            let seconds = between(details, "\"lengthSeconds\":\"", '"').unwrap_or_default();
            let published = between(page, "\"publishDate\":\"", '"').unwrap_or_default();
            return Some(format!(
                "# {title}\n{url}\n\nA video of YouTube, by {channel}. Published: {published}. \
                 Views: {views}. Length: {seconds} seconds."
            ));
        }
        // A channel: its number, then the feed of its latest videos.
        let id = between(page, "channel_id=UC", '"')
            .map(|rest| format!("UC{}", rest.chars().take(22).collect::<String>()))
            .filter(|id| {
                id.len() == 24 && id.chars().all(|c| c.is_ascii_alphanumeric() || "-_".contains(c))
            })?;
        let name = between(page, "<meta property=\"og:title\" content=\"", '"').unwrap_or_default();
        let followed = between(page, "\"subscriberCountText\"", '}')
            .and_then(|text| between(&text, "\"simpleText\":\"", '"'))
            .or_else(|| {
                let at = page.find(" subscribers")?;
                Some(page[..at].rsplit(['"', '>', ' ']).next()?.to_owned() + " subscribers")
            })
            .unwrap_or_default();
        let feed = Url::parse(&format!("https://www.youtube.com/feeds/videos.xml?channel_id={id}"))
            .ok()?;
        let (_, body) = self.download(&feed).await.ok()?;
        let feed = String::from_utf8_lossy(&body);
        let mut told = format!(
            "# {name}\nhttps://www.youtube.com/channel/{id}\n\nA channel of YouTube. {followed}\n\n\
             Its latest videos, the newest first (date published, views, title, address). Shorts \
             are among them: a video's own page says how long it is. Older videos are only on \
             the channel's page, which the Computer can open.\n"
        );
        for entry in feed.split("<entry>").skip(1).take(15) {
            let title = between(entry, "<title>", '<').unwrap_or_default();
            let published = between(entry, "<published>", '<').unwrap_or_default();
            let views = between(entry, "views=\"", '"').unwrap_or_default();
            let video = between(entry, "<yt:videoId>", '<').unwrap_or_default();
            told.push_str(&format!(
                "- {} · {views} views · {title} · https://www.youtube.com/watch?v={video}\n",
                published.get(..10).unwrap_or(&published)
            ));
        }
        Some(told)
    }

    /// What the search engine finds for `query`.
    pub(crate) async fn search(&self, query: &str) -> String {
        let found = self.search_once(query).await;
        // Asked for words exactly as written, a search often finds nothing
        // that asked plainly it finds: asked again that way before giving up.
        if found.starts_with("No result") && query.contains('"') {
            let plainly = query.replace('"', " ");
            let again = self.search_once(plainly.trim()).await;
            if !again.starts_with("No result") {
                return format!("Nothing for the exact words; asked plainly:\n{again}");
            }
        }
        if found.starts_with("No result") {
            return format!(
                "{found} Ask again in other words (fewer of them, or in the language of what you \
                 look for), or read a page you already know of."
            );
        }
        found
    }

    async fn search_once(&self, query: &str) -> String {
        let Ok(mut address) = Url::parse(&self.search_at) else {
            return "The search engine's address is not valid.".into();
        };
        address.query_pairs_mut().append_pair("q", query);
        let page = match self.download(&address).await {
            Ok((_, body)) => String::from_utf8_lossy(&body).into_owned(),
            Err(problem) => return format!("The search could not be made: {problem}."),
        };
        let page = Html::parse_document(&page);
        let links = Selector::parse("a.result-link").expect("valid selector");
        let excerpts = Selector::parse("td.result-snippet").expect("valid selector");
        let mut excerpts = page.select(&excerpts).map(|excerpt| one_line(&excerpt));
        let results: Vec<String> = page
            .select(&links)
            .take(RESULTS_KEPT)
            .enumerate()
            .map(|(index, link)| {
                let address = destination(link.value().attr("href").unwrap_or_default());
                let excerpt = excerpts.next().unwrap_or_default();
                format!("{}. {}\n   {address}\n   {excerpt}", index + 1, one_line(&link))
            })
            .collect();
        if results.is_empty() {
            return format!("No result for \"{query}\".");
        }
        results.join("\n")
    }

    /// The content type and the first bytes of what is at `url`.
    async fn download(&self, url: &Url) -> Result<(String, Vec<u8>), String> {
        match self.fetch(&self.http, url).await {
            // Cut off in the newer way of speaking, at the start or in the
            // middle: asked again in the older.
            Err(problem) if problem.contains("protocol error") => {
                self.fetch(&self.older, url).await
            }
            fetched => fetched,
        }
    }

    async fn fetch(
        &self,
        through: &reqwest::Client,
        url: &Url,
    ) -> Result<(String, Vec<u8>), String> {
        let mut asking = through.get(url.clone());
        // YouTube shows nothing in Europe before it is told cookies are
        // agreed to, which nobody is there to click: it is told so.
        if is_youtube(url) {
            asking = asking
                .header(reqwest::header::COOKIE, "SOCS=CAI; CONSENT=YES+cb")
                .header(reqwest::header::ACCEPT_LANGUAGE, "en");
        }
        let mut response = asking.send().await.map_err(|error| root_cause(&error))?;
        if !response.status().is_success() {
            return Err(format!("the site answered HTTP {}", response.status().as_u16()));
        }
        let kind = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_lowercase();
        let mut body = Vec::new();
        while body.len() < DOWNLOAD_LIMIT
            && let Some(chunk) = response.chunk().await.map_err(|error| root_cause(&error))?
        {
            body.extend_from_slice(&chunk);
        }
        Ok((kind, body))
    }
}

/// True when `url` is of YouTube.
fn is_youtube(url: &Url) -> bool {
    url.host_str().is_some_and(|host| {
        let host = host.trim_start_matches("www.").trim_start_matches("m.");
        host == "youtube.com" || host == "youtu.be"
    })
}

/// True when `url` points at the Host itself or the network around it,
/// rather than at the public web.
pub(crate) fn is_local(url: &Url) -> bool {
    match url.host() {
        None => true,
        Some(Host::Ipv4(address)) => is_local_address(IpAddr::V4(address)),
        Some(Host::Ipv6(address)) => is_local_address(IpAddr::V6(address)),
        Some(Host::Domain(name)) => {
            let name = name.trim_end_matches('.').to_lowercase();
            // A name without a dot is only known on the local network.
            !name.contains('.')
                || [".localhost", ".local", ".internal", ".lan", ".home.arpa"]
                    .iter()
                    .any(|suffix| name.ends_with(suffix))
        }
    }
}

fn is_local_address(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(v4) => {
            let [first, second, ..] = v4.octets();
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                // Addresses shared by carriers and private overlay networks.
                || (first == 100 && (64..128).contains(&second))
        }
        IpAddr::V6(v6) => {
            let first = v6.segments()[0];
            v6.is_loopback()
                || v6.is_unspecified()
                || (first & 0xfe00) == 0xfc00
                || (first & 0xffc0) == 0xfe80
                || v6.to_ipv4_mapped().is_some_and(|v4| is_local_address(IpAddr::V4(v4)))
        }
    }
}

/// A page's title and content as text, without what surrounds the content.
fn readable(url: &Url, page: &str) -> String {
    let document = Html::parse_document(page);
    let title = Selector::parse("title").expect("valid selector");
    let title = document.select(&title).next().map(|title| one_line(&title)).unwrap_or_default();

    // The part the page itself marks as its content, when it marks one.
    let main = Selector::parse("main, article, [role=main]").expect("valid selector");
    let body = Selector::parse("body").expect("valid selector");
    let content = document
        .select(&main)
        .max_by_key(|part| part.text().map(str::len).sum::<usize>())
        .or_else(|| document.select(&body).next())
        .unwrap_or_else(|| document.root_element());

    let mut cleaned = String::new();
    write_content(content, &mut cleaned);
    let text = html2text::from_read(cleaned.as_bytes(), 1_000).unwrap_or_default();
    let text = text.trim();

    if text.chars().count() < 200 && page.contains("<script") && !page.contains("<main") {
        return format!(
            "{url} is built by scripts in the browser: reading it this way gives almost \
             nothing. It needs a real browser.\n\n{title}\n{text}"
        );
    }
    kept(&format!("# {title}\n{url}\n\n{text}"))
}

/// Writes `element` back as HTML, leaving out what is not content.
fn write_content(element: ElementRef, html: &mut String) {
    let name = element.value().name();
    if NOISE.contains(&name) {
        return;
    }
    html.push_str(&format!("<{name}"));
    for attribute in ["href", "alt", "colspan"] {
        if let Some(value) = element.value().attr(attribute) {
            html.push_str(&format!(" {attribute}=\"{}\"", escape(value)));
        }
    }
    html.push('>');
    for child in element.children() {
        match child.value() {
            Node::Text(text) => html.push_str(&escape(text)),
            Node::Element(_) => {
                if let Some(child) = ElementRef::wrap(child) {
                    write_content(child, html);
                }
            }
            _ => {}
        }
    }
    html.push_str(&format!("</{name}>"));
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// The text of `element` on one line.
fn one_line(element: &ElementRef) -> String {
    element.text().collect::<String>().split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Where a result really leads: the search engine wraps addresses in its own.
fn destination(link: &str) -> String {
    let absolute = if link.starts_with("//") { format!("https:{link}") } else { link.to_owned() };
    Url::parse(&absolute)
        .ok()
        .and_then(|url| {
            url.query_pairs().find(|(key, _)| key == "uddg").map(|(_, to)| to.into_owned())
        })
        .unwrap_or(absolute)
}

/// `text`, cut when it is longer than what the model should be given.
fn kept(text: &str) -> String {
    if text.len() <= PAGE_KEPT {
        return text.to_owned();
    }
    let mut end = PAGE_KEPT;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n[page cut: {} more characters]", &text[..end], text.len() - end)
}

/// The innermost cause, which is the part a person can act on.
fn root_cause(error: &dyn std::error::Error) -> String {
    let mut cause = error;
    while let Some(source) = cause.source() {
        cause = source;
    }
    cause.to_string()
}
