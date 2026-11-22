//! Orbite's app: a window on the Conversation the Owner's agent serves. The
//! app holds nothing of the Conversation itself. It remembers where the
//! agent is reached, checks that it answers, and shows its page.

use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

/// Where an agent is reached, as its Owner was given it: the address alone,
/// and the code that lets this device in when the link carried one.
#[derive(Debug, PartialEq, Eq, serde::Serialize)]
pub struct Reached {
    pub address: String,
    pub code: Option<String>,
}

/// Reads what the Owner pasted: the link their agent gave them, with or
/// without the code after `#`, or the address typed by hand. The text says
/// what is wrong with it.
pub fn reached(pasted: &str) -> Result<Reached, &'static str> {
    let pasted = pasted.trim();
    let written = match pasted.contains("://") {
        true => pasted.to_owned(),
        false => format!("https://{pasted}"),
    };
    let link = url::Url::parse(&written).map_err(|_| "not an address")?;
    if !matches!(link.scheme(), "https" | "http") || link.host_str().is_none() {
        return Err("not an address");
    }
    // In the open only on the machine itself: anywhere else, what travels
    // is the Conversation and what lets a device in.
    let here = matches!(link.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    if link.scheme() == "http" && !here {
        return Err("not encrypted");
    }
    let code = link
        .fragment()
        .filter(|code| {
            code.len() == 9 && code.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        })
        .map(str::to_uppercase);
    let mut address = link.clone();
    address.set_fragment(None);
    address.set_path("");
    address.set_query(None);
    Ok(Reached { address: address.as_str().trim_end_matches('/').to_owned(), code })
}

/// True when something answers where `address` says the agent is: asked
/// before its page is shown, so that the Owner is never left on a blank one.
#[tauri::command]
async fn answers(address: String) -> bool {
    let Ok(link) = url::Url::parse(&address) else { return false };
    let (Some(host), Some(port)) = (link.host_str(), link.port_or_known_default()) else {
        return false;
    };
    let host = host.trim_matches(['[', ']']).to_owned();
    tauri::async_runtime::spawn_blocking(move || {
        (host.as_str(), port).to_socket_addrs().is_ok_and(|found| {
            found
                .into_iter()
                .any(|one| TcpStream::connect_timeout(&one, Duration::from_secs(4)).is_ok())
        })
    })
    .await
    .unwrap_or(false)
}

/// What the Owner pasted, read as where their agent is reached.
#[tauri::command]
fn read(pasted: String) -> Result<Reached, &'static str> {
    reached(&pasted)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![answers, read])
        .run(tauri::generate_context!())
        .expect("the app could not start");
}

#[cfg(test)]
mod tests {
    use super::{Reached, reached};

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
        assert_eq!(reached("http://192.168.1.20:8377"), Err("not encrypted"));
        assert_eq!(
            reached("http://127.0.0.1:8377/#ABCD-EFGH").map(|read| read.address),
            Ok("http://127.0.0.1:8377".into())
        );
    }

    #[test]
    fn what_is_no_address_is_said_so() {
        for pasted in ["", "ftp://host", "javascript:alert(1)", "https://"] {
            assert!(reached(pasted).is_err(), "{pasted}");
        }
    }

    #[test]
    fn what_follows_the_address_is_a_code_only_when_it_looks_like_one() {
        let read = reached("https://host.example/#<script>").unwrap();
        assert_eq!(read, Reached { address: "https://host.example".into(), code: None });
    }
}
