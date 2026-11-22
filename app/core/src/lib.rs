//! What Orbite's app decides and shows, apart from the window that shows
//! it: reading the link an agent gives, and the few pages the app writes
//! itself before the agent's own page takes their place. The pages hold no
//! script: a form to send, a line that leads on.

use std::fmt::Write;

/// Where an agent is reached, as its Owner was given it: the address alone,
/// and the code that lets this device in when the link carried one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reached {
    pub address: String,
    pub code: Option<String>,
}

/// Why what the Owner pasted leads to no agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wrong {
    NotAnAddress,
    NotEncrypted,
}

/// The language the app's own pages are written in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    English,
    French,
}

impl Language {
    /// The language of a system set to `locale`, such as `fr-FR`.
    pub fn of(locale: Option<&str>) -> Self {
        match locale.map(str::to_lowercase) {
            Some(locale) if locale.starts_with("fr") => Self::French,
            _ => Self::English,
        }
    }

    fn say<'a>(self, english: &'a str, french: &'a str) -> &'a str {
        match self {
            Self::English => english,
            Self::French => french,
        }
    }
}

/// True for what a code that lets a device in looks like: `ABCD-EFGH`.
pub fn is_a_code(code: &str) -> bool {
    code.len() == 9 && code.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// Reads what the Owner pasted: the link their agent gave them, with or
/// without the code after `#`, or the address typed by hand.
pub fn reached(pasted: &str) -> Result<Reached, Wrong> {
    let pasted = pasted.trim();
    let written = match pasted.contains("://") {
        true => pasted.to_owned(),
        false => format!("https://{pasted}"),
    };
    let link = url::Url::parse(&written).map_err(|_| Wrong::NotAnAddress)?;
    if !matches!(link.scheme(), "https" | "http") || link.host_str().is_none_or(str::is_empty) {
        return Err(Wrong::NotAnAddress);
    }
    // In the open only on the machine itself: anywhere else, what travels
    // is the Conversation and what lets a device in.
    let here = matches!(link.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    if link.scheme() == "http" && !here {
        return Err(Wrong::NotEncrypted);
    }
    let code = link.fragment().filter(|code| is_a_code(code)).map(str::to_uppercase);
    let mut address = link.clone();
    address.set_fragment(None);
    address.set_path("");
    address.set_query(None);
    Ok(Reached { address: address.as_str().trim_end_matches('/').to_owned(), code })
}

/// The host and port of `address`, as shown to the Owner.
pub fn host_of(address: &str) -> String {
    url::Url::parse(address)
        .ok()
        .and_then(|link| {
            let host = link.host_str()?.to_owned();
            Some(match link.port() {
                Some(port) => format!("{host}:{port}"),
                None => host,
            })
        })
        .unwrap_or_default()
}

/// What the agent's page says, through its title, of what the Owner has not
/// seen yet: how many things, the agent's name, and the latest of them in a
/// few words. The page writes `(2) Atlas — Done, the file is ready`; with
/// nothing unseen its title is the agent's name alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unseen {
    pub count: u32,
    pub agent: String,
    pub latest: String,
}

impl Unseen {
    pub fn of(title: &str) -> Self {
        let counted = title.strip_prefix('(').and_then(|rest| {
            let (count, rest) = rest.split_once(") ")?;
            Some((count.parse::<u32>().ok()?, rest))
        });
        let (count, rest) = counted.unwrap_or((0, title));
        let (agent, latest) = rest.split_once(" — ").unwrap_or((rest, ""));
        Self { count, agent: agent.trim().to_owned(), latest: latest.trim().to_owned() }
    }
}

/// True when `to` is a place the app's window may show: a page of the
/// app's own, or one of the agent at `agent`. Anything else is another
/// site, which opens in the Owner's browser and never in the app.
pub fn stays_in_the_app(to: &str, agent: Option<&str>) -> bool {
    let Ok(to) = url::Url::parse(to) else { return false };
    if to.scheme() == "orbite"
        || to.host_str() == Some("orbite.localhost")
        || to.scheme() == "about"
    {
        return true;
    }
    let Some(agent) = agent.and_then(|agent| url::Url::parse(agent).ok()) else { return false };
    to.scheme() == agent.scheme()
        && to.host_str() == agent.host_str()
        && to.port_or_known_default() == agent.port_or_known_default()
}

/// What the app's menu beside the clock offers, in order: open the window,
/// point the app at another agent, quit.
pub fn menu(language: Language) -> [&'static str; 3] {
    [
        language.say("Open Orbite", "Ouvrir Orbite"),
        language.say("Use another agent…", "Utiliser un autre agent…"),
        language.say("Quit", "Quitter"),
    ]
}

/// What a notification says when the page gave no words for it.
pub fn something_new(language: Language) -> &'static str {
    language.say("Something new for you", "Du nouveau pour toi")
}

