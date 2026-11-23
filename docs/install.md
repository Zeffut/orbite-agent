# Installing Orbite

Orbite installs on a machine you control and keeps running there: a spare computer, a mini PC, a VPS. This page takes you from nothing to an agent that answers you, in a browser or in Telegram.

Orbite is in early development: versions before 1.0 are for trying it out, and things will change.

## What you need

- A machine running **Linux with systemd**, as freshly installed: Debian 12 or later, Ubuntu 22.04 or later, Fedora, Arch, openSUSE, AlmaLinux and the systems made from them (Linux Mint, Kali, Raspberry Pi OS, Manjaro). Nothing has to be installed beforehand: what the installer needs and the system lacks, it installs itself. On a system whose packages it does not know, Orbite is installed all the same, and you add the agent's computer by hand, on an x86_64 or aarch64 processor, where you can run commands as root. The installer checks all of this before changing anything, and warns when the machine has little memory.
- **An AI to think with**: an API key from a provider, a ChatGPT Plus or Pro plan, or a model running on the machine itself with Ollama or LM Studio (see [Providers](providers.md)).
- **Somewhere to talk to it**: a browser, or a Telegram account, or both. For the browser on your phone, the machine and the phone on [Tailscale](https://tailscale.com); see [the web interface](interface.md).

## 1. If you want it in Telegram, create your bot

Skip this to talk to your agent in a browser only. In Telegram, your agent speaks through a bot that belongs to you.

1. In Telegram, open a chat with **@BotFather**.
2. Send `/newbot`, and answer its two questions: a display name, then a username ending in `bot`.
3. BotFather gives you a **token**, a line of digits and letters with a colon in it. Keep it for step 3. Anyone holding it controls the bot, so give it to nobody.

## 2. Run the installer

On the machine, as root:

```sh
curl -fsSL https://github.com/Zeffut/orbite-agent/releases/latest/download/install.sh | sudo sh
```

Where there is no `curl` (a fresh Debian has `wget` instead), the same with `wget`:

```sh
wget -qO- https://github.com/Zeffut/orbite-agent/releases/latest/download/install.sh | sudo sh
```

And from a root shell, where there is no `sudo`, leave `sudo` out.

The installer:

- checks the machine, and stops with an explanation if it cannot host Orbite;
- downloads Orbite and verifies that it is signed by Orbite's key;
- creates an account called `orbite`, which the agent runs as, and its folder `/var/lib/orbite`;
- lets the agent's account, which is not root, do a named few things as root (update Orbite, restart its service, restart or stop the machine, turn the web interface on or off) and nothing else; whether it does them without asking you is the permission level's to say;
- installs a service that starts with the machine and comes back by itself after a crash, and a daily check for updates;
- where the machine has a desktop, has a helper start with your graphical session (yours only: the account that ran the installer), which lends that desktop to your agent as its computer; see [the agent's computer](first-day.md#the-agents-computer);
- sets up the agent's own computer: a virtual screen (`xvfb`, `xdotool`), fonts, and Firefox from Mozilla's own package repository, which it adds to the machine's package sources. To install Orbite without it, put `ORBITE_WITHOUT_COMPUTER=1` before `sh`. If this part fails, Orbite is installed all the same, and running the installer again later adds it.

**If the installer stops** on something it did not foresee, it prints a link to a new issue, filled in with what stopped and your system's name, processor and memory. Nothing is sent unless you open it and send it; what the terminal showed just before helps, if you paste it in.

## 3. Answer the setup's questions

The installer ends with a short guided setup, on a screen of its own in the terminal: answers are chosen with the arrow keys and Enter. (In a terminal that cannot show it, or with `ORBITE_PLAIN=1`, the same questions are asked as plain lines.)

1. **Language**: English or French, for Orbite's own words. Your agent answers in whatever language you write to it.
2. **Provider**: chosen from the list, then your API key (pasted without being shown, and checked at once) or, for a ChatGPT plan, a link to sign in with and the address your browser ends on to paste back. The model is not a question: Orbite takes the one that provider is known to serve an agent well with, and a lighter one for summaries where there is one. You change it later by asking your agent; see [Choosing a model](providers.md#choosing-a-model). Only a provider with no usual model (a server of your own, Ollama or LM Studio with several models) has you choose.
3. **How your devices reach it**: your agent is talked to in a browser, on a page Orbite serves itself. Setup asks how your devices will reach it: **through Tailscale**, set up by itself when Tailscale is already on the machine (one entry added to your Tailscale settings), or **at this server's public address**, for a server on the Internet, where Orbite gets itself a certificate for the address (ports 80 and 443 must be open to the Internet).
4. **A name** for your agent.
5. **Its look**: a shape and a colour, shown as you point at each. Your agent then appears as itself, in the terminal and on its page.

Setup then shows a **link**, with a code to point your phone's camera at when the link is one a phone can follow: open it within ten minutes and you are in. That is it.

If you close the terminal half-way, run `sudo orbite setup` to carry on where you stopped. **Telegram too, if you want it.** Setup does not ask about it. To talk to your agent in Telegram as well, run `sudo orbite setup telegram`: it asks for the token of a bot of your own (made with @BotFather), then shows a code to send to that bot from your own account. Whoever sends the code becomes the Owner, the only person the agent will ever listen to.

To redo one part later, add its name: `setup language`, `setup provider`, `setup telegram`, `setup interface`, `setup name` or `setup look`. To add a second AI that takes over when the first cannot answer, `sudo orbite providers add` (see [Providers](providers.md#several-providers-in-order)).

## Afterwards

- **Updates.** Once a day Orbite installs the latest release, only if Orbite's key signed it, and goes back to the previous one if the new one does not start; your agent tells you when it happened. `sudo orbite update` updates now; `sudo orbite update off` turns automatic updates off.
- **Looking at the service.** `systemctl status orbite` shows whether it runs, and `journalctl -u orbite` what it said.
- **Moving to another machine.** `sudo orbite export FILE` writes everything of your agent to one file. Keep that file to yourself: it holds the agent's secrets. On the new machine, install without the setup (`curl -fsSL https://github.com/Zeffut/orbite-agent/releases/latest/download/install.sh | sudo ORBITE_SKIP_SETUP=1 sh`), then `sudo orbite restore FILE` and `sudo systemctl restart orbite`.
- **Removing Orbite.** `sudo orbite uninstall` removes the service, the program, the account and the agent's data, after asking. Add `--keep-data` to keep the data in `/var/lib/orbite`. What the installer brought for the agent's computer and the machine did not have before (Firefox, Xvfb, `xdotool`, fonts, Mozilla's package source) is removed too, so that the machine is left as it was; add `--keep-computer` to keep it. What the machine already had is never removed, nor a package that something you installed since needs, nor the everyday tools the installer had to install (`curl`, `sudo`): the command names what it left.

On the machine, the `orbite` command is run with `sudo`: it then works on the installed Orbite, as the agent's own account. `orbite` with no argument lists what it can do.

Next: [your first day with your agent](first-day.md).
