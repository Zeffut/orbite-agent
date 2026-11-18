//! What an Owner observes of the Agent's Skills.

mod support;

use std::fs;
use std::path::Path;

use orbite::{Agent, Message, Store};
use serde_json::json;
use support::{Owner, ScriptedProvider, memory_channel};

fn agent_learning_in(skills: &Path, provider: &ScriptedProvider) -> Owner {
    let (mut channel, owner) = memory_channel();
    let provider = provider.clone();
    let skills = skills.to_owned();
    tokio::spawn(async move {
        Agent::new(provider, Store::in_memory()).learning_in(skills).converse(&mut channel).await
    });
    owner
}

fn outcome(provider: &ScriptedProvider) -> String {
    match provider.last_asked().pop() {
        Some(Message::Outcome { text, .. }) => text,
        other => panic!("the model was not given an Outcome but {other:?}"),
    }
}

/// A Skill as another agent's tooling would have written it.
const FOREIGN_SKILL: &str = "---
name: pdf-forms
description: >
  Fill in PDF forms. Use when the user wants a PDF form
  completed or its fields listed.
license: MIT
metadata:
  author: someone-else
---

# PDF forms

1. Run `scripts/fields.py` on the form.
2. Fill each field.
";

fn install_foreign_skill(skills: &Path) {
    fs::create_dir_all(skills.join("pdf-forms/scripts")).unwrap();
    fs::write(skills.join("pdf-forms/SKILL.md"), FOREIGN_SKILL).unwrap();
    fs::write(skills.join("pdf-forms/scripts/fields.py"), "print('fields')").unwrap();
}

#[tokio::test]
async fn the_model_is_told_which_skills_exist_but_not_their_content() {
    let skills = tempfile::tempdir().unwrap();
    install_foreign_skill(skills.path());
    let provider = ScriptedProvider::answering(["Hi."]);
    let mut owner = agent_learning_in(skills.path(), &provider);

    owner.says("Hi");
    owner.hears().await;

    let instructions = provider.last_instructions();
    assert!(
        instructions.contains(
            "pdf-forms: Fill in PDF forms. Use when the user wants a PDF form completed or its fields listed."
        ),
        "{instructions}"
    );
    assert!(!instructions.contains("fields.py"), "a Skill's content is loaded only when used");
}

#[tokio::test]
async fn a_skill_written_for_another_agent_is_used_unchanged() {
    let skills = tempfile::tempdir().unwrap();
    install_foreign_skill(skills.path());
    let provider = ScriptedProvider::default();
    provider.then_acts([("use_skill", json!({ "name": "pdf-forms" }))]).then(Ok("Filled.".into()));
    let mut owner = agent_learning_in(skills.path(), &provider);

    owner.says("Fill in this form.");
    owner.hears().await;

    let outcome = outcome(&provider);
    assert!(outcome.contains("1. Run `scripts/fields.py` on the form."), "{outcome}");
    assert!(!outcome.contains("license: MIT"), "the header is not instructions: {outcome}");
    let folder = skills.path().join("pdf-forms");
    assert!(
        outcome.contains(folder.to_str().unwrap()),
        "the model must know where its files are: {outcome}"
    );
    assert!(owner.was_asked_nothing());
}

#[tokio::test]
async fn saving_a_skill_waits_for_the_owner_and_writes_it_in_the_open_format() {
    let skills = tempfile::tempdir().unwrap();
    let provider = ScriptedProvider::default();
    provider
        .then_acts([(
            "save_skill",
            json!({
                "name": "Weekly Report",
                "description": "Write the Owner's weekly report. Use every Friday or when asked for the report.",
                "instructions": "1. Read the week's notes.\n2. Summarise in five lines.",
            }),
        )])
        .then(Ok("Saved.".into()));
    let mut owner = agent_learning_in(skills.path(), &provider);

    owner.says("Remember how to do that.");
    let approval = owner.is_asked().await;
    assert!(approval.detail.contains("Summarise in five lines"), "{approval:?}");
    assert!(!approval.irreversible, "{approval:?}");
    assert!(!skills.path().join("weekly-report").exists(), "saved before the Approval");
    owner.allows();
    owner.hears().await;

    assert_eq!(
        fs::read_to_string(skills.path().join("weekly-report/SKILL.md")).unwrap(),
        "---\nname: weekly-report\ndescription: \"Write the Owner's weekly report. Use every Friday or when asked for the report.\"\n---\n\n1. Read the week's notes.\n2. Summarise in five lines.\n"
    );
    assert!(
        provider.last_instructions().contains("weekly-report: Write the Owner's weekly report.")
    );
}

#[tokio::test]
async fn a_denied_skill_is_not_saved() {
    let skills = tempfile::tempdir().unwrap();
    let provider = ScriptedProvider::default();
    provider
        .then_acts([(
            "save_skill",
            json!({ "name": "x", "description": "d", "instructions": "i" }),
        )])
        .then(Ok("Fine.".into()));
    let mut owner = agent_learning_in(skills.path(), &provider);

    owner.says("Go.");
    owner.is_asked().await;
    owner.denies();
    owner.hears().await;

    assert_eq!(fs::read_dir(skills.path()).unwrap().count(), 0);
}

