use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::{Provider, ProviderError, Reply, Request, Usage};

/// Several Providers behind one: the first that can answers, the next takes
/// over when one is down, busy or refuses the key, and lighter work goes to
/// a lighter model when there is one.
pub struct Routed<P> {
    /// The Providers for all work, the preferred one first.
    usual: Vec<Named<P>>,
    /// The Provider for light work, tried before the usual ones.
    light: Option<Named<P>>,
    /// How long a Provider that failed is left alone before it is tried again.
    rest: Duration,
}

struct Named<P> {
    name: String,
    provider: P,
    /// When it last failed, if it has not answered since.
    failed_at: Mutex<Option<Instant>>,
}

impl<P> Named<P> {
    fn new(name: &str, provider: P) -> Self {
        Self { name: name.to_owned(), provider, failed_at: Mutex::new(None) }
    }

    fn is_resting(&self, rest: Duration) -> bool {
        self.failed_at.lock().expect("not poisoned").is_some_and(|at| at.elapsed() < rest)
    }

    fn note(&self, failed: bool) {
        *self.failed_at.lock().expect("not poisoned") = failed.then(Instant::now);
    }
}

impl<P: Provider> Routed<P> {
    /// Starts with the preferred Provider, known as `name`.
    pub fn through(name: &str, provider: P) -> Self {
        Self {
            usual: vec![Named::new(name, provider)],
            light: None,
            rest: Duration::from_secs(120),
        }
    }

    /// Adds a Provider to fall back on when those before it cannot answer.
    pub fn or(mut self, name: &str, provider: P) -> Self {
        self.usual.push(Named::new(name, provider));
        self
    }

    /// Sends light work to this Provider first.
    pub fn with_light(mut self, name: &str, provider: P) -> Self {
        self.light = Some(Named::new(name, provider));
        self
    }

    /// How long a Provider that failed is left alone before it is tried again.
    pub fn resting_failed_providers_for(mut self, rest: Duration) -> Self {
        self.rest = rest;
        self
    }
}

impl<P: Provider> Provider for Routed<P> {
    fn usual(&self) -> Option<&str> {
        self.usual.first().map(|preferred| preferred.name.as_str())
    }

    async fn reply(&self, request: Request<'_>) -> Result<Reply, ProviderError> {
        let light = self.light.as_ref().filter(|_| request.light);
        let candidates: Vec<&Named<P>> = light.into_iter().chain(&self.usual).collect();
        // Those that failed lately come last rather than not at all: when
        // everything else fails too, they are worth another try.
        let (rested, resting): (Vec<_>, Vec<_>) =
            candidates.into_iter().partition(|candidate| !candidate.is_resting(self.rest));
        let mut first_failure = None;
        for candidate in rested.into_iter().chain(resting) {
            match candidate.provider.reply(request).await {
                Ok(mut reply) => {
                    candidate.note(false);
                    // Whoever answered is known by the name given here.
                    let usage = reply.usage.take().unwrap_or_default();
                    reply.usage = Some(Usage { by: candidate.name.clone(), ..usage });
                    return Ok(reply);
                }
                // Another Provider may well do better.
                Err(
                    failure @ (ProviderError::Unreachable(_)
                    | ProviderError::Busy { .. }
                    | ProviderError::Rejected(_)),
                ) => {
                    candidate.note(true);
                    first_failure.get_or_insert(failure);
                }
                // The request itself is the trouble: it would be anywhere.
                Err(failure) => return Err(failure),
            }
        }
        Err(first_failure
            .unwrap_or_else(|| ProviderError::Failed("no Provider is connected".into())))
    }
}
