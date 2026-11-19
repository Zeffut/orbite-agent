# Changes

What changed in each version of Orbite, for the people who run it.

## Unreleased

- When the installer stops on something it did not foresee, or cannot set up the agent's computer, it offers a report: a link to a new issue, filled in with what stopped and the kind of system. Nothing is sent unless you open the link and send it.
- A system too old for Orbite (before Debian 12 or Ubuntu 22.04) is told so at once, before anything is fetched or installed.
- Installed where there is no terminal to ask in (from a script), Orbite leaves setup for later instead of failing at the end.
- The installer takes systems made from Debian or Ubuntu (Linux Mint, Kali, Raspberry Pi OS and the like) as it takes those two, and says so.
- Each release is now installed, before it is published, on Debian 12 and 13 and Ubuntu 22.04, 24.04 and 26.04, on both x86_64 and aarch64, and the installer is checked to refuse cleanly where Orbite cannot run.
- A contributor agreement and a guide to contributing.

## 0.3.13

Released on 2026-11-19. From the same independent read as 0.3.12.

- **Allowing one tool of an MCP server for always no longer allows all its tools.** Since 0.3.3 a server with many tools is one action, and "always" covered the whole server: allowing a tool that comments also allowed the one that deletes.
- **What a recording says is not taken for your orders.** A voice or video message is told to the model as what a recording says, which may be someone else speaking; a message you pass on from somebody else is said to be theirs; nothing in a transcript, a place's name or a contact's can read as a line of Orbite's own.
- An agent whose local transcription was removed from the machine still starts, and says why it cannot hear, instead of not starting at all.
- A voice message sent while the agent is at work is written down too; the agent is seen at work while it writes one down.
- A server or an extension whose actions begin with `computer_` is offered again, and a server called `computer` no longer takes the place of the agent's own.
- Smaller: a model named by a relative path is kept by its full path; a download cut short by a full disk leaves nothing behind; what sub-agents used is counted when Orbite stops.

## 0.3.12

Released on 2026-11-19. **Security: update from 0.3.8 to 0.3.11.**

- **A command written on two lines could run without being asked about.** The change of 0.3.8, which lets a quoted pattern through for programs that only look, read a comment sign followed by a quote differently from the shell: a second line then ran as a command of its own, unasked. Found by an independent read, about two hours after 0.3.8. A command is now read on one line only, and never with a comment sign.
- Also closed: quotes in the middle of a name hiding a secret's path or `/proc` from the check; `/dev/fd` as another way to a program's environment; `ps` showing what programs were started with; `date` setting the clock and `file` writing, through options written together or shortened.

## 0.3.11

Released on 2026-11-19.

- **The agent shows you its screen at the steps that matter.** Any gesture on its computer can also send you the picture, with a word on what it is about: before paying or sending, or when a page asks something of you. Tried for real: a site asks for a verification code, the agent shows the page, asks for the code in the conversation, types it and carries on.

## 0.3.10

Released on 2026-11-19.

- A location, a place or a contact shared in Telegram reaches the agent in words ("what is near here?", "call them tomorrow").
- Video messages in Telegram (the round ones too) are taken in: what is said in them is written down like a voice message, and the agent is told it cannot see the picture.
- `ORBITE_SHOW_WAKE_UPS=1` makes Orbite note, where it logs, what the agent thought at each wake-up: why it stayed silent, or spoke up. A wake-up bringing up something due was seen for real.

## 0.3.9

Released on 2026-11-19.

- `/usage` now counts everything the model answered: what wake-ups, summaries, sub-agents and routines cost was left out, so the figures were below what was really used.

## 0.3.8

Released on 2026-11-19.

- A pattern given in single quotes to a program that only looks (`find . -name '*.txt'`, `grep 'a.*b' file`) no longer asks for an approval: the shell works nothing out of what is between such quotes. Seen in a real try, where a sub-agent, which has nobody to ask, was refused a plain search. Quotes still hide nothing: `find . '-delete'` asks.
- `/settings` shows what writes voice messages down and switches between a provider, nothing, and a local way already set up on the machine.

## 0.3.7

Released on 2026-11-19.

- **Voice messages written down on your own machine.** `orbite transcription` chooses what writes speech down: a provider (as before), whisper.cpp on the machine, a local server speaking OpenAI's protocol (speaches, which runs faster-whisper, is one), any command of yours, or nothing. For whisper.cpp it downloads the model its project publishes and checks it, and tells you before you choose when a model is too small to listen well, takes more of the machine's memory than is wise, will not fit at all, or is slower than speech on this machine, which it measures.

