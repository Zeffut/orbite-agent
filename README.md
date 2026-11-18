# Orbite

An open-source personal AI agent that lives on a machine you control, uses the AI you already pay for, and adapts to you over time.

> **Status: early development.** Nothing is installable yet. The Agent itself works, in a terminal and in Telegram: it acts on its machine behind Approvals, remembers, learns Skills, runs Routines and Sub-agents, reads the web and wakes up by itself. Version 1.0 is specified in the issue tracker; versions before it are numbered 0.x.

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

# talk to the Agent in a terminal; `cargo run -- providers` lists the Providers known by name
ORBITE_PROVIDER=openai ORBITE_API_KEY=... ORBITE_MODEL=... cargo run -- chat
# the same Conversation in Telegram, and in this terminal as well
ORBITE_TELEGRAM_TOKEN=... ORBITE_TELEGRAM_OWNER=... cargo run -- run --terminal
# its data lives in ~/.orbite, or in the folder named by ORBITE_HOME
```

## Documentation

- [`GLOSSARY.md`](GLOSSARY.md): the project's vocabulary.
- [`docs/adr/`](docs/adr/): the decisions behind the design, and why.

## License

[AGPL-3.0](LICENSE). Contributions are accepted under a contributor agreement (see ADR 0009).