/// What a page of the app's asked for, by its path and what follows `?`.
#[derive(Debug, PartialEq, Eq)]
pub enum Asked {
    /// The app opening.
    Start,
    /// The form to paste a link in.
    Link,
    /// The link pasted in that form.
    Open(String),
    /// On to the agent, with the code of a first visit when there is one.
    Go(Option<String>),
}

impl Asked {
    pub fn of(path: &str, query: &str) -> Self {
        let given = |name: &str| {
            url::form_urlencoded::parse(query.as_bytes())
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.into_owned())
        };
        match path.trim_matches('/') {
            "link" => Self::Link,
            "open" => Self::Open(given("link").unwrap_or_default()),
            "go" => Self::Go(given("code").filter(|code| is_a_code(code))),
            _ => Self::Start,
        }
    }
}

/// `text` as it may stand in a page, whatever it holds.
fn plain(text: &str) -> String {
    let mut written = String::with_capacity(text.len());
    for letter in text.chars() {
        match letter {
            '&' => written.push_str("&amp;"),
            '<' => written.push_str("&lt;"),
            '>' => written.push_str("&gt;"),
            '"' => written.push_str("&quot;"),
            '\'' => written.push_str("&#39;"),
            letter => written.push(letter),
        }
    }
    written
}

const STYLE: &str = "\
:root{--ground:#f6f4ef;--paper:#fff;--ink:#1a1c21;--quiet:#6b7180;--line:#e2ded4;--accent:#2f6fed;--warn:#b3261e;color-scheme:light dark}\
@media (prefers-color-scheme:dark){:root{--ground:#0f1115;--paper:#181b22;--ink:#ecebe6;--quiet:#9097a6;--line:#272b35;--accent:#7aa7ff;--warn:#ff8a80}}\
*{box-sizing:border-box}html,body{height:100%;margin:0}\
body{background:var(--ground);color:var(--ink);font:16px/1.5 system-ui,-apple-system,'Segoe UI',Roboto,sans-serif;-webkit-text-size-adjust:100%}\
main{min-height:100%;display:grid;place-content:center;justify-items:start;gap:14px;max-width:440px;margin:0 auto;\
padding:max(24px,env(safe-area-inset-top)) 24px max(24px,env(safe-area-inset-bottom))}\
form{display:grid;gap:12px;width:100%}h1{font-size:24px;line-height:1.25;margin:0}p{margin:0;color:var(--quiet)}\
label{display:grid;gap:6px;font-size:14px;color:var(--quiet)}\
input{font:inherit;color:var(--ink);padding:13px 14px;border-radius:12px;border:1px solid var(--line);background:var(--paper);width:100%}\
input:focus-visible,button:focus-visible,a:focus-visible{outline:2px solid var(--accent);outline-offset:2px}\
button,a.go{font:inherit;font-weight:600;padding:13px 16px;border-radius:12px;cursor:pointer;border:1px solid var(--accent);\
background:var(--accent);color:#fff;text-decoration:none;text-align:center;display:block;width:100%}\
a.other{color:var(--accent);padding:10px 0;display:block}.wrong{color:var(--warn)}\
.where{font-family:ui-monospace,Menlo,monospace;font-size:14px;overflow-wrap:anywhere}";

const ICON: &str = "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 64 64' width='64' height='64' aria-hidden='true'>\
<rect width='64' height='64' rx='14' fill='#0d0d0d'/><g transform='translate(6 6) scale(0.52)'>\
<g transform='rotate(-24 50 50)'><ellipse cx='50' cy='50' rx='44' ry='13' fill='none' stroke='#f2f2f0' stroke-width='5'/></g>\
<circle cx='50' cy='50' r='27' fill='#f2f2f0'/><rect x='40' y='38' width='6' height='13' rx='3' fill='#0d0d0d'/>\
<rect x='54' y='38' width='6' height='13' rx='3' fill='#0d0d0d'/></g></svg>";

/// A page of the app's, around `inside`; `then` is where it leads on to by
/// itself, at once, when it does.
fn page(language: Language, then: Option<&str>, inside: &str) -> String {
    let mut written = String::new();
    let _ = write!(
        written,
        "<!doctype html><html lang=\"{}\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1, viewport-fit=cover\">",
        language.say("en", "fr")
    );
    if let Some(then) = then {
        let _ = write!(written, "<meta http-equiv=\"refresh\" content=\"0;url={}\">", plain(then));
    }
    let _ = write!(
        written,
        "<title>Orbite</title><style>{STYLE}</style></head><body><main>{ICON}{inside}</main></body></html>"
    );
    written
}

