# Orbite

An open-source personal AI agent that lives on a machine you control, uses the AI you already pay for, and adapts to you over time.

> **Status: early development.** Versions before 1.0 are for trying it out. The Agent itself works, in a terminal and in Telegram: it acts on its machine behind Approvals, remembers, learns Skills, runs Routines and Sub-agents, reads the web and wakes up by itself. Version 1.0 is specified in the issue tracker; versions before it are numbered 0.x.

## Trying it

On a Debian or Ubuntu machine (x86_64 or aarch64), as root:

```sh
curl -fsSL https://github.com/Zeffut/orbite-agent/releases/latest/download/install.sh | sudo sh
```

The installer checks the machine before changing anything, verifies that what it downloads is signed by Orbite's key, gives Orbite an account of its own and a service that restarts by itself, then asks a few questions: which AI to think with, your Telegram bot, your agent's name. Running it again updates Orbite and keeps everything else.

## What 1.0 will be

- One command installs Orbite on a Debian or Ubuntu machine: a spare computer, a mini PC, a VPS.
- You talk to your agent in Telegram, in one continuous conversation.
- It runs on your own subscriptions (through each provider's official path) or on API keys.
- It has its own computer on your machine, with a screen and a browser.
- It remembers, learns skills, runs routines, and writes its own extensions when you agree.
- It sends no telemetry, and everything it learns is a file you can read.

## Developing

```bash
cargo test                                   # the whole test suite
cargo fmt && cargo clippy --all-targets      # before every commit

cargo run -- setup                           # the guided setup: AI, Telegram bot, the agent's name
cargo run -- run                             # the Conversation in Telegram (add --terminal for here too)
cargo run -- chat                            # the Conversation in this terminal only
cargo run -- secret set NAME                 # a secret the agent uses without seeing it
# everything is kept in ~/.orbite, or in the folder named by ORBITE_HOME;
# `cargo run` with no command lists the settings that override what setup kept
```

## Documentation

- [`GLOSSARY.md`](GLOSSARY.md): the project's vocabulary.
- [`docs/adr/`](docs/adr/): the decisions behind the design, and why.

## License

[AGPL-3.0](LICENSE). Contributions are accepted under a contributor agreement (see ADR 0009).