## 0.3.6

Released on 2026-11-19.

- **Voice messages.** What you say aloud in Telegram is written down by the first of your providers that offers it for an API key (OpenAI, Groq) and answered as what you said. Without one, the agent says it cannot hear the message and what would let it. Tried for real with Groq on a spoken recording.

## 0.3.5

Released on 2026-11-19.

- A recurring routine could not be scheduled by a model that writes every field of an action, zero or empty when unused: the unused delay introduced in 0.3.3 was taken for a second way of scheduling. Seen in a real try.

## 0.3.4

Released on 2026-11-19.

- **On a ChatGPT plan the model was never shown the agent's screen.** Pictures that come with a notice, which is how the screen reaches the model, were left out for this provider: the agent worked in its browser blind, and answered vaguely when a page stopped it. It now sees its screen. Tried on a real site with the smallest model of a plan: it says what the page shows and offers another way. This corrects the note of 0.3.3, which put the vague answers down to the model.
- Trying a model out now also shows it a picture the way the screen is shown, so that a provider that drops it is caught.

## 0.3.3

Released on 2026-11-19.

- **An MCP server with many tools no longer costs its whole description on every turn.** A server that says a great deal about its tools (Notion offers 77) is offered to the model as one action: it lists the tools in a line each, says what a tool takes when asked, then calls it. Measured with Notion connected: about 6,500 tokens a turn instead of 57,000.
- A reminder asked for "in three minutes" comes three minutes later to the second: it could come up to a minute early, the delay being rounded down to the minute.
- A site that asks for proof that a person is there: the agent is told, with each picture of its screen, to say exactly what stops it and to offer another way, never to pass it. Tried on a real site: a strong model does so; a small one (GPT-6 Luna) still answers vaguely, so prefer a stronger model for work in the browser.
- Correction to the note of 0.3.0: OpenAI does cache on a ChatGPT plan when requests are large (about a fifth of what was read, over nineteen large requests); it cached nothing on small ones.

## 0.3.2

Released on 2026-11-19.

- From an independent read of 0.3.0's new code:
  - The address a sign-in ended on is never kept or shown to a model: also when no sign-in is under way any more, when it comes with other words, or while the agent is at work (as for a secret sent then).
  - Changing which provider comes first no longer leaves the lighter model of the one before; each provider's key is kept under its own name only; `orbite tryout` uses the right key; a key given in the environment is for an agent with one provider.
  - Light work counts towards its provider's daily limit and stops with it.
  - On a ChatGPT plan: a renewal is carried to its end even when the turn is interrupted; OpenAI's own trouble lets another provider answer; an answer that comes whole, or a refusal, is read; an answer is waited for ten minutes at most.
  - Two changes to the secrets at once no longer lose one; `orbite` run by the agent from a command is asked about every time.
- On a ChatGPT plan, `/settings model` offers the models of your plan as buttons.

## 0.3.1

Released on 2026-11-19.

- The installer does not put an older release over a newer Orbite, unless `ORBITE_ALLOW_OLDER=1` says so.
- `uninstall --keep-data` gives the kept data to root, so that no later account reads it, and installing again gives it back to the agent. An update is written through to the disk before it takes the old program's place.
- The agent's browser is no longer handed what Orbite itself was started with (its `ORBITE_*` settings, where a key may be) nor the address of a desktop session's bus.
- The agent's screen no longer starts over each time nothing is connected to it: a browser that was starting then could be cut off, a click or a picture lost.
- Two runs of one extension at once no longer lose what the other kept: a run waits for the one before.
- The installer: says when the machine's OpenSSL is too old to check the signature, keeps aside a service file you had changed (`orbite.service.replaced`) and points to drop-ins, and checks the fingerprint of Mozilla's key where `gpg` is there.
- A large part of a secret (twelve characters in a row or more) is hidden from the model like the whole.

## 0.3.0

Released on 2026-11-19.

- **Several providers, in order.** `orbite providers add` sets one more up behind those there are; the first answers while it can and the next takes over when it is out of usage, unreachable, or over the daily limit you gave it (`orbite providers limit NAME TOKENS`). `first` and `remove` change the line-up; `/providers` shows and changes it from the conversation.
- `/providers connect chatgpt` signs in again from the conversation when a sign-in ends; removing the plan signs out at OpenAI too. Each turn counts its context against the plan (see 0.3.3 on caching).
- A picture of the agent's screen is no longer taken while the browser shows all black between two paintings.
- **ChatGPT plans.** `orbite setup` offers "ChatGPT plan": sign in with a link, paste back the address your browser ends on, and your agent thinks with the usage included in your Plus or Pro plan, with no API key. It uses OpenAI's sign-in for open-source programs and nothing else. Tried for real with one account: sign-in, models, actions, images and a conversation in Telegram.

