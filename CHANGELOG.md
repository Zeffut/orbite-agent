# Changes

What changed in each version of Orbite, for the people who run it.

## 0.5.15

Released on 2026-11-21. More independent checks of the secrets code, this time before release; update from any 0.5 version.

- **Orbite asks you one thing at a time.** When your agent asked for a secret and for the key of a new AI together, what you sent for the first could be stored as the second. Whatever would open a second question now waits for the first to be settled.
- **A secret taken out of your message is never filed under another name.** At the strict setting, a value your agent failed to name could later be kept under the next name it asked for. It is now dropped, and you are told, unless it is named in the turn that follows.
- **Your agent cannot have a secret replaced by your next message.** Asked twice for the same secret, or for one already kept, Orbite used to say that what you sent next would replace it. A repeat is now ignored, and replacing one your agent asks about takes a tap on a button first.
- On Telegram, a button left under an earlier question, or tapped twice, no longer answers the next one.
- A key written while Orbite waits for your answer is removed from the chat at once, and resetting the conversation removes the answers it gives up.
- After a restart, Orbite says when a message it was holding back, or something written or asked just before, was lost.

## 0.5.14

Released on 2026-11-21. A fourth independent check of the secrets code; update from any 0.5 version.

- **A secret holding a backslash, a quote or a new line was kept but not cleared from the conversation**, so it still went to the provider. It is cleared like any other now, and Orbite tells you if clearing ever fails.
- **At the strict setting, a second message written before you settled the first is no longer lost or sent in its place.** Orbite says it takes it up afterwards, and does: each message held back gets its own question.
- On Telegram, what you wrote while your agent was working is no longer taken for the answer to a question asked afterwards.
- A secret your agent asks you for while Orbite already awaits something from you waits its turn instead of replacing the question.
- Resetting the conversation gives up what was awaited and what your agent was about to ask you, even while it works. Before, a secret it had asked for was still asked after the reset.

## 0.5.13

Released on 2026-11-21. A third independent check of the secrets code; update from any 0.5 version.

- **With Anthropic, a secret your agent had put away was still sent back to the provider on every turn**, inside what Claude needs returned as it wrote it. It is now cleared there too, and from every older entry of the conversation, which a search could still read.
- **Only what you write after being asked answers the question.** Something you wrote while your agent was working, before Orbite asked you for a secret, could be taken for the secret, or cancel the question so that your real answer went to the model. In the terminal, a line typed ahead could do the same.
- What Orbite awaits from you is taken up before anything else when it starts again, written down before it is asked, and given up when you reset the conversation.
- What your agent may keep as a secret is looked for in your last few messages only.

## 0.5.12

Released on 2026-11-21. A second independent review found the fixes of 0.5.11 incomplete; update from any 0.5 version.

- **Keeping what you wrote as a secret is now yours to agree to.** When your agent picks a password out of your message, you are asked with a button, shown the name it chose and never the value again; at the `free` level it is done without asking, as everything is. Before, a model talked into it could still file an ordinary word of yours as a "secret" and have it wiped. This costs one tap when you write a password in passing; saying first that you have a secret to give still costs none.
- What is cleared from the conversation is only what was said, answered or given to an action, wherever the model still sees it: never the name of an action, which 0.5.11 could rewrite.
- A secret already kept is never replaced without your say, whichever way the new one comes; when Orbite asks you for one that exists, it says that your answer replaces it.
- What Orbite awaits from you is written down at once and said again after a restart ("I am still waiting for…"), including the value of a message held back at the strict setting. After adding an AI, your next message is no longer taken for its key.
- The message removed is always the one that held the secret.

## 0.5.11

Released on 2026-11-21. Fixes found by an independent review of versions 0.5.0 to 0.5.10; update from any of them.

- **A model talked into it could wipe words from your conversation.** When your agent put away "a password you wrote", any piece of text passed for one: a single letter would have been removed from every recent message, breaking the conversation until it was reset, and your Telegram messages holding it deleted. A secret is now only what you wrote as a whole word, in the message being answered, at least four characters long; it is cleared field by field, never inside other words; and replacing a secret already kept asks you first.
- **A secret you were asked for could reach the AI provider.** If your agent asked for another thing right after asking you for a secret, or if Orbite restarted meanwhile, your reply was taken for an ordinary message. Only what you write answers what Orbite asked you, and what it is waiting for survives a restart.
- A command your agent gives is carried out exactly as it was checked, and one Orbite does not know is dropped, never taken for something you said.
- At the strict setting, a password written while your agent is working is held back as well.
- In the terminal, what was typed before an approval was shown no longer answers it.
- A secret that begins with a slash is kept, not mistaken for a command; the message removed is the one that held it, not the latest.
- Sub-agents and routines under way are held to your permission level as it is now, not as it was when they started.
- Removing Orbite on Debian and Ubuntu only removes the packages its own removals leave unneeded.

## 0.5.10

Released on 2026-11-21.

- A change to the AIs your agent thinks with is confirmed in one line, in your language, without the whole list after it.

## 0.5.9

Released on 2026-11-21.

