# Changes

What changed in each version of Orbite, for the people who run it.

## Not released yet

Everything below is on `main` and tried on fresh machines by the release workflow, but no version carries it yet. Installations of 0.1.0 do not update by themselves: they will once the installer of the next version has been run on them.

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
- What is declared to the model on every turn is about a fifth lighter.
- When the model itself keeps failing or declining, the Agent says so and how to change it.
- A Provider's answer that its model wrote a broken Action is asked for again instead of reported as a failure.

### To know before releasing

- The installer now adds Mozilla's package repository to the machine, for Firefox.
- Releases must carry a `VERSION` file for updates to work; the release workflow writes it and refuses a tag that does not match the program's own version.
- The Computer was tried for real with a model for reading a page; a task with clicks and typing done by a real model has not been shown yet.

## 0.1.0

The first version: the Agent in a terminal and in Telegram, acting on its machine behind Approvals, with memory, skills, routines, sub-agents, web reading, wake-ups, files and pictures, sealed secrets, several Providers, a guided setup and a one-line installer.
