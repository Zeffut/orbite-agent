# Your first day with your agent

You talk to your agent in a browser ([the web interface](interface.md)) or in Telegram, in one continuous conversation. There is nothing to learn before writing to it: ask for what you want, as you would ask a person who works for you. This page says what it can do today and how you stay in charge.

## What it can do

- **Act on its machine.** It runs commands and reads, writes and edits files, in a folder of its own (`/var/lib/orbite/workspace`).
- **Read the web.** It searches and reads pages.
- **Remember.** What it learns about you is kept as plain files in `/var/lib/orbite/memory`, one per subject. You can read, change or delete any of them, and ask it to forget something.
- **Learn skills.** A way of doing something that worked can be kept as a skill (`/var/lib/orbite/skills`), in the Agent Skills format other agents use; skills written for other agents work unchanged.
- **Work in the background.** It can hand a long piece of work to a sub-agent and carry on talking with you.
- **Act on a schedule.** "Every morning at 8, tell me the weather" becomes a routine; "remind me at 5 to call Ada" a reminder. A routine's run, like a sub-agent, can ask you nothing: it does what your agent would do without asking at the permission level you chose, and reports the rest as left to do.
- **Wake up by itself** now and then to review what is pending, and tell you only when something deserves it.
- **Use a computer**: this machine's own desktop where it has one, or a screen of its own that nobody else looks at. It sees it through pictures and acts with mouse and keyboard. It is for sites that need signing in or a person's gestures. See below.
- **Keep a task list**, and whatever [extensions](extensions.md) add.
- **Receive and send files.** Send it a document or a photo; ask it to send you a file it made.

## Speaking instead of writing

Send your agent a voice message in Telegram and it answers what you said. Something has to write your words down first, and you choose what, on the machine, with `sudo orbite transcription`:

