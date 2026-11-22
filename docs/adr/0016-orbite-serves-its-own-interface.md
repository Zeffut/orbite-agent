# Orbite serves its own interface, and Telegram becomes optional

Version 1.0 has a web interface, served by Orbite itself from the Host: the Conversation, the Approvals and the Agent's screen in a browser, on a computer or a phone. This replaces [ADR 0005](0005-telegram-is-the-only-channel-in-1-0.md), which kept the web interface for the version after. Telegram stays as a Channel, and is no longer required: an Owner may have either, or both on the same Conversation ([ADR 0011](0011-the-conversation-belongs-to-orbite-not-to-a-channel.md)).

The interface is reached where the Host already is: by its public address when Orbite runs on a server in a data centre, or over the Owner's own private network (Tailscale and the like) when it runs at home. Orbite runs no relay and no account service of its own: nothing of the Conversation passes through the project, and there is nothing for the project to operate. A relay, for homes without a private network, remains possible later and is not part of 1.0.

A native mobile app comes after 1.0. Until then the web interface is what a phone uses, and can be installed from the browser.

## Consequences

- Orbite now listens on a port. It does so only once the Owner asks, on the address they choose, and refuses a public address without encryption: what travels is the Owner's Conversation and what lets a device in.
- A device is let in by pairing: a code shown on the Host, used once, exchanged for what the device keeps. There is no password and no account; the Owner sees the devices let in and can turn one away.
- The interface is part of the program: its pages are built into the binary, and nothing more is installed on the Host.
- Setup no longer requires a Telegram bot.
