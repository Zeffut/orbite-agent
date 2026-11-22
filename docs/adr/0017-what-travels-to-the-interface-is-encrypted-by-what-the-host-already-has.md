# What travels to the interface is encrypted by what the Host already has

The web interface ([ADR 0016](0016-orbite-serves-its-own-interface.md)) is reached over HTTPS, with a certificate browsers already trust, and the Owner does nothing to get one. A phone needs it for more than privacy: without it a browser gives a page no microphone, will not install it on the home screen and sends it no notifications.

How the certificate comes depends on where the Host is, in this order:

1. **The Host is on Tailscale.** Orbite listens on the Host alone (`127.0.0.1`), and asks Tailscale to serve that to the Owner's own network over HTTPS, at the name Tailscale gives the Host. Tailscale gets the certificate and renews it. Only the Owner's devices reach it. Orbite adds one served port and leaves alone whatever the Owner already serves; turning the interface off, or uninstalling Orbite, removes it.
2. **The Host has a public address** (a server in a data centre). Orbite gets a certificate for that address itself from Let's Encrypt, which issues them for bare addresses, valid six days, and renews it as it goes. No domain name is needed. *Decided, not built yet.*
3. **Neither.** The interface is served on the Host alone, or on an address of a private network the Owner names, where nothing travels in the open but the browser withholds what it keeps for HTTPS.

## Considered and not chosen

- **A certificate Orbite signs itself.** Every browser warns about it, on every device, and a phone still withholds the microphone. It teaches the Owner to click through warnings.
- **Fetching the certificate from Tailscale and serving HTTPS ourselves.** It takes Tailscale's operator rights at every renewal, which the Agent's account does not have, and duplicates what Tailscale's own serving does once and for good.
- **A relay run by the project.** The Conversation would pass through us; ADR 0016 rules it out for 1.0.
- **Asking for a domain name.** Most Owners have none, and it is no longer needed for a certificate.

## Consequences

- Turning the interface on where Tailscale is takes whoever may configure Tailscale: root, or its operator. It is therefore done when Orbite is installed or from the Host's command line, not by the Agent's account.
- Orbite depends on a behaviour of Tailscale's command line (`serve`, its status in JSON). The few calls are in one place, and when Tailscale refuses or cannot encrypt, the interface stays on the Host and the Owner is told why.
- Other private networks (ZeroTier, NetBird, plain WireGuard) give no certificate: there the interface works without the microphone, installation and notifications, until something in front of it encrypts.
- The public-address way will need Orbite to answer on port 443, which the Agent's account may not open on its own: its service is given that one right and no other.
