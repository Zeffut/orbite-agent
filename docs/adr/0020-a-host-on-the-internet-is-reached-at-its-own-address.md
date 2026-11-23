# A Host on the Internet is reached at its own address, under a certificate given to that address

The web interface is always reached from the Owner's other devices: an agent only its own machine can talk to is of little use. There are two ways, and setup asks which.

- **Through Tailscale**, where the Host is on the Owner's private network (ADR 0017): set up by itself when Tailscale is already on the machine.
- **At the Host's public address**, for a server on the Internet. Orbite listens on ports 443 and 80 itself and serves the interface encrypted under a certificate that Let's Encrypt gives to the address, not to a name: nothing to buy, no record to create, nothing the Owner configures.

## How

Let's Encrypt gives certificates to addresses since January 2026, in its short-lived kind (about six days), proved by answering on port 80 of the address. Orbite asks for one at start, proves the address itself, keeps the certificate and its key in the Agent's folder, and asks again once the certificate is sixty hours old; a failure is written down for the Owner and tried again further and further apart, as the authority only lets so many attempts through. Port 80 answers nothing but that proof and the way to the encrypted side.

The interface itself stays where it was, on the machine alone (127.0.0.1): the door decrypts and passes on, as Tailscale does in the other way. One place serves the interface, whatever is in front of it.

The service is let listen on ports 80 and 443 (one capability, `CAP_NET_BIND_SERVICE`) and nothing more: it still runs as the Agent's own account.

## Considered and not chosen

- **A name under a shared domain** (sslip.io, nip.io) with an ordinary certificate. Every user of those names shares one allowance of certificates a week at Let's Encrypt, which runs out without warning.
- **A certificate signed by the Host itself.** Every device would have to be told to trust it by hand, and an installed web app, the microphone and notifications refuse it.
- **Asking the Owner for a domain name.** It stays possible behind a reverse proxy of theirs (`orbite interface at`), and is not what a first install should ask.
- **A reverse proxy installed beside Orbite** (Caddy). One more program to install, keep up to date and remove, for what a few hundred lines do.

## Consequences

- On the Internet the pairing code is all that stands between a stranger and the Conversation: what limits wrong codes matters there as nowhere else.
- Ports 80 and 443 must be open to the Internet, at the provider's firewall too, and free of any other web server. The installer waits for the certificate and says so when it is not given.
- A Host behind another's address (most home connections, some clouds) does not carry its public address: the Owner gives it, and the ports must be passed on to the machine.
- An install made before this decision has a service that may not listen on those ports: running the installer again writes it anew.
