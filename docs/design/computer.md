# The Agent's Computer: design note

Status: proposal, written before building (#24). ADR 0008 makes the Computer mandatory; this note says what it is made of, how the Agent uses it, and what is still to decide.

## What it is

A desktop session of its own on the Host, running under the `orbite` account, that nobody else looks at:

- **A virtual screen**: an X display served by Xvfb, 1280 by 800, started and kept up by a second service, `orbite-computer.service`. No graphics card, no monitor, nothing listening on the network.
- **A browser**: a real, unmodified desktop browser, with one profile kept in the Agent's folder (`computer/profile`), so that a site signed in to yesterday is still signed in today, and moves with an export.
- **Nothing else at first**: no file manager, no office suite. The Agent already has files and commands without a screen.

The Agent **sees** it by taking pictures of the screen, and **acts** on it with the mouse and the keyboard, as a person would. Nothing is driven from inside the browser: no automation protocol, no injected script. This is the choice the product was specified with ("sees and controls like a person would"), and it has two consequences worth stating:

- A site sees an ordinary browser used with an ordinary mouse and keyboard, because that is what it is. Orbite does not disguise anything, and will not: no fingerprint spoofing, no solving of challenges meant to tell people from programs. Where a site asks for a verification code or a person, the Agent asks its Owner in the Conversation.
- The model has to see. A model that does not take images cannot use the Computer; the tryout already tells an Owner which models do.

## How the Agent uses it

Actions, all on the one screen:

| Action | What it does |
|---|---|
| `computer_look` | takes a picture of the screen and shows it to the model |
| `computer_open` | opens an address in the browser, starting the browser when needed |
| `computer_click` | moves the pointer to a point of the last picture and clicks (left, right, double) |
| `computer_type` | types a text; `{{NAME}}` stands for a secret, filled in by Orbite and never shown to the model |
| `computer_key` | presses a key or a combination: Enter, Tab, Ctrl+L |
| `computer_scroll` | scrolls up or down |

Every Action that changes the screen answers with a fresh picture, so the model always acts on what is there now. Only the last few pictures are kept in what the model is sent: a picture costs about as much as a page of text, and old ones are worth nothing.

Reading the web stays as it is, by fetching pages, which is faster and far cheaper. The Computer is for what fetching cannot do: sites that need a sign-in or a person's gestures.

A picture is sent to the Owner on request (`send_file` already carries it), and the Telegram progress line shows each step as it happens.

## What it asks of the Host

- Packages, installed by the installer: `xvfb`, `xdotool`, a browser, and fonts (`fonts-liberation`, `fonts-noto-color-emoji`).
- About 1 GB of memory on top of Orbite itself with one tab open; the installer already warns below 2 GB.
- The picture is taken by Orbite itself through the X protocol and written as a PNG: no screenshot program is needed. Mouse and keyboard go through `xdotool`.

## Built in this order

1. **The screen without a browser**: start a display, take its picture, move, click, type, read back what happened. Testable anywhere Xvfb is, including CI, with a plain X program as the thing to look at.
2. **The browser**: opening an address, the profile that persists, the service and the installer.
3. **The Agent's side**: the Actions, pictures in the Conversation, secrets typed by name, the Approval rule.
4. **For real**: a task on a site with no API, driven by a model that sees.

## To decide

**Which browser.** Ubuntu ships Chromium and Firefox only as snaps, which do not run well under a service account. Candidates: Firefox from Mozilla's own package repository (Debian and Ubuntu, x86_64 and aarch64), or Google Chrome's package (x86_64 only, not free software). Proposed: Firefox from Mozilla's repository, the only one that covers every machine Orbite installs on, to be confirmed by trying both on fresh machines in CI.

**When the Agent asks.** Asking before every click would make the Computer useless; never asking would let a model buy something. Proposed:

- Looking is free.
- `computer_open` asks the Owner, saying the site and what for: "Use the Computer on example.com to renew the library card". **Always** then covers that site. Clicks and typing on a site that was allowed run without asking.
- Typing a secret asks every time, as a Critical Action: it is how an account is entered.
- The model is told to stop and ask before anything that pays, sends or deletes. Orbite cannot tell such a click from another, so this last point rests on the model, and the tryout should grow a task that measures it.

**Letting the Owner watch.** Not in 1.0 (ADR 0005): the Owner sees pictures in the Conversation. A way to look at or take over the screen belongs with the web interface.
