//! The Owner's ChatGPT plan as a Provider, by the path OpenAI documents for
//! open-source programs running on their user's machine ("Sign in with
//! ChatGPT", ChatGPT plan usage). See `docs/research/chatgpt-sign-in.md`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::attachments::image_type;
use crate::{ActionRequest, Message, Provider, ProviderError, Reply, Request, Secrets, Usage};

/// Where the Owner's browser ends once they agreed. Nothing listens there:
/// the Owner sends the address back. OpenAI wants this very host.
pub(crate) const ENDS_AT: &str = "http://127.0.0.1:1455/auth/callback";
/// What the tokens are for, whatever address is spoken to.
const RESOURCE: &str = "https://api.openai.com/v1";
const ISSUER: &str = "https://auth.openai.com";
const SCOPES: &str =
    "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct";
/// Without this one the Owner's plan may not be used.
const PLAN_SCOPE: &str = "chatgpt.tokens.use.direct";
/// Under which the Agent's Actions are grouped: this path takes no bare ones.
const NAMESPACE: &str = "orbite";
pub(crate) const KEPT_AS: &str = "ORBITE_CHATGPT";
/// Where the Owner sees and limits what their plan is used for.
pub(crate) const MANAGE_USAGE: &str = "https://chatgpt.com/settings/usage";
const SIGN_IN_AGAIN: &str = "sign in to ChatGPT again: send me /providers connect chatgpt, or run \
                             `orbite setup provider` on the machine where I run";
/// A refresh that failed in one of these ways will never work again.
const ENDED: &[&str] = &[
    "invalid_grant",
    "invalid_refresh_token",
    "token_expired",
    "refresh_token_expired",
    "refresh_token_invalidated",
    "refresh_token_reused",
];

/// What is kept, sealed, of the Owner's sign-in.
#[derive(Default, Serialize, Deserialize)]
struct Kept {
    /// The name this machine goes by with OpenAI, drawn once.
    #[serde(default)]
    host: String,
    /// The name OpenAI gave this Agent the first time it signed in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    client: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    access: Option<String>,
    /// Replaced each time it is used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    refresh: Option<String>,
    /// Who signed in, as OpenAI wrote it; shown to it when signing in again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    identity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    email: Option<String>,
    /// When `access` stops working, in seconds since 1970.
    #[serde(default)]
    until: u64,
}

/// A sign-in under way: the link sent to the Owner and what proves, later,
/// that an address answers it.
pub struct Begun {
    link: String,
    verifier: String,
    state: String,
    nonce: String,
}

impl Begun {
    /// What the Owner opens in a browser, on any device.
    pub fn link(&self) -> &str {
        &self.link
    }
}

/// OpenAI's models, paid for by the Owner's ChatGPT plan.
pub struct ChatGpt {
    http: reqwest::Client,
    home: PathBuf,
    secrets: Secrets,
    /// Where signing in happens.
    auth: String,
    /// Where models answer.
    api: String,
    model: String,
    name: String,
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

fn unguessable() -> Result<String, String> {
    let mut random = [0u8; 32];
    getrandom::fill(&mut random).map_err(|error| error.to_string())?;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(random))
}

