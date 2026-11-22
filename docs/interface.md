# Talking to your agent in a browser

Orbite serves its own page: your conversation, the approvals your agent asks for and its screen, in a browser on your computer or your phone. Nothing of it passes through anyone else's server. You can have it instead of Telegram, or with it: both show the same conversation.

## Turning it on

Setup asks where you will talk to your agent; choosing the browser turns the page on. Later, tell your agent *"turn the web interface on"*, or run `sudo orbite interface on` on the machine.

Where the page is reached depends on the machine:

- **The machine is on [Tailscale](https://tailscale.com).** The page is served to your own Tailscale network over HTTPS, at the name Tailscale gives the machine, such as `https://my-machine.my-network.ts.net:8377`. Your devices on that network reach it, and nobody else. Tailscale gets the certificate and renews it; turn on *HTTPS Certificates* once in Tailscale's admin console (under DNS) if it is not. Orbite adds one served port and touches nothing else you serve.
- **Otherwise** the page is served on the machine alone, at `http://127.0.0.1:8377`. Put the machine and your phone on Tailscale and turn the interface on again to reach it from elsewhere.

You can also name an address of a private network yourself, `sudo orbite interface on 10.0.0.5:8377`. What travels there is not encrypted by Orbite, and a phone's browser then gives the page no microphone. Orbite refuses to serve the page on a public address in the open.

`sudo orbite interface off`, or *"turn the web interface off"*, closes it, and what Tailscale served for it.

## Letting a device in

There is no password and no account. A device is let in once, with a code:

- Tell your agent *"add my phone"*. It asks you to confirm with a button (at the free permission level it asks nothing, as for everything else), then gives you a link: open it on the device and it is in. The link works once, for ten minutes.
- Or run `sudo orbite pair` on the machine and type the code it shows on the page.

Setup ends with such a link for your first device.

*"Which devices can open our conversation?"* lists them, and *"remove my old phone"* turns one away: its page is told nothing more. On the machine: `sudo orbite devices` and `sudo orbite devices remove NAME`.

## What the page does

- **The conversation**, as it happens, with what your agent is doing while it works. It is the same one as in Telegram: what you write in one shows in the other.
- **Approvals** as cards with buttons. Writing instead of tapping leaves the action out and is heard as a message.
- **Files and pictures**: the paperclip, a paste, or dropping them on the page. What your agent sends you shows in the conversation.
- **Voice messages**: the microphone records, and your agent answers what you said (see [speaking instead of writing](first-day.md#speaking-instead-of-writing)). The browser offers the microphone only over HTTPS or on the machine itself.
- **Your agent's screen**: once it has used its computer, a screen button shows what it sees, following as it goes.
- **On a phone**, add the page to the home screen from the browser's menu: it then opens like an app.

## What is kept on the machine

In the agent's folder: `interface.db` (the devices let in, as what cannot be turned back into what they hold, and what the page showed, so that a device finds the conversation again) and `interface-files` (the files shown). Both go with the agent's data when Orbite is removed.