/// The form the Owner pastes their agent's link in; `wrong` says why what
/// they pasted before, shown again, led nowhere.
pub fn link_page(language: Language, wrong: Option<(Wrong, &str)>) -> String {
    let said = wrong.map(|(why, _)| match why {
        Wrong::NotAnAddress => language
            .say("That is not a link from your agent.", "Ce n'est pas un lien de ton agent."),
        Wrong::NotEncrypted => language.say(
            "That address is not encrypted: ask your agent for its link.",
            "Cette adresse n'est pas chiffrée : demande son lien à ton agent.",
        ),
    });
    let mut inside = format!(
        "<form action=\"open\" method=\"get\"><h1>{}</h1><p>{}</p><label>{}\
         <input name=\"link\" value=\"{}\" inputmode=\"url\" autocapitalize=\"none\" autocorrect=\"off\" \
         spellcheck=\"false\" placeholder=\"https://…\" required autofocus></label><button>{}</button>",
        language.say("Open your agent", "Ouvre ton agent"),
        language.say(
            "Ask your agent to add this device, and paste the link it gives you. You do this once.",
            "Demande à ton agent d'ajouter cet appareil, et colle le lien qu'il te donne. C'est à faire une seule fois.",
        ),
        language.say("Link from your agent", "Lien donné par ton agent"),
        plain(wrong.map_or("", |(_, pasted)| pasted)),
        language.say("Open", "Ouvrir"),
    );
    if let Some(said) = said {
        let _ = write!(inside, "<p class=\"wrong\" role=\"alert\">{said}</p>");
    }
    inside.push_str("</form>");
    page(language, None, &inside)
}

/// Shown while the agent at `address` is being reached, and leading on to
/// `then`, where the app finds out whether it answers.
pub fn reaching_page(language: Language, address: &str, then: &str) -> String {
    let inside = format!(
        "<h1>{}</h1><p class=\"where\">{}</p>",
        language.say("Reaching your agent…", "Connexion à ton agent…"),
        plain(&host_of(address)),
    );
    page(language, Some(then), &inside)
}

/// Shown when the agent at `address` does not answer.
pub fn away_page(language: Language, address: &str) -> String {
    let inside = format!(
        "<h1>{}</h1><p class=\"where\">{}</p><p>{}</p><a class=\"go\" href=\"go\">{}</a>\
         <a class=\"other\" href=\"link\">{}</a>",
        language.say("Your agent does not answer", "Ton agent ne répond pas"),
        plain(&host_of(address)),
        language.say(
            "Check that this device is on the same private network as your agent's machine (Tailscale, \
             for instance), and that the machine is on.",
            "Vérifie que cet appareil est sur le même réseau privé que la machine de ton agent (Tailscale, \
             par exemple), et que la machine est allumée.",
        ),
        language.say("Try again", "Réessayer"),
        language.say("Use another link", "Utiliser un autre lien"),
    );
    page(language, None, &inside)
}

