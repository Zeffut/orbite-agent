# Orbite

An open-source personal AI agent that lives on a machine you control, uses the AI you already pay for, and adapts to you over time.

> **Status: early development.** Nothing is installable yet: so far the Agent holds a conversation in a terminal. Version 1.0 is specified in the issue tracker; versions before it are numbered 0.x.

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

# talk to the Agent in a terminal, with any OpenAI-compatible Provider
ORBITE_API_KEY=... ORBITE_MODEL=... ORBITE_BASE_URL=https://api.openai.com/v1 cargo run -- chat
```

## Documentation

- [`GLOSSARY.md`](GLOSSARY.md): the project's vocabulary.
- [`docs/adr/`](docs/adr/): the decisions behind the design, and why.

## License

[AGPL-3.0](LICENSE). Contributions are accepted under a contributor agreement (see ADR 0009).
