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
    /// Says which day it is, for what is limited by the day.
    today: Box<dyn Fn() -> String + Send + Sync>,
}

struct Named<P> {
    name: String,
    provider: P,
    /// Until when it is left alone, having failed and not answered since.
    resting_until: Mutex<Option<Instant>>,
    /// The tokens its Owner lets it use in a day, when they set a limit.
    limit: Option<u64>,
    /// The day last counted, and the tokens used on it.
    used: Mutex<(String, u64)>,
}

impl<P> Named<P> {
    fn new(name: &str, provider: P) -> Self {
        Self {
            name: name.to_owned(),
            provider,
            resting_until: Mutex::new(None),
            limit: None,
            used: Mutex::new((String::new(), 0)),
        }
    }

    fn is_resting(&self) -> bool {
        self.resting_until.lock().expect("not poisoned").is_some_and(|until| until > Instant::now())
    }

    /// Notes an answer, or a failure after which it rests for `rest`.
    fn note(&self, rest: Option<Duration>) {
        // A wait no clock can hold is as long a one as there is.
        let far = Duration::from_secs(365 * 24 * 3600);
        *self.resting_until.lock().expect("not poisoned") =
            rest.map(|rest| Instant::now() + rest.min(far));
    }

    /// True once it used, on `day`, all its Owner lets it use in a day.
    fn is_spent(&self, day: &str) -> bool {
        let used = self.used.lock().expect("not poisoned");
        self.limit.is_some_and(|limit| used.0 == day && used.1 >= limit)
    }

    fn count(&self, day: &str, tokens: u64) {
        let mut used = self.used.lock().expect("not poisoned");
        if used.0 != day {
            *used = (day.to_owned(), 0);
        }
        used.1 += tokens;
    }
}

impl<P: Provider> Routed<P> {
    /// Starts with the preferred Provider, known as `name`.
    pub fn through(name: &str, provider: P) -> Self {
        Self {
            usual: vec![Named::new(name, provider)],
            light: None,
            rest: Duration::from_secs(120),
            today: Box::new(|| chrono::Local::now().format("%Y-%m-%d").to_string()),
        }
    }

    /// Lets the Provider added last use this many tokens a day, read and
    /// written together, and no more: the next one takes over from there.
    pub fn limited_to(mut self, tokens_a_day: u64) -> Self {
        if let Some(last) = self.usual.last_mut() {
            last.limit = Some(tokens_a_day);
        }
        self
    }

    /// Counts what the Provider called `name` already used today, before
    /// this program started.
    pub fn having_used(self, name: &str, tokens: u64) -> Self {
        let day = (self.today)();
        // Light work counts with the Provider that does it.
        if let Some(named) = self.usual.iter().find(|named| named.name == plain(name)) {
            named.count(&day, tokens);
        }
        self
    }

    /// Asks `today` which day it is rather than the machine's clock.
    pub fn telling_the_day_with(
        mut self,
        today: impl Fn() -> String + Send + Sync + 'static,
    ) -> Self {
        self.today = Box::new(today);
        self
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

/// The name of the Provider behind `name`, which may be that of its lighter model.
fn plain(name: &str) -> &str {
    name.strip_suffix(" (light)").unwrap_or(name)
}

impl<P: Provider> Provider for Routed<P> {
    fn usual(&self) -> Option<&str> {
        self.usual.first().map(|preferred| preferred.name.as_str())
    }

    async fn reply(&self, request: Request<'_>) -> Result<Reply, ProviderError> {
        let light = self.light.as_ref().filter(|_| request.light);
        let day = (self.today)();
        // One that used up what its Owner allows for the day is not asked.
        // Light work is that Provider's work too: it counts towards its
        // limit and stops with it.
        let counted_with = |candidate: &Named<P>| {
            self.usual.iter().find(|usual| usual.name == plain(&candidate.name))
        };
        let light =
            light.filter(|light| !counted_with(light).is_some_and(|with| with.is_spent(&day)));
        let candidates: Vec<&Named<P>> = light
            .into_iter()
            .chain(self.usual.iter().filter(|candidate| !candidate.is_spent(&day)))
            .collect();
        if candidates.is_empty() && !self.usual.is_empty() {
            return Err(ProviderError::Busy {
                reason: "every AI provider used up the limit set for it today (see /providers)"
                    .into(),
                retry_after: Some(Duration::from_secs(3600)),
            });
        }
        // Those that failed lately come last rather than not at all: when
        // everything else fails too, they are worth another try.
        let (rested, resting): (Vec<_>, Vec<_>) =
            candidates.into_iter().partition(|candidate| !candidate.is_resting());
        let mut first_failure = None;
        for candidate in rested.into_iter().chain(resting) {
            match candidate.provider.reply(request).await {
                Ok(mut reply) => {
                    candidate.note(None);
                    // Whoever answered is known by the name given here.
                    let usage = reply.usage.take().unwrap_or_default();
                    counted_with(candidate)
                        .unwrap_or(candidate)
                        .count(&day, usage.input + usage.output);
                    reply.usage = Some(Usage { by: candidate.name.clone(), ..usage });
                    return Ok(reply);
                }
                // Another Provider may well do better.
                Err(
                    failure @ (ProviderError::Unreachable(_)
                    | ProviderError::Busy { .. }
                    | ProviderError::Rejected(_)),
                ) => {
                    // One that says for how long it cannot answer, such as a
                    // plan out of usage, is left alone that long.
                    let asked = match &failure {
                        ProviderError::Busy { retry_after, .. } => retry_after.unwrap_or_default(),
                        _ => Duration::ZERO,
                    };
                    candidate.note(Some(self.rest.max(asked)));
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