/// The last page of the app's: it gives way to the agent's own, at
/// `address`, with the code of a first visit when there is one.
pub fn leaving_page(language: Language, address: &str, code: Option<&str>) -> String {
    let to = match code.filter(|code| is_a_code(code)) {
        Some(code) => format!("{address}/#{code}"),
        None => address.to_owned(),
    };
    let inside = format!(
        "<h1>{}</h1><p class=\"where\">{}</p>",
        language.say("Reaching your agent…", "Connexion à ton agent…"),
        plain(&host_of(address)),
    );
    page(language, Some(&to), &inside)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_link_an_agent_gives_is_read_as_its_address_and_the_code_that_lets_in() {
        assert_eq!(
            reached(" https://host.tail1234.ts.net:8377/#abcd-efgh "),
            Ok(Reached {
                address: "https://host.tail1234.ts.net:8377".into(),
                code: Some("ABCD-EFGH".into()),
            })
        );
    }

    #[test]
    fn an_address_typed_by_hand_is_taken_as_encrypted() {
        assert_eq!(
            reached("agent.example.com"),
            Ok(Reached { address: "https://agent.example.com".into(), code: None })
        );
    }

    #[test]
    fn an_address_in_the_open_is_refused_unless_it_is_this_machine() {
        assert_eq!(reached("http://192.168.1.20:8377"), Err(Wrong::NotEncrypted));
        assert_eq!(
            reached("http://127.0.0.1:8377/#ABCD-EFGH").map(|read| read.address),
            Ok("http://127.0.0.1:8377".into())
        );
    }

    #[test]
    fn what_is_no_address_is_said_so() {
        for pasted in ["", "ftp://host", "javascript:alert(1)", "https://", "orbite://app/go"] {
            assert_eq!(reached(pasted), Err(Wrong::NotAnAddress), "{pasted}");
        }
    }

    #[test]
    fn what_follows_the_address_is_a_code_only_when_it_looks_like_one() {
        let read = reached("https://host.example/#<script>").unwrap();
        assert_eq!(read, Reached { address: "https://host.example".into(), code: None });
    }

    #[test]
    fn what_is_unseen_is_read_from_the_title_of_the_agents_page() {
        assert_eq!(
            Unseen::of("(2) Atlas — Done, the file is ready"),
            Unseen { count: 2, agent: "Atlas".into(), latest: "Done, the file is ready".into() }
        );
        assert_eq!(
            Unseen::of("Atlas"),
            Unseen { count: 0, agent: "Atlas".into(), latest: String::new() }
        );
        // A name that happens to hold brackets or a dash is still a name.
        assert_eq!(Unseen::of("(x) Bot").count, 0);
        assert_eq!(Unseen::of("(3) My agent").agent, "My agent");
    }

    #[test]
    fn the_window_shows_the_apps_pages_and_the_agents_and_nothing_else() {
        let agent = Some("https://host.tail1234.ts.net:8377");
        assert!(stays_in_the_app("orbite://localhost/go", agent));
        assert!(stays_in_the_app("http://orbite.localhost/link", None));
        assert!(stays_in_the_app("https://host.tail1234.ts.net:8377/api/file?id=x", agent));
        // Another site, another port of the same machine, or the same name in the open.
        assert!(!stays_in_the_app("https://example.com/", agent));
        assert!(!stays_in_the_app("https://host.tail1234.ts.net/", agent));
        assert!(!stays_in_the_app("http://host.tail1234.ts.net:8377/", agent));
        assert!(!stays_in_the_app("https://host.tail1234.ts.net.evil.example:8377/", agent));
        assert!(!stays_in_the_app("https://example.com/", None));
    }

    #[test]
    fn what_a_page_asks_for_is_read_from_its_path() {
        assert_eq!(Asked::of("/", ""), Asked::Start);
        assert_eq!(Asked::of("/link", ""), Asked::Link);
        assert_eq!(
            Asked::of("/open", "link=https%3A%2F%2Fhost.example%3A8377%2F%23ABCD-EFGH"),
            Asked::Open("https://host.example:8377/#ABCD-EFGH".into())
        );
        assert_eq!(Asked::of("/go", "code=ABCD-EFGH"), Asked::Go(Some("ABCD-EFGH".into())));
        // What is no code is not carried on to the agent's page.
        assert_eq!(Asked::of("/go", "code=%22%3E%3Cscript%3E"), Asked::Go(None));
        assert_eq!(Asked::of("/anything-else", ""), Asked::Start);
    }

    #[test]
    fn the_pages_are_written_in_the_systems_language_and_hold_no_script() {
        assert_eq!(Language::of(Some("fr-FR")), Language::French);
        assert_eq!(Language::of(Some("de-DE")), Language::English);
        assert_eq!(Language::of(None), Language::English);
        let pages = [
            link_page(Language::French, None),
            reaching_page(Language::French, "https://host.example:8377", "go"),
            away_page(Language::French, "https://host.example:8377"),
            leaving_page(Language::French, "https://host.example:8377", Some("ABCD-EFGH")),
        ];
        assert!(pages[0].contains("Ouvre ton agent"));
        assert!(
            pages[2].contains("Ton agent ne répond pas") && pages[2].contains("host.example:8377")
        );
        for written in &pages {
            assert!(!written.contains("<script"), "{written}");
        }
    }

    #[test]
    fn a_page_leads_on_only_where_the_app_means_it_to() {
        let leaving =
            leaving_page(Language::English, "https://host.example:8377", Some("ABCD-EFGH"));
        assert!(
            leaving.contains("content=\"0;url=https://host.example:8377/#ABCD-EFGH\""),
            "{leaving}"
        );
        // A code that is none is left out rather than written into the page.
        let odd = leaving_page(Language::English, "https://host.example:8377", Some("\"><script>"));
        assert!(odd.contains("content=\"0;url=https://host.example:8377\""), "{odd}");
    }

    #[test]
    fn what_the_owner_pasted_is_shown_again_as_text_whatever_it_holds() {
        let pasted = "\"><script>alert(1)</script>";
        let shown = link_page(Language::English, Some((Wrong::NotAnAddress, pasted)));
        assert!(!shown.contains("<script>alert"), "{shown}");
        assert!(shown.contains("&lt;script&gt;"), "{shown}");
        assert!(shown.contains("That is not a link from your agent."));
    }
}
