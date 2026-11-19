# Contributing to Orbite

Thank you for wanting to. This says how the project works, so that what you send can be merged.

## Before you write code

- **A defect**: open an issue saying what you did, what you expected and what happened, with the version (`orbite version`). A defect that lets something run or be read without the Owner's Approval is a security report: send it privately through GitHub's "Report a vulnerability" on the repository's Security page, not in an issue.
- **A change of behaviour or something new**: open an issue first. Orbite is one Agent, for one Owner, on a machine they control, and says no to much that would fit another product. The spec is [issue #1](https://github.com/Zeffut/orbite-agent/issues/1), the decisions that shape it are in [`docs/adr/`](docs/adr/), and its words are in [`GLOSSARY.md`](GLOSSARY.md): Owner, Agent, Conversation, Action, Approval, Provider and the others mean one thing each, in the code and the docs alike.
- **A Provider or a service**: only through what its publisher documents and allows for programs such as this one. No private endpoints, no credentials taken from another program, no way around a check for a person.

## The agreement

Outside contributions are taken under the [Orbite Contributor Agreement](CLA.md). You keep your rights to your work; you allow the project to distribute it, including under other licences than today's (see [ADR 0009](docs/adr/0009-agpl-with-contributor-agreement.md) for why). You agree once, by posting one sentence on your first pull request; a message there tells you how, and the pull request is held until everyone whose commits are in it has.

## Writing the change

Orbite is written in Rust (2024 edition), in English.

```sh
cargo fmt
cargo clippy --all-targets
cargo test
```

All three must be clean. The tests of the Agent's Computer need `Xvfb`, `xdotool` and a browser, and skip themselves where there is none.

- **A test first, through what an Owner would observe.** Tests drive the Agent through a Channel with a scripted Provider, the command as an Owner would run it, or the installer against a pretend machine; other services are played by stand-ins answering as their documentation says. A test that reaches inside a module, or that passes by construction, is not one.
- **Nothing secret in the repository**, tests included: a value shaped like a key is built from parts.
- **What every turn carries stays light.** `tests/weight.rs` holds what the model is told before the Conversation under a limit; a new Action has to earn its words.
- **Say it in the changelog**, in words for the people who run Orbite, when they would notice the change.
- Commits say what changed for whoever uses or maintains Orbite, and why; one change, one commit.

## What happens next

A maintainer reads the pull request. CI runs the tests, the installer on fresh Debian and Ubuntu machines, and the Computer with two browsers. Releases are cut by the maintainers; installations update themselves from them.
