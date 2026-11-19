//! What an Owner learns from trying a model out before relying on it: a few
//! real tasks, and a verdict that says what the model did with them.

use orbite::{
    ActionRequest, Language, Message, Provider, ProviderError, Reply, Request, Tools, Usage,
    try_out,
};
use serde_json::json;

/// A pretend model with the abilities given.
#[derive(Clone, Copy)]
struct Model {
    acts: bool,
    /// False for one that calls Actions but gets their input wrong.
    careful: bool,
    sees: bool,
    remembers: bool,
    strict: bool,
}

const ABLE: Model = Model { acts: true, careful: true, sees: true, remembers: true, strict: false };

impl Provider for Model {
    async fn reply(&self, request: Request<'_>) -> Result<Reply, ProviderError> {
        let say = |text: &str| {
            Ok(Reply {
                text: text.into(),
                actions: Vec::new(),
                kept: None,
                usage: Some(Usage {
                    by: "test".into(),
                    model: "pretend".into(),
                    input: 100,
                    cached: 0,
                    output: 10,
                }),
            })
        };
        let act = |name: &str, input: serde_json::Value| {
            Ok(Reply {
                text: String::new(),
                actions: vec![ActionRequest { id: "1".into(), name: name.into(), input }],
                kept: None,
                usage: None,
            })
        };
        if !request.images.is_empty() {
            return if self.sees {
                say("Red.")
            } else {
                Err(ProviderError::Failed("HTTP 400: images are not supported".into()))
            };
        }
        let offered = |name: &str| request.actions.iter().any(|action| action.name == name);
        match request.conversation.last() {
            Some(Message::Outcome { text, .. }) => say(&format!("That makes {text}.")),
            Some(Message::Owner(asked))
                if asked.contains("code word") && request.conversation.len() > 2 =>
            {
                if self.remembers {
                    say("The code word was **PELICAN\u{2011}42**.")
                } else {
                    say("I do not know.")
                }
            }
            Some(Message::Owner(_)) if !self.acts => {
                say("I cannot do that, but here is how you could.")
            }
            Some(Message::Owner(asked)) if asked.contains("1847") => act(
                "add",
                if self.careful { json!({ "a": 1847, "b": 2596 }) } else { json!({ "a": 1847 }) },
            ),
            Some(Message::Owner(asked)) if asked.contains("Ada") => act(
                "set_reminder",
                if self.careful {
                    json!({ "text": "call Ada", "at": { "day": "tuesday", "hour": 9 } })
                } else {
                    json!({ "text": "call Ada on tuesday at 9" })
                },
            ),
            Some(Message::Owner(_)) if offered("fill_form") => {
                if self.strict {
                    say("I'm sorry, but I can't help with creating accounts.")
                } else {
                    act("fill_form", json!({ "site": "forum.example", "fields": {} }))
                }
            }
            Some(Message::Owner(_)) if offered("send_message") => {
                act("send_message", json!({ "to": "Sam", "text": "See you at 8" }))
            }
            _ => say("Hello there."),
        }
    }
}

#[tokio::test]
async fn an_able_model_gets_a_clean_verdict() {
    let verdict = try_out(&ABLE).await.unwrap();

    assert_eq!(verdict.tools, Tools::Reliable);
    assert!(verdict.sees_images && verdict.remembers == Some(true));
    assert!(verdict.not_tried.is_empty(), "{:?}", verdict.not_tried);
    assert_eq!((verdict.declined, verdict.asked), (0, 2));
    assert!(!verdict.strongly_discouraged());
    assert!(verdict.input_tokens > 0, "what the tryout cost is not counted");
}

#[tokio::test]
async fn a_model_that_cannot_act_is_strongly_discouraged_and_the_owner_is_told_why() {
    let verdict = try_out(&Model { acts: false, ..ABLE }).await.unwrap();

    assert_eq!(verdict.tools, Tools::None);
    assert!(verdict.strongly_discouraged());
    let told = verdict.told(Language::English);
    assert!(told.contains("cannot act"), "{told}");
}

