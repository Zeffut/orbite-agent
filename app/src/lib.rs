//! Orbite's app: a window on the Conversation the Owner's agent serves. The
//! app holds nothing of the Conversation itself. It remembers where the
//! agent is reached, checks that it answers, and gives way to its page.
//! The few pages of its own are written here, in Rust, and hold no script.
//! Around that page it does what a browser tab cannot: it tells the Owner
//! of something new when they are elsewhere, keeps other sites out of the
//! window, and stays at hand beside the clock.

use std::net::{TcpStream, ToSocketAddrs};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use orbite_app_core::{
    Asked, Language, Unseen, away_page, leaving_page, link_page, reached, reaching_page,
    something_new, stays_in_the_app,
};
use tauri::http::{Request, Response};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_opener::OpenerExt;

/// Where the app's own pages are, as each system has a window ask for them.
#[cfg(any(windows, target_os = "android"))]
const OWN_PAGES: &str = "http://orbite.localhost/";
#[cfg(not(any(windows, target_os = "android")))]
const OWN_PAGES: &str = "orbite://localhost/";

/// How many things the Owner had not seen when the app last told them.
struct Told(AtomicU32);

fn language() -> Language {
    Language::of(sys_locale::get_locale().as_deref())
}

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
    let language = language();
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

/// Opens `to` in the Owner's browser, when it is a page of the web; what
/// could not be opened is said where the app's own words go.
fn open_outside(app: &AppHandle, to: &tauri::Url) {
    if !matches!(to.scheme(), "https" | "http") {
        return;
    }
    if let Err(why) = app.opener().open_url(to.as_str(), None::<&str>) {
        eprintln!("orbite: {to} could not be opened in a browser: {why}");
    }
}

/// Shows the window again, in front.
fn come_forward(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        #[cfg(desktop)]
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

/// Tells the Owner of what the agent's page says they have not seen, when
/// they are not looking at the window, and once for each new thing.
fn tell(app: &AppHandle, title: &str) {
    let unseen = Unseen::of(title);
    let told = app.state::<Told>();
    let before = told.0.swap(unseen.count, Ordering::SeqCst);
    let Some(window) = app.get_webview_window("main") else { return };
    // The window is named after the agent, without the count the page adds.
    if !unseen.agent.is_empty() {
        let _ = window.set_title(&unseen.agent);
    }
    // How many, on the app's icon, where the system shows such a thing.
    #[cfg(desktop)]
    let _ = window.set_badge_count((unseen.count > 0).then_some(i64::from(unseen.count)));
    let looking = window.is_focused().unwrap_or(false) && window.is_visible().unwrap_or(false);
    if unseen.count > before && !looking {
        let body = match unseen.latest.is_empty() {
            true => something_new(language()).to_owned(),
            false => unseen.latest.clone(),
        };
        let _ = app.notification().builder().title(&unseen.agent).body(body).show();
    }
}

/// Beside the clock: the app stays at hand once its window is closed.
#[cfg(desktop)]
fn stay_at_hand(app: &AppHandle) -> tauri::Result<()> {
    use tauri::menu::{CheckMenuItem, Menu, MenuItem};
    use tauri::tray::TrayIconBuilder;
    use tauri_plugin_autostart::ManagerExt;
    let [open, start, other, quit] = orbite_app_core::menu(language());
    // Ticked when Orbite starts with the session, which the Owner decides here.
    let starts = app.autolaunch().is_enabled().unwrap_or(false);
    let starting = CheckMenuItem::with_id(app, "start", start, true, starts, None::<&str>)?;
    let offered = Menu::with_items(
        app,
        &[
            &MenuItem::with_id(app, "open", open, true, None::<&str>)?,
            &starting,
            &MenuItem::with_id(app, "other", other, true, None::<&str>)?,
            &MenuItem::with_id(app, "quit", quit, true, None::<&str>)?,
        ],
    )?;
    let mut tray = TrayIconBuilder::with_id("orbite").tooltip("Orbite").menu(&offered);
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.on_menu_event(move |app, chosen| match chosen.id().as_ref() {
        "open" => come_forward(app),
        "start" => {
            let wanted = !app.autolaunch().is_enabled().unwrap_or(false);
            let done = match wanted {
                true => app.autolaunch().enable(),
                false => app.autolaunch().disable(),
            };
            // The tick says what is, not what was asked.
            let _ = starting.set_checked(if done.is_ok() { wanted } else { !wanted });
        }
        "other" => {
            if let (Some(window), Ok(link)) =
                (app.get_webview_window("main"), format!("{OWN_PAGES}link").parse())
            {
                let _ = window.navigate(link);
            }
            come_forward(app);
        }
        "quit" => app.exit(0),
        _ => {}
    })
    .build(app)?;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init());
    // One Orbite at a time: opened again, the one that runs comes forward.
    #[cfg(desktop)]
    let app = app
        .plugin(tauri_plugin_single_instance::init(|app, _, _| come_forward(app)))
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ));
    app.manage(Told(AtomicU32::new(0)))
        .register_asynchronous_uri_scheme_protocol("orbite", |context, request, responder| {
            let app = context.app_handle().clone();
            tauri::async_runtime::spawn(async move {
                responder.respond(answer(app, request).await);
            });
        })
        .setup(|app| {
            let pages = WebviewUrl::CustomProtocol(OWN_PAGES.parse()?);
            let (leaving, telling) = (app.handle().clone(), app.handle().clone());
            let window = WebviewWindowBuilder::new(app, "main", pages)
                // The window shows the app's pages and the agent's: any
                // other site opens in the Owner's browser.
                .on_navigation(move |to| {
                    if stays_in_the_app(to.as_str(), kept(&leaving).as_deref()) {
                        return true;
                    }
                    open_outside(&leaving, to);
                    false
                })
                .on_document_title_changed(move |_, title| tell(&telling, &title));
            #[cfg(desktop)]
            let window = {
                let opening = app.handle().clone();
                window
                    .title("Orbite")
                    .inner_size(460.0, 780.0)
                    .min_inner_size(340.0, 480.0)
                    // A link the agent's page opens apart is another site's.
                    .on_new_window(move |to, _| {
                        open_outside(&opening, &to);
                        tauri::webview::NewWindowResponse::Deny
                    })
            };
            let window = window.build()?;
            #[cfg(desktop)]
            {
                // Where the system keeps apps beside the clock, closing the
                // window puts Orbite there, where it still tells of what is new.
                let at_hand = stay_at_hand(app.handle()).is_ok();
                if at_hand && cfg!(any(windows, target_os = "macos")) {
                    let closing = window.clone();
                    window.on_window_event(move |happened| {
                        if let tauri::WindowEvent::CloseRequested { api, .. } = happened {
                            api.prevent_close();
                            let _ = closing.hide();
                        }
                    });
                }
            }
            let _ = window;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("the app could not start");
}
