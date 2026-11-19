# Orbite

An open-source personal AI agent that lives on a machine you control, uses the AI you already pay for, and adapts to you over time.

> **Status: early development.** Versions before 1.0 are for trying it out, and things will change. What works today: an agent you talk to in Telegram (or a terminal) that acts on its machine behind your approvals, remembers, learns skills, runs routines and sub-agents, reads the web, wakes up by itself, uses MCP servers, writes its own extensions, and has a computer of its own with a browser. [CHANGELOG.md](CHANGELOG.md) says what each version brought.

## Trying it

On a Debian or Ubuntu machine (x86_64 or aarch64), as root:

```sh
curl -fsSL https://github.com/Zeffut/orbite-agent/releases/latest/download/install.sh | sudo sh
```

The installer checks the machine before changing anything, verifies that what it downloads is signed by Orbite's key, gives Orbite an account of its own and a service that restarts by itself, then asks a few questions: which AI to think with, your Telegram bot, your agent's name. Running it again updates Orbite and keeps everything else.

Then:

- [Installing Orbite](docs/install.md): from nothing to an agent that answers you in Telegram, and how to update, move and remove it.
- [Your first day](docs/first-day.md): what your agent can do, Approvals, commands, settings, secrets, MCP servers.
- [Extensions](docs/extensions.md): living with them, and writing one.
- [Providers](docs/providers.md): the AI your agent thinks with, and how to choose a model.

## What 1.0 will be

Already there:

- One command installs Orbite on a Debian or Ubuntu machine: a spare computer, a mini PC, a VPS.
- You talk to your agent in Telegram, in one continuous conversation.
- It runs on API keys from the provider you choose, or on a model on your own machine.
- It has its own computer on your machine, with a screen and a browser.
- It remembers, learns skills, runs routines, and writes its own extensions when you agree.
- It sends no telemetry, and everything it learns is a file you can read.

Still to come before 1.0:

- Running on a subscription you already pay for, through each provider's official path only: a ChatGPT plan works (tried with one account so far); others only where their provider supports it natively.
- Gmail and Google Calendar.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). Outside contributions are taken under a short [contributor agreement](CLA.md), agreed to once on your first pull request.

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

- The guides above, in [`docs/`](docs/).
- [`GLOSSARY.md`](GLOSSARY.md): the project's vocabulary.
- [`docs/adr/`](docs/adr/): the decisions behind the design, and why.
- [`docs/research/`](docs/research/): what was looked into before building.

## License

[AGPL-3.0](LICENSE). Contributions are accepted under a contributor agreement (see ADR 0009).