| Choice | What writes speech down | What it takes |
|---|---|---|
| `provider` (the default) | the first of your providers that offers it for an API key: OpenAI and Groq do | nothing to set up; the recording goes to that provider |
| `whisper.cpp NAME` | [whisper.cpp](https://github.com/ggml-org/whisper.cpp) on your own machine: nothing leaves it | `whisper-cli` and `ffmpeg` on the machine (Ubuntu 26.04: `sudo apt install whisper.cpp ffmpeg`; elsewhere, built from its source), and a model, which Orbite downloads and checks |
| `server ADDRESS MODEL` | a server of yours that speaks OpenAI's transcription protocol, such as [speaches](https://github.com/speaches-ai/speaches), which runs faster-whisper | that server, running |
| `command 'PROGRAM ... {file}'` | any program: it is given the recording and prints what was said | whatever you like: sherpa-onnx, Vosk, a script of yours |
| `off` | nothing: your agent tells you it cannot hear the message | |

`sudo orbite transcription` alone shows the current choice, what this machine has (memory, processor cores) and every model of whisper.cpp with what to expect of it here:

- **too small to listen well**: `tiny` and `base` are quick and make many mistakes, more so outside English; `small` is the smallest that listens well;
- **too heavy for the machine**: a model that takes more than half of the machine's memory is flagged, since the agent's browser needs room too, and one that does not fit at all is refused unless you add `--anyway`;
- **too slow**: few cores under a large model are flagged, and when you choose a model Orbite times it on ten seconds of sound and tells you if your machine listens slower than people speak.

In Telegram, `/settings` shows the choice under **Voice messages** and switches between a provider, nothing, and a local way you already set up on the machine.

`sudo orbite transcription language fr` names the language rather than leaving it to be found, which helps short messages. A ChatGPT plan and Anthropic do not write speech down: with only those, choose a local way, or add Groq or OpenAI behind them (`sudo orbite providers add`).

## Approvals: you stay in charge

Before an action that changes something, your agent asks. In Telegram the request shows exactly what would be done, with buttons: **Allow**, **Always** (allow this kind of thing once and for all) and **Deny**. Writing a message instead of tapping cancels the request and is heard as your answer.

How much it does without asking is the **permission level**:

| Level | What runs without asking |
|---|---|
| careful (at first) | only what looks at things: reading files, searching the web |
| balanced | also changes inside the agent's own folders |
| free | everything: the agent keeps its own machine, its updates and restarts included |

**Critical actions** are asked about at the careful and balanced levels, each time, and cannot be allowed once and for all: taking administrator rights, restarting or stopping the machine, anything that could show a secret to the AI provider, creating an extension, and writing straight into Orbite's own files (its extensions, `mcp.json`, its settings, its secrets).

At the **free** level nothing is asked, critical or not. Choose it knowing what that means: your agent may update itself, restart its machine, change its own settings and read any file its account can read, on its own judgement; and what it reads on the web or in a file could talk it into something you would have refused. Raising the level is always yours to allow; your agent cannot do it by itself.

A command runs without asking only when it plainly does nothing but look: `ls`, `cat notes.txt`, `grep`. One the shell would complete by itself (`cat *.txt`, anything with `~`) is asked about, because what it ends up reading cannot be known beforehand.

## The agent's computer

Reading the web is done by fetching pages, which is fast. For what that cannot do, your agent has a computer, of one of two kinds:

- **This computer**: the desktop you see on the machine's screen, with its programs and the accounts you are signed in to. You watch what your agent does and can take the keyboard back at any time. It is used whenever the machine has a desktop open under the account that installed Orbite.
- **A computer of its own**: a virtual screen and a browser running under its account, which nobody else looks at and where nothing of yours is. Its sign-ins are kept from one day to the next in `/var/lib/orbite/computer`. It is what a machine without a desktop, such as a server, has.

Which of the two is chosen once, when Orbite is installed, and stays: neither a setting nor your agent changes it. Installing Orbite again is how you choose again. On this computer your agent acts with your accounts, and a mouse and a keyboard can do there whatever you can. So it asks before taking them (**Use this computer**), for ten minutes at a time, and that request cannot be allowed once and for all; only the free permission level does away with it, as with every other request. Give it a machine, or a session, where that suits you, and keep it to a computer of its own otherwise.

This computer is lent by a small helper, `orbite desk`, which starts with your graphical session. It works on sessions that run on Xorg. A Wayland session, the default on recent Ubuntu and Fedora, cannot be lent yet, and your agent keeps a computer of its own there; choosing "on Xorg" at the login screen is the way around for now.

- **Going to a site asks you first**, with the site and what for. **Always** then covers that site. What it does there afterwards (clicks, typing) does not ask.
- **Typing a secret asks every time.** The agent names the secret and Orbite types its value; the model does not see it, unless the field shows what is typed, in which case the picture of the screen does. The request says so.
- Where a site asks for a verification code, or for proof of being a person, the agent asks you. Orbite does not disguise its browser and does not solve such challenges.
- The model must be one that sees images.

The installer sets the computer up: a virtual screen, Firefox from Mozilla's own packages, and fonts. `sudo orbite computer check` says whether it works on your machine. An Orbite installed with `ORBITE_WITHOUT_COMPUTER=1` has none; running the installer again without it adds one.

When a page asks something of you (a verification code, a confirmation before paying), your agent is told to send you a picture of its screen with its question, so that you answer knowing what it sees. How reliably it does depends on the model you chose; you can ask it for its screen at any time.

## Commands

None of these is needed: ask your agent in plain words ("add the Notion connector", "turn it off for now", "I have to give you a password", "what did I use today?") and it carries the command out for you. They are shortcuts for when you know what you want.

Everything else is said in plain words, but a few commands are always there, also in Telegram's menu:

| Command | What it does |
|---|---|
| `/stop` (or just `stop`) | stops what the agent is doing |
| `/reset` | starts the conversation afresh; what it remembers about you is kept |
| `/settings` | shows the settings, with buttons to change them |
| `/usage` | what the agent has used, by provider and model |
| `/connections` | every connection in one place: the AIs and the connectors, how each stands, and buttons to sign in, sign out or remove; `/connect NAME ADDRESS` adds a connector |
| `/providers` | the AIs it thinks with, in order, with their limits; `/providers first NAME`, `limit NAME 200000`, `remove NAME`, `connect chatgpt` |
| `/extensions` | lists the extensions; `/extensions off NAME`, `on NAME`, `remove NAME` |
| `/mcp` | lists the MCP servers and their state |
| `/secret NAME` | gives your agent a password or a key safely: sealed on the machine, never shown to the AI (see below) |
| `/secrets on` | lets pasted secrets through to the AI provider (off by default; see below) |

## Settings

`/settings` in Telegram and `sudo orbite settings` on the machine show and change the same settings; a change in one is seen in the other. Among them: your agent's name (`/settings name NEW NAME`), its model, and a quicker, cheaper model of the same provider for light work such as summaries and wake-ups (`/settings light NAME`, or `off`).

| Setting | Values |
|---|---|
| `permission` | `careful`, `balanced`, `free` |
| `quiet` | `off`, or the hours during which the agent keeps what is not urgent for later, as `22:00-07:00` |
| `wakeups` | `off`, or minutes between two wake-ups |
| `secrets` | `off` or `on`: whether a secret may be given in the conversation |
| `updates` | `on` or `off`: automatic updates |
| `language` | `en` or `fr` |
| `model` | the model, by its name at your provider |
| `voice` | what writes voice messages down: `provider`, `off`, or a local way set up on the machine |

Changing the language or the model makes Orbite restart, which takes a few seconds.

## Secrets

A key or a password your agent needs is given with `/secret`, in the conversation:

```
/secret SHOP_PASSWORD
```

Orbite asks for its value, which you send in your next message. It is sealed on the machine, your message is removed from the conversation, and the AI provider never sees it: your agent is only told the name, and uses the secret by that name, in commands, on its computer and in extensions. Whatever comes back from an action has the value hidden before the model reads it.

What you send does travel through Telegram on its way, as every message to a bot does. To avoid even that, enter it on the machine instead:

```sh
sudo orbite secret set SHOP_PASSWORD
```

If you simply write a password in a message ("my password is …"), your agent offers to put it away: you agree with a button, and it is sealed under a name, the message is removed, and it is not shown to the model again. It was seen by the AI provider that once, so prefer saying first that you have a secret to give.

For nothing of a password ever to reach the AI provider, set `/settings secrets strict`: a message that speaks of a password or a code is then held back, Orbite asks whether it holds one, and takes its value out before the model reads the rest. It costs a tap on messages that merely mention passwords, which is why it is off unless you choose it.

If you paste something that looks like a secret in an ordinary message, the agent refuses it, removes your message and points you to `/secret`. `/secrets on` lifts that refusal, and what you paste is then shown to the AI provider: it is there for those who accept that, and `/secret` is the better way.

## MCP servers

Tools from MCP servers (connectors) become actions of your agent. The simple way: in the conversation, `/connect NAME ADDRESS`, for instance `/connect notion https://mcp.notion.com/mcp`. Orbite starts again and comes back with a link to sign in, or with the connector connected; `/connections` shows them all, with buttons to sign out of one, turn it off for a while (it is kept, with its sign-in, and left alone) or remove it. A sign-in is renewed by itself for as long as the service allows; when one ends or is withdrawn, your agent tells you, once, with the command to sign in again. For a server that is a program on the machine, or that takes a key in its headers, list it in `/var/lib/orbite/mcp.json`, in the form other MCP clients use, then restart Orbite:

```json
{ "mcpServers": {
    "memory": { "command": "npx", "args": ["-y", "@modelcontextprotocol/server-memory"] },
    "github": { "url": "https://api.githubcopilot.com/mcp/", "headers": { "Authorization": "Bearer ${GITHUB_TOKEN}" } }
} }
```

`${NAME}` stands for a secret entered as above, so no key is written in that file. A tool that declares it only reads runs freely; any other asks you first. Add only servers you trust: a server is taken at its word on which of its tools only read. `/mcp` shows each server's state.

A server that offers many tools (Notion has 77) is shown to the model as one action, named after the server: the model asks it what a tool takes, then uses it. That keeps every turn light; nothing changes for you.

**Servers you sign in to.** For a server that wants you to sign in (Notion, Linear, Sentry and others), list only its address, then send `/mcp connect NAME`. Your agent answers with a link: open it and sign in. Your browser then stops on a page that does not load, at an address starting with `http://127.0.0.1`: that is expected, since that address is your agent's machine and not the device you are on. Copy the whole address and send it to your agent, which finishes the sign-in, removes your message, and keeps what it obtained sealed with its secrets. A server that takes no program it does not already know (GitHub's or Google's, for instance) is used with a key in its headers, or with a client you register with that service yourself, ending at `http://127.0.0.1:8976/callback`, and name in `mcp.json`: `"client": { "id": "…", "secret": "${NAME}" }`.

## When something goes wrong

- **It does not answer.** `systemctl status orbite` on the machine, then `journalctl -u orbite -n 50`.
- **"My AI provider is too busy"** is the provider's limit, not Orbite's: wait, choose a lighter model, or give your agent a second provider to fall back on (`sudo orbite providers add`).
- **"The sign-in to ChatGPT ended."** Send `/providers connect chatgpt`, follow the link, and send back the address your browser ends on.
- **It keeps failing or declining.** After a few times in a row the agent says the model seems to be the cause. `sudo orbite tryout NAME` on the machine shows what to expect of another model, and `/settings model NAME` changes it.
