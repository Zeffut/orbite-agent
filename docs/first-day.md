# Your first day with your agent

You talk to your agent in Telegram, in one continuous conversation. There is nothing to learn before writing to it: ask for what you want, as you would ask a person who works for you. This page says what it can do today and how you stay in charge.

## What it can do

- **Act on its machine.** It runs commands and reads, writes and edits files, in a folder of its own (`/var/lib/orbite/workspace`).
- **Read the web.** It searches and reads pages.
- **Remember.** What it learns about you is kept as plain files in `/var/lib/orbite/memory`, one per subject. You can read, change or delete any of them, and ask it to forget something.
- **Learn skills.** A way of doing something that worked can be kept as a skill (`/var/lib/orbite/skills`), in the Agent Skills format other agents use; skills written for other agents work unchanged.
- **Work in the background.** It can hand a long piece of work to a sub-agent and carry on talking with you.
- **Act on a schedule.** "Every morning at 8, tell me the weather" becomes a routine; "remind me at 5 to call Ada" a reminder.
- **Wake up by itself** now and then to review what is pending, and tell you only when something deserves it.
- **Keep a task list**, and whatever [extensions](extensions.md) add.
- **Receive and send files.** Send it a document or a photo; ask it to send you a file it made.

## Approvals: you stay in charge

Before an action that changes something, your agent asks. In Telegram the request shows exactly what would be done, with buttons: **Allow**, **Always** (allow this kind of thing once and for all) and **Deny**. Writing a message instead of tapping cancels the request and is heard as your answer.

How much it does without asking is the **permission level**:

| Level | What runs without asking |
|---|---|
| careful (at first) | only what looks at things: reading files, searching the web |
| balanced | also changes inside the agent's own folders |
| free | everything, except critical actions |

**Critical actions**, such as anything that could show a secret to the AI provider, are asked about at every level and cannot be allowed once and for all.

## Commands

Everything else is said in plain words, but a few commands are always there, also in Telegram's menu:

| Command | What it does |
|---|---|
| `/stop` (or just `stop`) | stops what the agent is doing |
| `/reset` | starts the conversation afresh; what it remembers about you is kept |
| `/settings` | shows the settings, with buttons to change them |
| `/usage` | what the agent has used, by provider and model |
| `/extensions` | lists the extensions; `/extensions off NAME`, `on NAME`, `remove NAME` |
| `/mcp` | lists the MCP servers and their state |
| `/secrets on` | lets you give a secret in the conversation (off by default; see below) |

## Settings

`/settings` in Telegram and `sudo orbite settings` on the machine show and change the same settings; a change in one is seen in the other.

| Setting | Values |
|---|---|
| `permission` | `careful`, `balanced`, `free` |
| `quiet` | `off`, or the hours during which the agent keeps what is not urgent for later, as `22:00-07:00` |
| `wakeups` | `off`, or minutes between two wake-ups |
| `secrets` | `off` or `on`: whether a secret may be given in the conversation |
| `updates` | `on` or `off`: automatic updates |
| `language` | `en` or `fr` |
| `model` | the model, by its name at your provider |

Changing the language or the model makes Orbite restart, which takes a few seconds.

## Secrets

A key or a password your agent needs is entered **on the machine**, never in the conversation, where it would travel through Telegram and reach the AI provider:

```sh
sudo orbite secret set GITHUB_TOKEN
```

The agent then uses it by name, in commands and extensions, and never sees its value: whatever comes back from an action has the value hidden before the model reads it. If you paste something that looks like a secret in the conversation, the agent refuses it and removes your message. `/secrets on` lifts that, for when you accept the risk.

## MCP servers

Tools from MCP servers become actions of your agent. List the servers in `/var/lib/orbite/mcp.json`, in the form other MCP clients use, then restart Orbite:

```json
{ "mcpServers": {
    "memory": { "command": "npx", "args": ["-y", "@modelcontextprotocol/server-memory"] },
    "github": { "url": "https://api.githubcopilot.com/mcp/", "headers": { "Authorization": "Bearer ${GITHUB_TOKEN}" } }
} }
```

`${NAME}` stands for a secret entered as above, so no key is written in that file. A tool that declares it only reads runs freely; any other asks you first. `/mcp` shows each server's state. Servers that need you to sign in through a web page are not supported yet.

## When something goes wrong

- **It does not answer.** `systemctl status orbite` on the machine, then `journalctl -u orbite -n 50`.
- **"My AI provider is too busy"** is the provider's limit, not Orbite's: wait, or choose a lighter model.
- **It keeps failing or declining.** After a few times in a row the agent says the model seems to be the cause. `sudo orbite tryout NAME` on the machine shows what to expect of another model, and `/settings model NAME` changes it.
