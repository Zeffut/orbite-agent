# Changes

What changed in each version of Orbite, for the people who run it.

## Unreleased

- Two runs of one extension at once no longer lose what the other kept: a run waits for the one before.
- A large part of a secret (twelve characters in a row or more) is hidden from the model like the whole.

## 0.3.0

Released on 2026-11-19.

- **Several providers, in order.** `orbite providers add` sets one more up behind those there are; the first answers while it can and the next takes over when it is out of usage, unreachable, or over the daily limit you gave it (`orbite providers limit NAME TOKENS`). `first` and `remove` change the line-up; `/providers` shows and changes it from the conversation.
- `/providers connect chatgpt` signs in again from the conversation when a sign-in ends; removing the plan signs out at OpenAI too. OpenAI caches nothing on this path for now, so each turn counts its whole context against the plan.
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