- **Add an AI by asking.** "Add Mistral as a backup, with its large model": your agent adds it once you agree, Orbite asks for its API key in the conversation, seals it, removes your message, and never shows it to a model. Changing which AI is asked first, setting a daily limit or removing one is asked the same way. Until now, adding one took the terminal.

## 0.5.8

Released on 2026-11-21.

- The agent's browser no longer offers to save each password it types: the question covered the page it was working on. Passwords are Orbite's to keep, sealed, not the browser's. This applies to a browser profile from its first start.

## 0.5.7

Released on 2026-11-21.

- **The lists no longer show commands.** Connections, settings and extensions say what to ask ("to add a connector, just ask me", "another pace: tell me, such as a wake-up every 90 minutes") and keep their buttons. Your agent can also start the sign-in to a ChatGPT plan and turn an extension off, on or remove it when you ask.

## 0.5.6

Released on 2026-11-21.

- **Neither your agent nor Orbite sends you to a command for a secret any more.** Your agent is told never to ask you to type a command or to go to the machine: it does the thing itself, and a new password replaces the old one under the name it already had. Orbite's own messages say what to say ("just tell me you have a secret to give me", "say cancel to give up") instead of naming commands.

## 0.5.5

Released on 2026-11-21.

- A setting your agent changes for you is confirmed in one line, without the whole menu after it, and at the strict setting your agent adds nothing once a secret is put away.

## 0.5.4

Released on 2026-11-21.

- "Be strict with my secrets", said to your agent, turns the strict setting on, once you agree. Making it less strict stays yours alone to write.

## 0.5.3

Released on 2026-11-21.

- **A strict setting for secrets, for when nothing of them may reach the AI provider.** `/settings secrets strict` (or "be strict with my secrets"): a message that speaks of a password or a code is held back, and Orbite asks whether it holds one. If it does, you send the value alone, Orbite takes it out of the message, and the model reads "my password is [secret]", names it, and never sees it. If it does not, the message goes on as it was. Left off by default: with a local AI there is no provider to keep it from.
- In French, Orbite's own messages now say "tu", as your agent does, in the conversation and in setup.

## 0.5.2

Released on 2026-11-21.

- **Fewer words when a secret is put away or a command carried out for you.** Orbite confirms in one short line ("SHOP_PASSWORD is kept, and your message removed."), and your agent no longer adds a paragraph of its own saying the same.

## 0.5.1

Released on 2026-11-21.

- **A password you write in passing is put away by your agent itself.** "My password is …" in an ordinary message: your agent names it, Orbite seals it on the machine, removes the message it was written in from the conversation, and no longer shows it to the model from then on. Written in plain words, it was seen by the AI provider that once, which Orbite says; telling your agent first that you have a secret to give avoids even that. Only messages received since Orbite last started can be removed from Telegram.
- The approval asked when your agent carries out a command for you says in plain words what it does ("Add the connector notion"), and when several are asked at once all are done, the one that restarts Orbite last.

## 0.5.0

Released on 2026-11-21.

- **Ask in plain words.** "Add the Notion connector", "sign me in to it", "turn it off for now", "I have to give you my shop password", "no wake-ups at night": your agent now does these itself, by carrying out Orbite's own command for you; Orbite answers with the link to follow, the question to answer or the confirmation. The commands remain as shortcuts, none is needed any more. Adding, turning off or removing a connector and changing a setting are asked about first below the `free` level. Three things stay yours alone to write, whatever the agent is told: loosening its permission level, letting pasted secrets through to the AI provider, and resetting the conversation.
- **A password or a key can be given in the conversation, safely.** `/secret NAME`, then its value in your next message (or both at once): Orbite seals it on the machine, removes your message from the conversation, and tells the model the name only, never the value. Your agent then uses it by its name, on its computer, in commands and in extensions. Until now a secret had to be typed on the machine, or let through to the AI provider with `/secrets on`. The value still travels through Telegram on its way; `orbite secret set` on the machine remains for those who want to avoid that.
- A voice message is written down by the provider given as a fallback in the environment too, when it offers that.

## 0.4.9

Released on 2026-11-21.

- **A command written while your agent works is carried out as a command.** Written instead of answering an approval, or while it was thinking, `/connections`, `/usage` and the others were handed to the model as something you had said, which answered in its own words. They now wait for the end of what it is doing, then are carried out.
- `orbite chat`, the conversation in a terminal, hears `stop` while a command runs, as Telegram always did; what else is typed meanwhile waits for its turn.
- `orbite connections` speaks one language throughout, yours, and shows the connectors turned off.

## 0.4.8

Released on 2026-11-20.

- **Routines and sub-agents can act, within the permission level you chose.** Until now a routine's run or a sub-agent could only look at things, whatever the level: anything that would have asked you was refused, since they cannot ask. They now do what your agent itself would do without asking: everything at the `free` level, what stays in its own folder at `balanced`, what you always allowed at any level, and they report the rest as left to do. At `careful`, nothing changes.
- **The agent's computer works wherever Orbite keeps its folder.** Since 0.4.4, with a folder far down the disk (a path of some fifty letters or more) the browser showed nothing: its sockets had no room for their address. Found by using it for real; an install in the usual place was not affected.

