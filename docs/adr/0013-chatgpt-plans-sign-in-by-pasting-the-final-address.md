# A ChatGPT plan is signed in to by pasting the final address back

OpenAI documents a way for open-source programs running on their user's machine to use that user's ChatGPT plan (ADR 0006's "official path"; the contract is in `docs/research/chatgpt-sign-in.md`). Its sign-in must end on `http://127.0.0.1`, on the machine whose browser is used. Orbite's Host usually has no browser its Owner sits at, and for that case OpenAI describes one procedure only: sign in on a machine that has one, with the same program, then copy the credential file to the server.

Orbite instead shows the link, lets the Owner open it on any device, and asks them to paste the address their browser ends on (a page that does not load, since nothing listens there). The exchange with OpenAI is exactly the documented one: same addresses, same fields, same loopback redirect, proof of the link by PKCE, `state` and `nonce`. Only who carries the final address differs: the Owner, by hand, rather than a listener on the loopback port.

Considered and set aside: asking the Owner to install Orbite on their laptop to sign in and copy a file over (the documented procedure, and far too much for what is one paste); an SSH tunnel of the loopback port (the same exchange, harder for a newcomer); a relay of the project's own (OpenAI allows loopback addresses only).

Three smaller choices go with it. The identity token is checked for who issued it, whom it is for, its expiry and the link's `nonce`, but its signature is not verified: it comes straight from OpenAI's token endpoint over TLS, which OpenID Connect accepts in place of the signature, and that spares an RSA dependency. The model's private reasoning is not sent back on later turns: Conversations stay readable by any Provider. And Actions are grouped under one namespace, as this path requires.

## Consequences

- OpenAI may decide this is not how its sign-in is to be used, or end the programme, which it calls a preview. The Owner is told so when signing in, and an API Key always works.
- Signing in takes a terminal on the Host (`orbite setup provider`), also when a sign-in ends; doing it from the Conversation can come later.
- If OpenAI comes to require the signature check, a dependency is added then.
