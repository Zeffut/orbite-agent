//! Orbite's app: a window on the Conversation the Owner's agent serves. The
//! app holds nothing of the Conversation itself. It remembers where the
//! agent is reached, checks that it answers, and gives way to its page.
//! The few pages of its own are written here, in Rust, and hold no script.

use std::net::{TcpStream, ToSocketAddrs};
use std::path::PathBuf;
use std::time::Duration;

use orbite_app_core::{
    Asked, Language, away_page, leaving_page, link_page, reached, reaching_page,
};
use tauri::http::{Request, Response};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

/// Where the app's own pages are, as each system has a window ask for them.
#[cfg(any(windows, target_os = "android"))]
const OWN_PAGES: &str = "http://orbite.localhost/";
#[cfg(not(any(windows, target_os = "android")))]
const OWN_PAGES: &str = "orbite://localhost/";

/// The file the app keeps the agent's address in, and nothing else.
fn kept_in(app: &AppHandle) -> Option<PathBuf> {
    Some(app.path().app_data_dir().ok()?.join("agent"))
}

/// The address of the agent this app was pointed at, when it was.
fn kept(app: &AppHandle) -> Option<String> {
    let address = std::fs::read_to_string(kept_in(app)?).ok()?;
    // Read again as if just pasted: nothing else is ever led on to.
    reached(address.trim()).ok().map(|read| read.address)
}

fn keep(app: &AppHandle, address: &str) -> std::io::Result<()> {
    let file = kept_in(app).ok_or_else(|| std::io::Error::other("no folder for the app"))?;
    if let Some(folder) = file.parent() {
        std::fs::create_dir_all(folder)?;
    }
    std::fs::write(file, address)
}

/// True when something answers where `address` says the agent is: found out
/// before its page is shown, so that the Owner is never left on a blank one.
async fn answers(address: String) -> bool {
    tauri::async_runtime::spawn_blocking(move || {
        let Ok(link) = address.parse::<tauri::Url>() else { return false };
        let (Some(host), Some(port)) = (link.host_str(), link.port_or_known_default()) else {
            return false;
        };
        (host.trim_matches(['[', ']']), port).to_socket_addrs().is_ok_and(|found| {
            found
                .into_iter()
                .any(|one| TcpStream::connect_timeout(&one, Duration::from_secs(4)).is_ok())
        })
    })
    .await
    .unwrap_or(false)
}

/// The page of the app's that answers what one of its pages asked for.
async fn shown(app: &AppHandle, path: &str, query: &str) -> String {
    let language = Language::of(sys_locale::get_locale().as_deref());
    match Asked::of(path, query) {
        Asked::Link => link_page(language, None),
        Asked::Open(pasted) => match reached(&pasted) {
            Err(wrong) => link_page(language, Some((wrong, &pasted))),
            Ok(read) => {
                let _ = keep(app, &read.address);
                let then = match &read.code {
                    Some(code) => format!("go?code={code}"),
                    None => "go".to_owned(),
                };
                reaching_page(language, &read.address, &then)
            }
        },
        Asked::Start => match kept(app) {
            Some(address) => reaching_page(language, &address, "go"),
            None => link_page(language, None),
        },
        Asked::Go(code) => match kept(app) {
            None => link_page(language, None),
            Some(address) if answers(address.clone()).await => {
                leaving_page(language, &address, code.as_deref())
            }
            Some(address) => away_page(language, &address),
        },
    }
}

async fn answer(app: AppHandle, request: Request<Vec<u8>>) -> Response<Vec<u8>> {
    let page = shown(&app, request.uri().path(), request.uri().query().unwrap_or_default()).await;
    Response::builder()
        .header("content-type", "text/html; charset=utf-8")
        .header("cache-control", "no-store")
        // Written by the app, and still given nothing: no script, no frame,
        // nothing fetched from anywhere.
        .header(
            "content-security-policy",
            "default-src 'none'; style-src 'unsafe-inline'; form-action 'self'; frame-ancestors 'none'",
        )
        .body(page.into_bytes())
        .expect("a page of the app's own")
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .register_asynchronous_uri_scheme_protocol("orbite", |context, request, responder| {
            let app = context.app_handle().clone();
            tauri::async_runtime::spawn(async move {
                responder.respond(answer(app, request).await);
            });
        })
        .setup(|app| {
            let pages = WebviewUrl::CustomProtocol(OWN_PAGES.parse()?);
            let window = WebviewWindowBuilder::new(app, "main", pages);
            #[cfg(desktop)]
            let window =
                window.title("Orbite").inner_size(460.0, 780.0).min_inner_size(340.0, 480.0);
            window.build()?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("the app could not start");
}