## 0.4.7

Released on 2026-11-20.

- **Your agent's name and its model for light work are changed from Telegram**, as every other setting: `/settings name NEW NAME`, and `/settings light NAME` (or `off`) for a quicker, cheaper model that writes summaries and does wake-ups. Before, both took the machine.

## 0.4.6

Released on 2026-11-20.

- **A connector can be turned off for a while, and on again.** `/connections` has a button for it (`/connections off NAME`, `/connections on NAME`). Turned off, a connector is kept in the list with its sign-in, is not reached and its tools are not offered; turned on, it is back with nothing to sign in to again. In `mcp.json` this is `"disabled": true`, as other MCP clients write it.
- Signing in to a ChatGPT plan from the conversation says, as setup does, that this uses OpenAI's preview programme for open-source apps, which OpenAI may change or end.
- When a site asks for a code, your agent is told to show you its screen with the question.

## 0.4.5

Released on 2026-11-20.

- **Your agent tells you when a connector's sign-in has ended.** A connector that refuses though you signed in to it has its access renewed at once, whatever date it bore, and the action goes on. When it cannot be renewed (the sign-in ran out, or you withdrew it at the service), your agent tells you so, once, with the command to sign in again, instead of its tools quietly failing or being gone. `/connections` shows it as to be signed in to.

## 0.4.4

Released on 2026-11-20. (0.4.3 was never published: its last check before publishing found one more file left behind on AlmaLinux.)

- **Removing Orbite leaves the machine as it was.** The installer writes down what it brings for the agent's computer that the machine did not have (Firefox, Xvfb, `xdotool`, fonts, Mozilla's package source), and `orbite uninstall` takes it away again, with the account's mailbox and the last traces of the service. What the machine already had is never removed, nor a package that something you installed since needs, nor everyday tools the installer had to install (`curl`, `sudo`): those are named. `--keep-computer` keeps what was brought. This is checked on every real machine before a version is published, by comparing the whole machine before installing and after removing.
- **The agent's computer leaves nothing in `/tmp`.** Its browser writes what it needs while running into a folder of the computer's own, cleared when it closes, and the screen takes its socket away when it stops. Before, every stop left a few files and folders there.

## 0.4.2

Released on 2026-11-20.

- **Installing on a machine that was just started no longer fails while the system is updating itself**: on Debian and Ubuntu the installer waits for its turn to install packages.
- The installer needs neither `awk` nor `cmp`, which the smallest openSUSE does not have, and installs what creates accounts where it is missing.
- Every version is now installed on real machines before it is published: the image each system publishes (Debian 12 and 13, Ubuntu 24.04 and 26.04, Fedora, AlmaLinux 9, Arch, openSUSE Tumbleweed), started untouched in a virtual machine, then restarted to check that Orbite comes back by itself.

## 0.4.1

Released on 2026-11-20.

- **The installer runs on any Linux with systemd, freshly installed, with nothing to prepare.** Beyond Debian and Ubuntu: Fedora, AlmaLinux, Arch, openSUSE and the systems made from them, each with its own way of installing packages. It fetches with `curl` or `wget`, whichever is there, and installs itself what it needs and the system lacks. On a system whose packages it does not know, Orbite is installed all the same and you are told how to give the agent its computer.

## 0.4.0

Released on 2026-11-20.

- **At the free permission level nothing is asked any more, critical actions included.** Your agent keeps its own machine: it may update itself, restart its service, restart the machine, change its own settings and read any file its account can read, without asking. At the careful and balanced levels critical actions are asked about each time, as before. Choose `free` knowing that what your agent reads on the web or in a file could then talk it into something you would have refused. Raising the level is still yours alone to allow.
- **Choosing a model by what was tried.** Where you choose a model (setup, `/settings model`), Orbite says which models of the provider you use do all an agent needs, and which is the quickest for light work, from real tryouts: those shipped with Orbite and those run on your own machine.
- Outside contributions are held until their authors have agreed to the contributor agreement, and nothing is merged automatically.
- The installer lets the agent's account do exactly this as root, and nothing else: update Orbite, restart Orbite's service, restart or stop the machine.

## 0.3.15

Released on 2026-11-20.

- **Every connection in one place.** `/connections` shows the AIs your agent thinks with and its connectors, each with how it stands (connected, to sign in to, or what is wrong) and buttons to act: sign in, sign out, remove.
- **A connector is added from the conversation**: `/connect NAME ADDRESS`, with nothing to edit by hand. Orbite starts again and comes back with the link to sign in, or with the connector connected. `orbite connections` does the same on the machine.

## 0.3.14

Released on 2026-11-19.

- When the installer stops on something it did not foresee, or cannot set up the agent's computer, it offers a report: a link to a new issue, filled in with what stopped and the kind of system. Nothing is sent unless you open the link and send it.
- On a machine whose accounts make files for themselves only (umask 077), the agent's computer could not be set up: the key of Mozilla's packages was written unreadable by apt. Found by installing on such a machine.
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