/// A name for this machine no other has, in the form OpenAI takes.
fn host_name() -> Result<String, String> {
    let mut random = [0u8; 16];
    getrandom::fill(&mut random).map_err(|error| error.to_string())?;
    random[6] = (random[6] & 0x0f) | 0x40;
    random[8] = (random[8] & 0x3f) | 0x80;
    let hex: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
    Ok(format!(
        "urn:uuid:{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    ))
}

fn form_written(fields: &[(&str, &str)]) -> String {
    url::form_urlencoded::Serializer::new(String::new()).extend_pairs(fields).finish()
}

/// What an identity token says, unverified: it came straight from OpenAI
/// over a connection that proves who is speaking, which OpenID Connect
/// accepts in place of checking its signature.
fn claims(identity: &str) -> Option<Value> {
    let written = identity.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(written.trim_end_matches('='))
        .ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn root_cause(error: &dyn std::error::Error) -> String {
    let mut cause = error;
    while let Some(source) = cause.source() {
        cause = source;
    }
    cause.to_string()
}

/// The code and the words of a refusal, wherever OpenAI put them.
fn refusal_in(body: &Value) -> (String, String) {
    let error = &body["error"];
    let code = error["code"].as_str().or(error.as_str()).or(body["code"].as_str()).unwrap_or("");
    let words = error["message"]
        .as_str()
        .or(body["message"].as_str())
        .or(body["error_description"].as_str())
        .or(body["detail"].as_str())
        .unwrap_or(code);
    (code.to_owned(), words.to_owned())
}

/// Why OpenAI gave no answer, from the status and the body of its refusal.
fn refused(status: u16, body: &Value, retry_after: Option<Duration>) -> ProviderError {
    let (code, words) = refusal_in(body);
    match (code.as_str(), status) {
        ("subscription_sharing_usage_limit_exceeded", _) => ProviderError::Busy {
            reason: format!(
                "Usage limit reached: your ChatGPT plan has no usage left for me for now. \
                 Manage usage: {MANAGE_USAGE}"
            ),
            // Not lifted in the time anything waits: it is not asked again.
            retry_after: Some(Duration::from_secs(3600)),
        },
        ("subscription_sharing_usage_unavailable" | "subscription_sharing_user_unavailable", _) => {
            ProviderError::Busy { reason: words, retry_after }
        }
        ("subscription_sharing_user_not_eligible", _) => ProviderError::Rejected(format!(
            "this ChatGPT account may not use its plan here: it takes an eligible Plus or Pro \
             plan, and the account's workspace must allow it ({words})"
        )),
        (_, 401) => ProviderError::Rejected(format!("{words}: {SIGN_IN_AGAIN}")),
        (_, 403) => ProviderError::Rejected(words),
        (_, 429) => ProviderError::Busy { reason: words, retry_after },
        (_, 500..) => {
            ProviderError::Busy { reason: format!("HTTP {status}: {words}"), retry_after }
        }
        // Met while answering: OpenAI's own trouble, not the request's.
        (code, 200)
            if ["server_error", "rate_limit", "overloaded", "unavailable"]
                .iter()
                .any(|sign| code.contains(sign)) =>
        {
            ProviderError::Busy { reason: words, retry_after }
        }
        (_, 200) => ProviderError::Failed(words),
        _ => ProviderError::Failed(format!("HTTP {status}: {words}")),
    }
}

impl ChatGpt {
    /// The ChatGPT plan signed in to in the installation whose folder is
    /// `home`, thinking with `model`.
    pub fn kept_in(home: impl AsRef<Path>, model: impl Into<String>) -> Self {
        Self {
            http: reqwest::Client::new(),
            home: home.as_ref().to_owned(),
            secrets: Secrets::kept_in(home.as_ref()),
            auth: ISSUER.into(),
            api: RESOURCE.into(),
            model: model.into(),
            name: "chatgpt".into(),
        }
    }

    /// Reaches OpenAI at this address rather than its own.
    pub fn at(mut self, address: impl AsRef<str>) -> Self {
        let address = address.as_ref().trim_end_matches('/');
        self.auth = address.to_owned();
        self.api = address.to_owned();
        self
    }

    /// The name this Provider goes by in what the Owner is shown.
    pub fn named(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    fn kept(&self) -> Result<Kept, String> {
        let written = self.secrets.get(KEPT_AS).map_err(|error| error.to_string())?;
        Ok(written.and_then(|written| serde_json::from_str(&written).ok()).unwrap_or_default())
    }

    fn keep(&self, kept: &Kept) -> Result<(), String> {
        let written = serde_json::to_string(kept).map_err(|error| error.to_string())?;
        self.secrets.set(KEPT_AS, &written).map(|_| ()).map_err(|error| error.to_string())
    }

    /// True once the Owner signed in and until that sign-in ends.
    pub fn is_signed_in(&self) -> bool {
        self.kept().is_ok_and(|kept| kept.refresh.is_some())
    }

    /// The account signed in with, when OpenAI said.
    pub fn account(&self) -> Option<String> {
        self.kept().ok().and_then(|kept| kept.email)
    }

    /// Begins a sign-in: a link for the Owner to follow in any browser.
    pub fn sign_in(&self) -> Result<Begun, String> {
        let mut kept = self.kept()?;
        if kept.host.is_empty() {
            kept.host = host_name()?;
            self.keep(&kept)?;
        }
        let (verifier, state, nonce) = (unguessable()?, unguessable()?, unguessable()?);
        let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(Sha256::digest(verifier.as_bytes()));
        let mut asked = vec![
            ("response_type", "code"),
            ("client_id", kept.client.as_deref().unwrap_or("dynamic_agent_client")),
            ("ext_agent_host_id", kept.host.as_str()),
            ("redirect_uri", ENDS_AT),
            ("scope", SCOPES),
            ("resource", RESOURCE),
            ("state", state.as_str()),
            ("nonce", nonce.as_str()),
            ("code_challenge_method", "S256"),
            ("code_challenge", challenge.as_str()),
        ];
        match &kept.client {
            // OpenAI knows this Agent: it is told who signed in before.
            Some(_) => {
                if let Some(identity) = &kept.identity {
                    asked.push(("id_token_hint", identity));
                }
                if let Some(email) = &kept.email {
                    asked.push(("login_hint", email));
                }
            }
            None => asked.push(("agent_name_hint", "Orbite")),
        }
        let link = format!("{}/api/accounts/authorize?{}", self.auth, form_written(&asked));
        Ok(Begun { link, verifier, state, nonce })
    }

    /// Finishes the sign-in `begun` with the address the Owner's browser
    /// ended on. Says which account signed in. The text of a failure is for
    /// the Owner.
    pub async fn signed_in_at(&self, begun: &Begun, ended_on: &str) -> Result<String, String> {
        let ended = url::Url::parse(ended_on.trim()).map_err(|_| {
            "that is not an address: copy the whole address your browser ended on, starting \
             with http://127.0.0.1"
                .to_owned()
        })?;
        let given: HashMap<String, String> = ended.query_pairs().into_owned().collect();
        // Only the answer to the very link that was sent is taken.
        if given.get("state") != Some(&begun.state) {
            return Err("that address is not the answer to the link I sent".into());
        }
        let Some(code) = given.get("code") else {
            let why = given.get("error").cloned().unwrap_or_default();
            return Err(format!("the sign-in was not granted ({why})"));
        };
        let mut kept = self.kept()?;
        let client = match (&kept.client, given.get("client_id")) {
            (Some(known), Some(named)) if known != named => {
                return Err("that address names another program than this one".into());
            }
            (Some(known), _) => known.clone(),
            (None, Some(named)) => named.clone(),
            (None, None) => {
                return Err("OpenAI did not finish registering this program: try again".into());
            }
        };
        let form = form_written(&[
            ("grant_type", "authorization_code"),
            ("client_id", &client),
            ("code", code),
            ("code_verifier", &begun.verifier),
            ("redirect_uri", ENDS_AT),
            ("resource", RESOURCE),
        ]);
        let answer = self
            .http
            .post(format!("{}/api/accounts/oauth/token", self.auth))
            .header("content-type", "application/x-www-form-urlencoded")
            .body(form)
            .timeout(Duration::from_secs(30))
            .send()
            .await
            .map_err(|error| format!("OpenAI could not be reached ({})", root_cause(&error)))?;
        let granted = answer.json::<Value>().await.unwrap_or_default();
        let (Some(access), Some(refresh)) =
            (granted["access_token"].as_str(), granted["refresh_token"].as_str())
        else {
            let (_, words) = refusal_in(&granted);
            return Err(format!("OpenAI did not accept the sign-in ({words}): start again"));
        };
        if !granted["scope"].as_str().unwrap_or("").split(' ').any(|scope| scope == PLAN_SCOPE) {
            return Err("this sign-in does not allow your ChatGPT plan to be used here: it takes \
                        an eligible Plus or Pro plan, and agreeing to its use when signing in"
                .into());
        }
        let identity = granted["id_token"].as_str().unwrap_or("");
        let said = claims(identity).unwrap_or_default();
        let for_this_program = match &said["aud"] {
            Value::String(audience) => *audience == client,
            Value::Array(audiences) => audiences.iter().any(|audience| audience == client.as_str()),
            _ => false,
        };
        if said["iss"] != ISSUER
            || !for_this_program
            || said["nonce"] != begun.nonce.as_str()
            || said["exp"].as_u64().is_none_or(|ends| ends <= now())
        {
            return Err(
                "the identity OpenAI sent was not made for this sign-in: start again".into()
            );
        }
        let email = said["email"].as_str().map(str::to_owned);
        kept.client = Some(client);
        kept.access = Some(access.to_owned());
        kept.refresh = Some(refresh.to_owned());
        kept.identity = Some(identity.to_owned());
        kept.email = email.clone();
        kept.until = now() + granted["expires_in"].as_u64().unwrap_or(0);
        self.keep(&kept)?;
        Ok(email.unwrap_or_default())
    }

    /// Ends the sign-in: OpenAI is told to take back what let the Agent
    /// in, and nothing of it is kept, whether or not OpenAI could be told.
    pub async fn sign_out(&self) {
        if let Ok(kept) = self.kept()
            && let (Some(refresh), Some(client)) = (&kept.refresh, &kept.client)
        {
            let form = form_written(&[
                ("token", refresh),
                ("token_type_hint", "refresh_token"),
                ("client_id", client),
            ]);
            let _ = self
                .http
                .post(format!("{}/api/accounts/oauth/revoke", self.auth))
                .header("content-type", "application/x-www-form-urlencoded")
                .body(form)
                .timeout(Duration::from_secs(20))
                .send()
                .await;
        }
        let _ = self.secrets.remove(KEPT_AS);
    }

    /// Alone, in this program and in any other of the same installation, to
    /// renew what lets the Agent in: a renewal used twice ends the sign-in.
    async fn renewing_alone(&self) -> Result<std::fs::File, ProviderError> {
        let beside = self.home.join("chatgpt.renewing");
        tokio::task::spawn_blocking(move || {
            use std::os::fd::AsRawFd;
            let lock = std::fs::File::create(beside)?;
            // SAFETY: a lock on a file opened just above, let go when it is closed.
            if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX) } != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(lock)
        })
        .await
        .map_err(|error| ProviderError::Unreachable(error.to_string()))?
        .map_err(|error| {
            ProviderError::Unreachable(format!("could not renew the sign-in: {error}"))
        })
    }

    /// What lets the Agent in right now, renewed when it ran out or when
    /// OpenAI just refused it as `stale`.
    async fn access(&self, stale: Option<&str>) -> Result<String, ProviderError> {
        let not_signed_in =
            || ProviderError::Rejected(format!("nobody is signed in to ChatGPT: {SIGN_IN_AGAIN}"));
        let usable = |kept: &Kept| {
            kept.access
                .clone()
                .filter(|access| kept.until > now() + 60 && Some(access.as_str()) != stale)
        };
        let kept = self.kept().map_err(ProviderError::Unreachable)?;
        if kept.refresh.is_none() {
            return Err(not_signed_in());
        }
        if let Some(access) = usable(&kept) {
            return Ok(access);
        }
        // Carried to its end even when whoever asked stops waiting: a
        // renewal OpenAI granted and Orbite did not keep ends the sign-in.
        let renewing = Self {
            http: self.http.clone(),
            home: self.home.clone(),
            secrets: Secrets::kept_in(&self.home),
            auth: self.auth.clone(),
            api: self.api.clone(),
            model: self.model.clone(),
            name: self.name.clone(),
        };
        let stale = stale.map(str::to_owned);
        tokio::spawn(async move { renewing.renew(stale.as_deref()).await })
            .await
            .unwrap_or_else(|error| Err(ProviderError::Unreachable(error.to_string())))
    }

    async fn renew(&self, stale: Option<&str>) -> Result<String, ProviderError> {
        let not_signed_in =
            || ProviderError::Rejected(format!("nobody is signed in to ChatGPT: {SIGN_IN_AGAIN}"));
        let usable = |kept: &Kept| {
            kept.access
                .clone()
                .filter(|access| kept.until > now() + 60 && Some(access.as_str()) != stale)
        };
        let _alone = self.renewing_alone().await?;
        // Another may have renewed while this one waited. Failing to read
        // says nothing of the request: another Provider may answer.
        let mut kept = self.kept().map_err(ProviderError::Unreachable)?;
        if let Some(access) = usable(&kept) {
            return Ok(access);
        }
        let (Some(refresh), Some(client)) = (kept.refresh.clone(), kept.client.clone()) else {
            return Err(not_signed_in());
        };
        let form = form_written(&[
            ("grant_type", "refresh_token"),
            ("client_id", &client),
            ("refresh_token", &refresh),
            ("resource", RESOURCE),
        ]);
        let answer = self
            .http
            .post(format!("{}/api/accounts/oauth/token", self.auth))
            .header("content-type", "application/x-www-form-urlencoded")
            .body(form)
            .timeout(Duration::from_secs(30))
            .send()
            .await
            .map_err(|error| ProviderError::Unreachable(root_cause(&error)))?;
        let status = answer.status().as_u16();
        let granted = answer.json::<Value>().await.unwrap_or_default();
        if let Some(access) = granted["access_token"].as_str() {
            kept.access = Some(access.to_owned());
            if let Some(next) = granted["refresh_token"].as_str() {
                kept.refresh = Some(next.to_owned());
            }
            if let Some(identity) = granted["id_token"].as_str() {
                kept.identity = Some(identity.to_owned());
            }
            kept.until = now() + granted["expires_in"].as_u64().unwrap_or(0);
            // Not kept, it still lets the Agent in this once; said aloud,
            // since the next renewal will not find it.
            if let Err(error) = self.keep(&kept) {
                eprintln!("The renewed sign-in to ChatGPT could not be kept: {error}");
            }
            return Ok(access.to_owned());
        }
        let (code, words) = refusal_in(&granted);
        if ENDED.contains(&code.as_str()) {
            // Who this Agent is to OpenAI is kept: it signs in again as itself.
            kept.access = None;
            kept.refresh = None;
            let _ = self.keep(&kept);
            return Err(ProviderError::Rejected(format!(
                "the sign-in to ChatGPT ended ({code}): {SIGN_IN_AGAIN}"
            )));
        }
        Err(ProviderError::Busy {
            reason: format!("OpenAI could not renew the sign-in (HTTP {status}: {words})"),
            retry_after: None,
        })
    }

    /// The models the Owner's plan offers: what to send, and what to show.
    pub async fn models(&self) -> Vec<(String, String)> {
        let Ok(access) = self.access(None).await else { return Vec::new() };
        let asking = self
            .http
            .get(format!("{}/models", self.api))
            .bearer_auth(access)
            .timeout(Duration::from_secs(20));
        let Ok(answer) = asking.send().await else { return Vec::new() };
        let Ok(listed) = answer.json::<Value>().await else { return Vec::new() };
        let models = listed["models"].as_array().cloned().unwrap_or_default();
        models
            .iter()
            .filter(|model| model["visibility"] == "list")
            .filter_map(|model| {
                let slug = model["slug"].as_str()?;
                let shown = model["display_name"].as_str().unwrap_or(slug);
                Some((slug.to_owned(), shown.to_owned()))
            })
            .collect()
    }

    /// Asks once, as whoever `access` lets in. A refusal comes with its status.
    async fn ask(&self, body: &Value, access: &str) -> Result<Reply, (u16, ProviderError)> {
        let response = self
            .http
            .post(format!("{}/responses", self.api))
            .bearer_auth(access)
            .json(body)
            // Long enough for the longest answer; a stream that stalls ends.
            .timeout(Duration::from_secs(600))
            .send()
            .await
            .map_err(|error| (0, ProviderError::Unreachable(root_cause(&error))))?;
        let status = response.status().as_u16();
        // What OpenAI says of the plan beside its answer: when it has
        // something to say of what is left, or of when there is more.
        let beside: Vec<(String, String)> = response
            .headers()
            .iter()
            .filter(|(name, _)| {
                ["limit", "reset", "retry", "credit", "usage", "plan"]
                    .iter()
                    .any(|sign| name.as_str().contains(sign))
            })
            .filter_map(|(name, value)| Some((name.to_string(), value.to_str().ok()?.to_owned())))
            .collect();
        let retry_after = response
            .headers()
            .get("retry-after")
            .and_then(|value| value.to_str().ok()?.trim().parse::<f64>().ok())
            .filter(|seconds| seconds.is_finite() && (0.0..=86_400.0).contains(seconds))
            .map(Duration::from_secs_f64);
        let written = response
            .text()
            .await
            .map_err(|error| (status, ProviderError::Failed(root_cause(&error))))?;
        if status != 200 {
            // Kept where whoever looks after the machine reads it: a refusal
            // holds no key, and says more than what is passed on.
            let shown: String = written.chars().take(1500).collect();
            eprintln!("ChatGPT refused (HTTP {status}): {shown} {beside:?}");
            let body = serde_json::from_str(&written).unwrap_or(json!({ "message": written }));
            return Err((status, refused(status, &body, retry_after)));
        }

        let (mut text, mut actions, mut usage, mut ended) =
            (String::new(), Vec::new(), None, false);
        // What a message said, when it came whole, and what the model declined.
        let (mut whole, mut declined) = (String::new(), String::new());
        for line in written.lines() {
            let Some(event) = line.strip_prefix("data:") else { continue };
            let Ok(event) = serde_json::from_str::<Value>(event.trim()) else { continue };
            match event["type"].as_str().unwrap_or("") {
                "response.output_text.delta" => {
                    text.push_str(event["delta"].as_str().unwrap_or(""))
                }
                "response.output_item.done" if event["item"]["type"] == "function_call" => {
                    let item = &event["item"];
                    let name = item["name"].as_str().unwrap_or("").to_owned();
                    let arguments = item["arguments"].as_str().unwrap_or("");
                    let input = if arguments.trim().is_empty() {
                        json!({})
                    } else {
                        serde_json::from_str(arguments).map_err(|_| {
                            let unreadable = format!("the model gave unreadable input for {name}");
                            (status, ProviderError::Failed(unreadable))
                        })?
                    };
                    let id = item["call_id"].as_str().unwrap_or("").to_owned();
                    actions.push(ActionRequest { id, name, input });
                }
                "response.output_item.done" if event["item"]["type"] == "message" => {
                    for part in event["item"]["content"].as_array().into_iter().flatten() {
                        match part["type"].as_str() {
                            Some("output_text") => {
                                whole.push_str(part["text"].as_str().unwrap_or(""));
                            }
                            Some("refusal") => {
                                declined.push_str(part["refusal"].as_str().unwrap_or(""));
                            }
                            _ => {}
                        }
                    }
                }
                "response.completed" => {
                    let used = &event["response"]["usage"];
                    let count = |value: &Value| value.as_u64().unwrap_or(0);
                    usage = Some(Usage {
                        by: self.name.clone(),
                        model: self.model.clone(),
                        input: count(&used["input_tokens"]),
                        cached: count(&used["input_tokens_details"]["cached_tokens"]),
                        output: count(&used["output_tokens"]),
                    });
                    ended = true;
                }
                "response.failed" => {
                    let shown: String = event.to_string().chars().take(1500).collect();
                    eprintln!("ChatGPT gave up while answering: {shown} {beside:?}");
                    return Err((status, refused(status, &event["response"], None)));
                }
                "error" => {
                    let shown: String = event.to_string().chars().take(1500).collect();
                    eprintln!("ChatGPT gave up while answering: {shown} {beside:?}");
                    return Err((status, refused(status, &event, None)));
                }
                "response.incomplete" => {
                    let why = event["response"]["incomplete_details"]["reason"]
                        .as_str()
                        .unwrap_or("no reason given");
                    let cut = format!("the answer was left unfinished ({why})");
                    return Err((status, ProviderError::Failed(cut)));
                }
                _ => {}
            }
        }
        // Only an answer read to its end is one.
        if !ended {
            let reason = "the answer was cut before its end".to_owned();
            return Err((status, ProviderError::Busy { reason, retry_after: None }));
        }
        if text.is_empty() {
            text = whole;
        }
        if text.is_empty() && actions.is_empty() && !declined.is_empty() {
            return Err((status, ProviderError::Declined(declined)));
        }
        if text.is_empty() && actions.is_empty() {
            return Err((status, ProviderError::Failed("the answer contained no text".into())));
        }
        Ok(Reply { text, actions, kept: None, usage })
    }
}

impl Provider for ChatGpt {
    async fn reply(&self, request: Request<'_>) -> Result<Reply, ProviderError> {
        let mut input = Vec::new();
        // What the Owner or Orbite says, with the pictures that came with it:
        // a photo sent, or the Computer's screen that comes with a notice.
        let said = |place: usize, text: String| {
            let mut parts = Vec::new();
            for (_, image) in request.images.iter().filter(|(at, _)| *at == place) {
                let (Some(kind), Ok(bytes)) = (image_type(image), std::fs::read(image)) else {
                    continue;
                };
                let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
                parts.push(json!({
                    "type": "input_image",
                    "image_url": format!("data:{kind};base64,{encoded}"),
                }));
            }
            if parts.is_empty() {
                return json!({ "role": "user", "content": text });
            }
            parts.insert(0, json!({ "type": "input_text", "text": text }));
            json!({ "role": "user", "content": parts })
        };
        for (place, message) in request.conversation.iter().enumerate() {
            match message {
                Message::Owner(text) => input.push(said(place, text.clone())),
                Message::Agent { text, actions, .. } => {
                    if !text.is_empty() {
                        input.push(json!({ "role": "assistant", "content": text }));
                    }
                    for action in actions {
                        input.push(json!({
                            "type": "function_call",
                            "call_id": action.id,
                            "namespace": NAMESPACE,
                            "name": action.name,
                            "arguments": action.input.to_string(),
                        }));
                    }
                }
                Message::Outcome { action, text } => input.push(json!({
                    "type": "function_call_output", "call_id": action, "output": text,
                })),
                Message::Notice(text) => input.push(said(
                    place,
                    format!("[Notice for you, not written by your Owner]\n{text}"),
                )),
            }
        }
        // Nothing else: this path refuses most of what the API takes elsewhere.
        let mut body =
            json!({ "model": self.model, "input": input, "store": false, "stream": true });
        if !request.instructions.is_empty() {
            body["instructions"] = json!(request.instructions);
        }
        if !request.actions.is_empty() {
            let tools: Vec<Value> = request
                .actions
                .iter()
                .map(|action| {
                    json!({
                        "type": "function",
                        "name": action.name,
                        "description": action.description,
                        "parameters": action.input,
                    })
                })
                .collect();
            body["tools"] = json!([{
                "type": "namespace",
                "name": NAMESPACE,
                "description": "Everything the agent can do for its owner.",
                "tools": tools,
            }]);
        }

        // The model is asked to think before it answers: more for what the
        // Owner asked than for the light work of summaries and wake-ups.
        body["reasoning"] = json!({ "effort": if request.light { "low" } else { "medium" } });

        let access = self.access(None).await?;
        let mut asked = self.ask(&body, &access).await;
        // A plan that does not take being told how much to think is asked
        // the plain way, as it was before.
        if let Err((400, _)) = &asked {
            body.as_object_mut().map(|body| body.remove("reasoning"));
            asked = self.ask(&body, &access).await;
        }
        match asked {
            Ok(reply) => Ok(reply),
            // What let the Agent in may only have run out early: once more,
            // renewed.
            Err((401, _)) => {
                let renewed = self.access(Some(&access)).await?;
                self.ask(&body, &renewed).await.map_err(|(_, failure)| failure)
            }
            Err((_, failure)) => Err(failure),
        }
    }
}
