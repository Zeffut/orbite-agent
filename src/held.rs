//! What an Agent held by a server (ADR 0022) asks of it beside thinking:
//! who else is there, to pass a word to another Agent, to have a recording
//! written down. Never to let a device in: that is not an Agent's to ask.
//! The server is the Owner's; an Agent shows what it was given when it was
//! made, and is answered as that Agent and no other.

use std::time::{Duration, Instant};

use serde_json::Value;

/// The server that holds this Agent.
pub(crate) struct Holder {
    at: String,
    token: String,
}

/// The other Agents of the server, as last told, and when: asked for at
/// every turn, they are not asked of the server each time.
static OTHERS: std::sync::Mutex<Option<(Instant, Vec<String>)>> = std::sync::Mutex::new(None);

/// What this Agent's container was given to know its server by: where it
/// is, and what to show it. Read once, from the program's surroundings.
static GIVEN: std::sync::OnceLock<(Option<String>, Option<String>)> = std::sync::OnceLock::new();

fn given() -> &'static (Option<String>, Option<String>) {
    GIVEN.get_or_init(|| {
        // Run as root by sudo for another account, what that account put
        // around the command has no say in what root then does.
        // SAFETY: a plain system call.
        let for_another = unsafe { libc::geteuid() } == 0
            && std::env::var("SUDO_UID").is_ok_and(|asked_by| asked_by.trim() != "0");
        if for_another {
            return (None, None);
        }
        (
            std::env::var("ORBITE_SERVER_AT").ok().filter(|at| !at.is_empty()),
            std::env::var("ORBITE_SERVER_TOKEN").ok().filter(|token| token.len() >= 32),
        )
    })
}

/// Takes what the server gave out of the program's surroundings and keeps
/// it to the program itself: nothing the Agent starts, a browser, a
/// command, a connector, finds it there. Called first, before any other
/// thread exists.
pub fn keep_what_the_server_gave() {
    let held = given().1.is_some();
    // SAFETY: nothing else runs yet that could be reading the surroundings;
    // what is written over is the program's own copy of them, each value
    // for as long as it is.
    unsafe {
        // Written over where it stood, too: what the system shows of a
        // program's first surroundings is read from that very place.
        let kept = libc::getenv(c"ORBITE_SERVER_TOKEN".as_ptr());
        if !kept.is_null() {
            std::ptr::write_bytes(kept, b'x', libc::strlen(kept));
        }
        std::env::remove_var("ORBITE_SERVER_AT");
        std::env::remove_var("ORBITE_SERVER_TOKEN");
        // And nothing the Agent runs reads this program's memory.
        if held {
            libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0);
        }
    }
}

/// Where the server that holds this Agent is, when one does.
pub fn server_at() -> Option<String> {
    given().0.clone()
}

/// What this Agent shows its server, and knows it by, when one holds it.
pub fn server_token() -> Option<String> {
    given().1.clone()
}

impl Holder {
    /// The server holding this Agent, when one does: said in what the
    /// Agent's container was given.
    pub(crate) fn of_this_agent() -> Option<Self> {
        Some(Self { at: server_at()?, token: server_token()? })
    }

    /// Asks the server `what`, with `given`, and waits for its answer. For
    /// the few things asked from where nothing waits on anything else.
    pub(crate) fn ask(&self, what: &str, given: Value) -> Result<Value, String> {
        let asking = self.asked(what, given);
        // On a thread of its own: whoever asks may be in the middle of
        // something that must not wait on itself.
        std::thread::scope(|scope| {
            let asked = std::thread::Builder::new().spawn_scoped(scope, move || {
                let waiting = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|error| error.to_string())?;
                // What is asked from here is answered at once, or not at all.
                waiting.block_on(async move {
                    tokio::time::timeout(Duration::from_secs(20), asking)
                        .await
                        .unwrap_or_else(|_| Err("the server did not answer".into()))
                })
            });
            match asked {
                Ok(asked) => {
                    asked.join().unwrap_or_else(|_| Err("the server could not be asked".into()))
                }
                Err(_) => Err("the server could not be asked".into()),
            }
        })
    }

    /// Asks the server `what`, with `given`: its answer, or why there is none.
    pub(crate) fn asked(
        &self,
        what: &str,
        given: Value,
    ) -> impl Future<Output = Result<Value, String>> + Send + 'static {
        let address = format!("{}/internal/ask/{what}", self.at.trim_end_matches('/'));
        let token = self.token.clone();
        async move {
            let answer = reqwest::Client::new()
                .post(address)
                .header("x-orbite-agent", token)
                .json(&given)
                .timeout(Duration::from_secs(320))
                .send()
                .await
                .map_err(|error| error.without_url().to_string())?;
            let well = answer.status().is_success();
            let told: Value = answer.json().await.unwrap_or_default();
            if well {
                Ok(told)
            } else {
                Err(told["error"].as_str().unwrap_or("the server refused").to_owned())
            }
        }
    }

    /// The names of the other Agents of the server.
    pub(crate) fn others(&self) -> Vec<String> {
        {
            let kept = OTHERS.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some((when, names)) = kept.as_ref()
                && when.elapsed() < Duration::from_secs(60)
            {
                return names.clone();
            }
        }
        let names: Vec<String> = self
            .ask("agents", Value::Null)
            .ok()
            .and_then(|told| serde_json::from_value(told["agents"].clone()).ok())
            .unwrap_or_default();
        *OTHERS.lock().unwrap_or_else(std::sync::PoisonError::into_inner) =
            Some((Instant::now(), names.clone()));
        names
    }
}
