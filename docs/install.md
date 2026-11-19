# Installing Orbite

Orbite installs on a machine you control and keeps running there: a spare computer, a mini PC, a VPS. This page takes you from nothing to an agent that answers you in Telegram.

Orbite is in early development: versions before 1.0 are for trying it out, and things will change.

## What you need

- A machine running **Debian 12 or later, Ubuntu 22.04 or later**, or a system made from them (Linux Mint, Kali, Raspberry Pi OS), with systemd, on an x86_64 or aarch64 processor, where you can run commands as root. The installer checks all of this before changing anything, and warns when the machine has little memory.
- **An AI to think with**: an API key from a provider, a ChatGPT Plus or Pro plan, or a model running on the machine itself with Ollama or LM Studio (see [Providers](providers.md)).
- **A Telegram account.** Telegram is where you talk to your agent.

## 1. Create your Telegram bot

Your agent speaks through a bot that belongs to you.

1. In Telegram, open a chat with **@BotFather**.
2. Send `/newbot`, and answer its two questions: a display name, then a username ending in `bot`.
3. BotFather gives you a **token**, a line of digits and letters with a colon in it. Keep it for step 3. Anyone holding it controls the bot, so give it to nobody.

## 2. Run the installer

On the machine, as root:

```sh
curl -fsSL https://github.com/Zeffut/orbite-agent/releases/latest/download/install.sh | sudo sh
```

The installer:

- checks the machine, and stops with an explanation if it cannot host Orbite;
- downloads Orbite and verifies that it is signed by Orbite's key;
- creates an account called `orbite`, which the agent runs as, and its folder `/var/lib/orbite`;
- installs a service that starts with the machine and comes back by itself after a crash, and a daily check for updates;
- sets up the agent's own computer: a virtual screen (`xvfb`, `xdotool`), fonts, and Firefox from Mozilla's own package repository, which it adds to the machine's package sources. To install Orbite without it, put `ORBITE_WITHOUT_COMPUTER=1` before `sh`. If this part fails, Orbite is installed all the same, and running the installer again later adds it.

**If the installer stops** on something it did not foresee, it prints a link to a new issue, filled in with what stopped and your system's name, processor and memory. Nothing is sent unless you open it and send it; what the terminal showed just before helps, if you paste it in.

## 3. Answer the setup's questions

The installer ends with a short guided setup, in the terminal:

1. **Language**: English or French, for Orbite's own words. Your agent answers in whatever language you write to it.
2. **Provider**: its name from the list shown, then your API key (typed without being shown, and checked at once) or, for a ChatGPT plan, a link to sign in with and the address your browser ends on to paste back, then a model. A model nobody has tried before is tried out on a few real tasks, which takes about a minute, and you are told what to expect of it; see [Choosing a model](providers.md#choosing-a-model).
3. **Telegram**: the bot's token. Setup then shows a **code**: send that code to your bot from your own Telegram account. Whoever sends the code becomes the Owner, the only person the agent will ever listen to.
4. **A name** for your agent.

Your agent then says hello in Telegram. That is it.

If you close the terminal half-way, run `sudo orbite setup` to carry on where you stopped. To redo one part later, add its name: `setup language`, `setup provider`, `setup telegram` or `setup name`. To add a second AI that takes over when the first cannot answer, `sudo orbite providers add` (see [Providers](providers.md#several-providers-in-order)).

## Afterwards

- **Updates.** Once a day Orbite installs the latest release, only if Orbite's key signed it, and goes back to the previous one if the new one does not start; your agent tells you when it happened. `sudo orbite update` updates now; `sudo orbite update off` turns automatic updates off.
- **Looking at the service.** `systemctl status orbite` shows whether it runs, and `journalctl -u orbite` what it said.
- **Moving to another machine.** `sudo orbite export FILE` writes everything of your agent to one file. Keep that file to yourself: it holds the agent's secrets. On the new machine, install without the setup (`curl -fsSL https://github.com/Zeffut/orbite-agent/releases/latest/download/install.sh | sudo ORBITE_SKIP_SETUP=1 sh`), then `sudo orbite restore FILE` and `sudo systemctl restart orbite`.
- **Removing Orbite.** `sudo orbite uninstall` removes the service, the program, the account and the agent's data, after asking. Add `--keep-data` to keep the data in `/var/lib/orbite`. The packages the installer added for the agent's computer (Firefox, `xvfb`, `xdotool`, fonts) and Mozilla's package source are left in place: remove them with `apt` if nothing else uses them.

On the machine, the `orbite` command is run with `sudo`: it then works on the installed Orbite, as the agent's own account. `orbite` with no argument lists what it can do.

Next: [your first day with your agent](first-day.md).