#[tokio::test]
async fn a_model_that_gets_actions_wrong_is_said_to_be_unreliable() {
    let verdict = try_out(&Model { careful: false, ..ABLE }).await.unwrap();

    assert_eq!(verdict.tools, Tools::Unreliable);
    assert!(verdict.told(Language::English).contains("unreliable"));
}

#[tokio::test]
async fn what_a_model_cannot_do_is_said_plainly() {
    let verdict = try_out(&Model { sees: false, remembers: false, ..ABLE }).await.unwrap();

    assert!(!verdict.sees_images && verdict.remembers == Some(false));
    let told = verdict.told(Language::French);
    assert!(told.contains("ne voit pas les images"), "{told}");
}

#[tokio::test]
async fn a_strict_model_is_reported_as_it_is() {
    let verdict = try_out(&Model { strict: true, ..ABLE }).await.unwrap();

    assert_eq!((verdict.declined, verdict.asked), (1, 2));
    let told = verdict.told(Language::English);
    assert!(
        told.contains("did not carry out 1 of 2")
            && told.contains("can't help with creating accounts"),
        "{told}"
    );
    // Strictness is the Owner's to weigh: it does not make a model unusable.
    assert!(!verdict.strongly_discouraged());
}

#[tokio::test]
async fn a_provider_that_refuses_the_key_ends_the_tryout_with_that() {
    struct Refusing;
    impl Provider for Refusing {
        async fn reply(&self, _: Request<'_>) -> Result<Reply, ProviderError> {
            Err(ProviderError::Rejected("bad key".into()))
        }
    }
    assert_eq!(try_out(&Refusing).await.unwrap_err(), ProviderError::Rejected("bad key".into()));
}

#[tokio::test]
async fn what_the_provider_failed_on_is_not_held_against_the_model() {
    /// Able, but its Provider fails on anything long.
    struct Short;
    impl Provider for Short {
        async fn reply(&self, request: Request<'_>) -> Result<Reply, ProviderError> {
            if request.conversation.len() > 5 {
                return Err(ProviderError::Failed("HTTP 413: request too large".into()));
            }
            ABLE.reply(request).await
        }
    }

    let verdict = try_out(&Short).await.unwrap();

    assert_eq!(verdict.remembers, None);
    assert_eq!(verdict.not_tried.len(), 1);
    assert!(verdict.told(Language::English).contains("Not tried"));
}

#[test]
fn every_entry_of_the_catalogue_is_a_dated_verdict() {
    let catalogue: serde_json::Value =
        serde_json::from_str(include_str!("../catalogue/models.json"))
            .expect("the catalogue is JSON");
    for (provider, models) in catalogue.as_object().unwrap() {
        assert!(
            orbite::known_providers().iter().any(|known| known.name == provider),
            "{provider} is not a known Provider"
        );
        for (model, verdict) in models.as_object().unwrap() {
            let verdict: orbite::Verdict = serde_json::from_value(verdict.clone())
                .unwrap_or_else(|error| panic!("{provider} {model}: {error}"));
            assert_eq!(
                verdict.tried.len(),
                10,
                "{provider} {model} does not say when it was tried"
            );
        }
    }
}

#[tokio::test]
async fn what_was_tried_on_this_host_is_kept_and_comes_before_the_catalogue() {
    let home = tempfile::tempdir().unwrap();
    let from_catalogue =
        orbite::rating(home.path(), "groq", "qwen/qwen3.8-27b").expect("it is in the catalogue");
    assert_eq!(from_catalogue.tools, Tools::Reliable);
    assert_eq!(orbite::rating(home.path(), "groq", "a-model-nobody-tried"), None);

    let tried = try_out(&Model { acts: false, ..ABLE }).await.unwrap();
    orbite::keep_rating(home.path(), "groq", "qwen/qwen3.8-27b", &tried).unwrap();

    assert_eq!(orbite::rating(home.path(), "groq", "qwen/qwen3.8-27b"), Some(tried.clone()));
    assert!(tried.in_brief(Language::English).contains("cannot act"));
}