#[tokio::test]
async fn improving_a_skill_keeps_its_other_files_and_says_the_old_text_is_lost() {
    let skills = tempfile::tempdir().unwrap();
    install_foreign_skill(skills.path());
    let provider = ScriptedProvider::default();
    provider
        .then_acts([(
            "save_skill",
            json!({ "name": "pdf-forms", "description": "Fill in PDF forms.", "instructions": "Better steps." }),
        )])
        .then(Ok("Improved.".into()));
    let mut owner = agent_learning_in(skills.path(), &provider);

    owner.says("Improve it.");
    let approval = owner.is_asked().await;
    assert!(approval.irreversible, "{approval:?}");
    owner.allows();
    owner.hears().await;

    assert!(
        fs::read_to_string(skills.path().join("pdf-forms/SKILL.md"))
            .unwrap()
            .contains("Better steps.")
    );
    assert!(skills.path().join("pdf-forms/scripts/fields.py").exists());
}

#[tokio::test]
async fn removing_a_skill_waits_for_the_owner() {
    let skills = tempfile::tempdir().unwrap();
    install_foreign_skill(skills.path());
    let provider = ScriptedProvider::default();
    provider
        .then_acts([("remove_skill", json!({ "name": "pdf-forms" }))])
        .then(Ok("Removed.".into()));
    let mut owner = agent_learning_in(skills.path(), &provider);

    owner.says("Drop that skill.");
    let approval = owner.is_asked().await;
    assert!(approval.irreversible, "{approval:?}");
    assert!(skills.path().join("pdf-forms").exists());
    owner.allows();
    owner.hears().await;

    assert!(!skills.path().join("pdf-forms").exists());
    assert!(!provider.last_instructions().contains("pdf-forms:"));
}

#[tokio::test]
async fn using_or_removing_a_skill_that_does_not_exist_tells_the_model_which_do() {
    let skills = tempfile::tempdir().unwrap();
    install_foreign_skill(skills.path());
    for action in ["use_skill", "remove_skill"] {
        let provider = ScriptedProvider::default();
        provider.then_acts([(action, json!({ "name": "excel" }))]).then(Ok("Hm.".into()));
        let mut owner = agent_learning_in(skills.path(), &provider);

        owner.says("Go.");
        owner.hears().await;

        assert!(outcome(&provider).contains("pdf-forms"), "{}", outcome(&provider));
        assert!(owner.was_asked_nothing());
    }
}

#[tokio::test]
async fn a_skill_name_cannot_reach_outside_the_skills_folder() {
    let home = tempfile::tempdir().unwrap();
    let skills = home.path().join("skills");
    fs::create_dir_all(home.path().join("precious")).unwrap();
    fs::write(
        home.path().join("precious/SKILL.md"),
        "---\nname: precious\ndescription: d\n---\nkeep",
    )
    .unwrap();
    for (action, input) in [
        (
            "save_skill",
            json!({ "name": "../precious", "description": "d", "instructions": "overwritten" }),
        ),
        ("remove_skill", json!({ "name": "../precious" })),
        ("use_skill", json!({ "name": "../precious" })),
    ] {
        let provider = ScriptedProvider::default();
        provider.then_acts([(action, input)]).then(Ok("Ok.".into()));
        let (mut channel, mut owner) = memory_channel();
        let agent_provider = provider.clone();
        let folder = skills.clone();
        let running = tokio::spawn(async move {
            Agent::new(agent_provider, Store::in_memory())
                .learning_in(folder)
                .converse(&mut channel)
                .await
        });
        owner.says("Go.");
        // Allow whatever is asked: the name alone must keep it inside.
        tokio::select! {
            _ = owner.is_asked() => owner.allows(),
            () = tokio::time::sleep(std::time::Duration::from_millis(300)) => {}
        }
        owner.hears().await;
        running.abort();
        assert!(!outcome(&provider).contains("keep"), "{}", outcome(&provider));
    }

    assert!(fs::read_to_string(home.path().join("precious/SKILL.md")).unwrap().ends_with("keep"));
}

#[tokio::test]
async fn a_folder_that_is_not_a_readable_skill_is_left_out_without_breaking_the_rest() {
    let skills = tempfile::tempdir().unwrap();
    install_foreign_skill(skills.path());
    fs::create_dir_all(skills.path().join("broken")).unwrap();
    fs::write(skills.path().join("broken/SKILL.md"), "no header here").unwrap();
    fs::create_dir_all(skills.path().join("empty")).unwrap();
    let provider = ScriptedProvider::answering(["Hi."]);
    let mut owner = agent_learning_in(skills.path(), &provider);

    owner.says("Hi");
    owner.hears().await;

    let instructions = provider.last_instructions();
    assert!(instructions.contains("pdf-forms:"), "{instructions}");
    assert!(!instructions.contains("broken") && !instructions.contains("empty:"), "{instructions}");
}