## 0.2.1

Released on 2026-11-19.

- The catalogue of models gains Claude Haiku 5.5 and Claude Sonnet 5.5, from real tryouts.
- MCP servers you sign in to: `/mcp connect NAME` gives a link to follow, and the address your browser ends on, sent back, finishes the sign-in. Tried for real with an account, in Telegram.
- A secret a service sends back in base64 is not handed to an Extension either; an MCP server of the earlier protocol that quits when probed is started again.

## 0.2.0

Released on 2026-11-19. Installations of 0.1.0 do not update by themselves: run the installer again on them once, and they keep themselves up to date from then on.

### New

- **Automatic updates.** Once a day Orbite installs the latest release, only if Orbite's key signed it, and goes back to the previous one if the new one does not start. `orbite update off` turns that off.
- **Extensions.** Small JavaScript programs that add Actions, each run in a sandbox that reaches only a store of its own and the hosts it was granted. The Agent writes one when asked, after showing its Owner the script, and can repair it. `/extensions` lists them. A task list comes with Orbite as the first one.
- **Secrets for Extensions.** An Extension names a secret and Orbite fills it in on the way to a granted host; the script never holds it.
- **MCP servers.** Local commands and remote addresses listed in `mcp.json`; their tools become Actions behind the same Approvals. Servers that need signing in through a web page are not supported yet.
- **The Agent's Computer.** A screen of its own with a real browser, that the Agent sees through pictures and acts on with mouse and keyboard. Going to a site asks first; typing a secret asks every time. The installer sets it up (a virtual screen, fonts, and Firefox from Mozilla's package repository, which it adds to the machine); `ORBITE_WITHOUT_COMPUTER=1` leaves it out.
- **Settings.** `/settings` in the Conversation, with buttons in Telegram, and `orbite settings` on the machine, on the same values: permission level, quiet hours, wake-ups, secrets in the Conversation, updates, language, model.
- **Trying a model out.** `orbite tryout MODEL` gives a model a few real tasks and says what to expect of it; setup does it for a model nobody rated, shows what is known beside the others, and asks before choosing one that cannot act.
- **Moving and removing.** `orbite export`, `orbite restore` and `orbite uninstall`.
- **Guides** in `docs/`: installing, the first day, Extensions, Providers.

### Better

- Commands run with `sudo` act on the installed Orbite, as the Agent's own account.
- A message Telegram could not take is kept and sent with the next one, instead of ending the Conversation.
- Pictures are sent as pictures in Telegram.
- Letters with accents are typed as they are on the Computer, also when Orbite runs as a service.
- What is declared to the model on every turn is about a fifth lighter.
- When the model itself keeps failing or declining, the Agent says so and how to change it.
- A Provider's answer that its model wrote a broken Action is asked for again instead of reported as a failure.

### Safer

Two independent reads of the code found defects that are fixed here; none was in 0.1.0's own features except the first two.

- A command with a pattern the shell fills in (`cat ../s*.key`), or reading a process's environment, was run without asking as one that "only looks". It is asked about.
- Writing with the ordinary file Actions into the Agent's Extensions, MCP servers, settings or secrets is asked about at every Permission Level.
- Updates, run as root, no longer follow anything the Agent's own account could have put in its folder.
- An Extension cannot learn a secret from an error message, and a script not yet approved cannot touch what an Extension keeps.
- The Agent's Computer: its screen has a key of its own, so no other account of the machine can look at it or act on it; a Computer in use by a running Orbite is left alone; one left behind by an Orbite that was killed is cleaned up.
- Restoring an export hands links over as links, and changes nothing unless it can restore everything.

What is still open from those reads is listed in the issue tracker.

### To know

- The installer adds Mozilla's package repository to the machine, for Firefox, unless `ORBITE_WITHOUT_COMPUTER=1` is given.
- The Computer was tried for real with Claude Haiku: a search on a site, with clicks and typing, read back from the screen.
- How much the Agent does without asking on its Computer follows the permission level: at `free` it goes to any site without asking.

## 0.1.0

The first version: the Agent in a terminal and in Telegram, acting on its machine behind Approvals, with memory, skills, routines, sub-agents, web reading, wake-ups, files and pictures, sealed secrets, several Providers, a guided setup and a one-line installer.
