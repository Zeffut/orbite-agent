# Telegram is the only Channel in 1.0

Version 1.0 has no web interface: the Conversation happens in Telegram, and settings are reached through Telegram commands or the `orbite` command on the Host. A Telegram bot needs no inbound port, no domain and no HTTPS, so an installation works behind any home router or VPS with nothing exposed, and the whole login and remote-access problem leaves 1.0. The web interface, with its own design language, account and device approval, is the next major version.

## Consequences

- The Owner cannot take over the Computer's screen in 1.0; the Agent sends screenshots and asks for verification codes in the Conversation.
- Secrets are never requested in the Conversation by default, because they would transit through Telegram; an Owner can opt in.
